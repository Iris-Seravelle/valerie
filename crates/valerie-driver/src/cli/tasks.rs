use crate::cli::handlers::{OcamlBinaryHandler, interpolate_variables};
use std::collections::HashMap;
use valerie_driver::terminal::Status;
use valerie_manifest::resolver::ResolvedEnvironment;
use valerie_manifest::{Manifest, TaskDef};

pub fn execute_manifest_task(
    task_name: &str,
    task: &TaskDef,
    forwarded_args: &[&str],
    all_tasks: &std::collections::HashMap<String, TaskDef>,
    manifest: Option<&Manifest>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut visited = std::collections::HashSet::new();
    let mut call_stack = Vec::new();
    execute_manifest_task_internal(
        task_name,
        task,
        forwarded_args,
        all_tasks,
        manifest,
        &mut visited,
        &mut call_stack,
    )
}

fn execute_manifest_task_internal(
    task_name: &str,
    task: &TaskDef,
    forwarded_args: &[&str],
    all_tasks: &std::collections::HashMap<String, TaskDef>,
    manifest: Option<&Manifest>,
    visited: &mut std::collections::HashSet<String>,
    call_stack: &mut Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if call_stack.contains(&task_name.to_string()) {
        return Err(format!(
            "Cyclic task dependency detected: {} -> {}",
            call_stack.join(" -> "),
            task_name
        )
        .into());
    }

    if visited.contains(task_name) {
        return Ok(());
    }

    call_stack.push(task_name.to_string());

    // 1. Resolve prerequisite tasks (custom tasks or core verbs)
    for dep in &task.deps {
        if let Some(dep_task) = all_tasks.get(dep) {
            execute_manifest_task_internal(
                dep,
                dep_task,
                &[],
                all_tasks,
                manifest,
                visited,
                call_stack,
            )?;
        } else {
            // Built-in core verbs support (e.g. "build", "verify", "extract", "clean")
            let is_core_verb = matches!(
                dep.as_str(),
                "build" | "verify" | "extract" | "clean" | "emit" | "test"
            );
            if is_core_verb {
                if !visited.contains(dep) {
                    Status::running(format!("prerequisite `valerie {}`", dep));
                    let current_exe = std::env::current_exe()?;
                    let status =
                        std::process::Command::new(current_exe).arg(dep).status()?;
                    if !status.success() {
                        return Err(format!(
                            "Prerequisite verb `valerie {}` failed with status: {}",
                            dep, status
                        )
                        .into());
                    }
                    visited.insert(dep.clone());
                }
            } else {
                return Err(format!(
                    "Unrecognized prerequisite task or command `{}` required by `{}`",
                    dep, task_name
                )
                .into());
            }
        }
    }

    // 2. Discover toolchain environment variables & synthesize CFLAGS
    // Use resolve_with_manifest so [toolchain] overrides (e.g. vale = "dist/vale-compiler") are honoured.
    let resolved_env = if let Some(m) = manifest {
        ResolvedEnvironment::resolve_with_manifest(m)
    } else {
        ResolvedEnvironment::resolve()
    };
    let mut env_map = resolved_env.to_env_map();

    if let Ok(curr_exe) = std::env::current_exe() {
        env_map.insert(
            "VALERIE_EXE".into(),
            curr_exe.to_string_lossy().into_owned(),
        );
        if let Some(parent) = curr_exe.parent() {
            let path_env = std::env::var("PATH").unwrap_or_default();
            env_map
                .insert("PATH".into(), format!("{}:{}", parent.display(), path_env));
        }
    }

    // Expose FSTAR_LIB if not already set by resolved_env
    if !env_map.contains_key("FSTAR_LIB")
        && let Some(ref ulib) = resolved_env.fstar_ulib
        && let Some(parent) = ulib.parent()
    {
        let lib_dir = parent.join("lib");
        if lib_dir.join("prims.cmi").exists() {
            env_map
                .insert("FSTAR_LIB".into(), lib_dir.to_string_lossy().into_owned());
        } else {
            env_map
                .insert("FSTAR_LIB".into(), parent.to_string_lossy().into_owned());
        }
    }

    // Synthesize CFLAGS and TARGET from manifest target features and extraction configuration
    let mut cflags_parts = Vec::new();
    let host_target = valerie_target::TargetDescriptor::host();
    let mut target_name = host_target.triple();

    if let Some(m) = manifest {
        if let Some(ext_cfg) = m.extraction.as_ref().and_then(|e| e.c.as_ref()) {
            cflags_parts.extend(ext_cfg.cflags.clone());

            // OCAML_EXTRACTION_DIR: where KaRaMeL writes .ml files; subdirectory "ocaml" within output-dir
            if let Some(ref out_dir) = ext_cfg.output_dir {
                let ocaml_dir = out_dir.join("ocaml");
                env_map.insert(
                    "OCAML_EXTRACTION_DIR".into(),
                    ocaml_dir.to_string_lossy().into_owned(),
                );
            }
        }

        // Check if manifest specifies a target matching host or canonical triple
        let selected_target = m
            .target
            .iter()
            .find(|(k, _)| {
                *k == &target_name
                    || *k
                        == &format!("{:?}", host_target.architecture).to_lowercase()
            })
            .or_else(|| m.target.iter().next());

        if let Some((arch_name, target_cfg)) = selected_target {
            target_name = arch_name.clone();
            for feat in &target_cfg.features {
                match feat.as_str() {
                    "+crypto"
                        if arch_name.contains("aarch64")
                            || arch_name.contains("arm64") =>
                    {
                        cflags_parts.push("-march=armv8-a+crypto".to_string());
                    }
                    "+aes" if arch_name.contains("x86") => {
                        cflags_parts.push("-maes".to_string());
                    }
                    "+pclmul" if arch_name.contains("x86") => {
                        cflags_parts.push("-mpclmul".to_string());
                    }
                    "+zk" if arch_name.contains("riscv") => {
                        cflags_parts.push("-march=rv64gc_zk".to_string());
                    }
                    _ => {}
                }
            }
        }
    }

    env_map.insert("TARGET".into(), target_name);
    if !cflags_parts.is_empty() {
        env_map.insert("CFLAGS".into(), cflags_parts.join(" "));
    }

    // Inject task-level ocaml-modules, ocaml-packages, and env overrides
    if !task.ocaml_modules.is_empty() {
        env_map.insert("OCAML_MODULES".into(), task.ocaml_modules.join(" "));
    }
    if !task.ocaml_packages.is_empty() {
        env_map.insert("OCAML_PACKAGES".into(), task.ocaml_packages.join(","));
    }
    for (k, v) in &task.env {
        // Task-level env overrides win — interpolate values against existing map first
        let expanded = interpolate_variables(v, &env_map);
        env_map.insert(k.clone(), expanded);
    }

    // 3. Run build hook if defined with variable interpolation
    if let Some(ref build_cmd) = task.build {
        let _ = std::fs::create_dir_all("target");
        let interpolated_build = interpolate_variables(build_cmd, &env_map);
        Status::compiling(format!("task build dependency `{}`", interpolated_build));
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string());
        let mut proc = std::process::Command::new(shell);
        proc.arg("-c").arg(&interpolated_build);
        for (k, v) in &env_map {
            proc.env(k, v);
        }
        let status = proc.status()?;
        if !status.success() {
            return Err(
                format!("Task build step failed with status: {}", status).into()
            );
        }
    }

    // 4. Declarative OCaml binary compilation or shell run
    if let Some(ref ocaml_cfg) = task.ocaml_bin {
        Status::compiling(format!(
            "native OCaml target `{}` ({})",
            task_name,
            ocaml_cfg.output.display()
        ));
        execute_ocaml_binary_build(ocaml_cfg, &env_map, manifest)?;
    } else {
        // Assemble execution command line with forwarded flags and variable interpolation
        let mut command_line = interpolate_variables(&task.run, &env_map);
        if !forwarded_args.is_empty() {
            command_line.push(' ');
            command_line.push_str(&forwarded_args.join(" "));
        }

        Status::running(format!("task `{}` ({})", task_name, command_line));

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string());
        let mut proc = std::process::Command::new(shell);
        proc.arg("-c").arg(&command_line);
        for (k, v) in &env_map {
            proc.env(k, v);
        }

        let status = proc.status()?;
        if !status.success() {
            return Err(format!(
                "Task '{}' failed with exit status {}",
                task_name, status
            )
            .into());
        }
    }

    call_stack.pop();
    visited.insert(task_name.to_string());

    Ok(())
}

pub fn execute_ocaml_binary_build(
    cfg: &valerie_manifest::OCamlBuildConfig,
    env_map: &HashMap<String, String>,
    manifest: Option<&Manifest>,
) -> Result<(), Box<dyn std::error::Error>> {
    OcamlBinaryHandler::execute(cfg, env_map, manifest)
}
