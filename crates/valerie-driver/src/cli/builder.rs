//! Workspace build orchestration pipeline.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::cli::pipeline::{auto_detect_target, load_or_extract_asm_with_aliases};
use valerie_driver::BackendMode as DriverBackendMode;
use valerie_driver::terminal::Status;
use valerie_manifest::Manifest;
use valerie_target::TargetDescriptor;

/// Checks if destination artifact exists and is newer than the source file.
pub fn is_up_to_date(src: &Path, dst: &Path) -> bool {
    if let (Ok(m_src), Ok(m_dst)) = (fs::metadata(src), fs::metadata(dst))
        && let (Ok(t_src), Ok(t_dst)) = (m_src.modified(), m_dst.modified())
    {
        return t_dst >= t_src;
    }
    false
}

/// Builds the workspace static library or distribution targets defined in the manifest.
pub fn build_workspace(
    manifest: &Manifest,
    target: &TargetDescriptor,
    has_explicit_target: bool,
    backend_mode: DriverBackendMode,
    vale_bin: &Path,
    output: &Option<PathBuf>,
    start_time: Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    let dist_lib = output
        .clone()
        .or_else(|| {
            manifest
                .distribution
                .as_ref()
                .and_then(|d| d.static_lib.clone())
        })
        .unwrap_or_else(|| PathBuf::from("dist/liboutput.a"));

    if let Some(parent) = dist_lib.parent() {
        fs::create_dir_all(parent)?;
    }

    Status::compiling(format!(
        "Building workspace static library `{}` (package: {})",
        dist_lib.display(),
        manifest.package.name
    ));

    let mut compiled_objs = Vec::new();
    let mut any_recompiled = false;
    let mut emitted_asm_files = std::collections::HashSet::new();
    let temp_dir = std::env::temp_dir().join("valerie_build_workspace");
    let _ = fs::create_dir_all(&temp_dir);

    // Check if assembly is enabled globally and for the target
    let target_triple = target.triple();
    let target_arch = format!("{:?}", target.architecture).to_lowercase();
    let target_asm_override = manifest
        .target
        .get(&target_triple)
        .or_else(|| manifest.target.get(&target_arch))
        .or_else(|| manifest.target.get(&format!("{}-linux", target_arch)))
        .and_then(|t| t.assembly);

    let assembly_enabled = target_asm_override.unwrap_or_else(|| {
        manifest
            .assembly
            .as_ref()
            .map(|a| a.enabled)
            .unwrap_or(true)
    });

    // 1. Lower and assemble Vale sources from manifest.assembly.sources
    if assembly_enabled && let Some(ref assembly_cfg) = manifest.assembly {
        let total_asm_units = assembly_cfg.sources.len();
        for (idx, src) in assembly_cfg.sources.iter().enumerate() {
            let stem = src.file_stem().unwrap_or_default().to_string_lossy();
            let obj_file = temp_dir.join(format!("{}.o", stem));

            if let Some(ref out_dir) = assembly_cfg.output_dir {
                let arch_str = format!("{:?}", target.architecture).to_lowercase();
                emitted_asm_files
                    .insert(out_dir.join(format!("{}_{}.s", stem, arch_str)));
                emitted_asm_files.insert(out_dir.join(format!("{}.s", stem)));
            }

            if is_up_to_date(src, &obj_file) {
                Status::detail(format!(
                    "[{}/{}] Up-to-date Vale unit `{}`",
                    idx + 1,
                    total_asm_units,
                    src.display()
                ));
                compiled_objs.push(obj_file);
                continue;
            }

            any_recompiled = true;
            let src_target =
                auto_detect_target(src, target.clone(), has_explicit_target);
            let asm = load_or_extract_asm_with_aliases(
                src,
                &src_target,
                &temp_dir,
                false,
                vale_bin,
                &assembly_cfg.aliases,
            )?;

            let mut asm_emitted_file = None;
            if let Some(ref out_dir) = assembly_cfg.output_dir {
                fs::create_dir_all(out_dir)?;
                let arch_str =
                    format!("{:?}", src_target.architecture).to_lowercase();
                let target_asm_name = format!("{}_{}.s", stem, arch_str);
                let target_asm_path = out_dir.join(target_asm_name);
                let _ = fs::write(&target_asm_path, &asm);
                asm_emitted_file = Some(target_asm_path);
            }

            valerie_driver::assemble_string_with_backend(
                &asm,
                &obj_file,
                &src_target,
                backend_mode,
            )?;
            Status::assembling(format!(
                "[{}/{}] Compiled Vale unit `{}` -> {}{}",
                idx + 1,
                total_asm_units,
                src.display(),
                obj_file.display(),
                if let Some(ref p) = asm_emitted_file {
                    format!(" (emitted `{}`)", p.display())
                } else {
                    String::new()
                }
            ));
            compiled_objs.push(obj_file);
        }
    }

    // 2. Compile C bundle files in dist/
    let dist_dir_opt = manifest
        .extraction
        .as_ref()
        .and_then(|e| e.c.as_ref())
        .and_then(|c| c.output_dir.clone())
        .or_else(|| Some(PathBuf::from("dist")));

    if let Some(ref dist_dir) = dist_dir_opt {
        let has_c_files = dist_dir.is_dir()
            && fs::read_dir(dist_dir)
                .into_iter()
                .flatten()
                .flatten()
                .any(|e| e.path().extension().is_some_and(|ext| ext == "c"));

        if !has_c_files
            && manifest
                .extraction
                .as_ref()
                .and_then(|e| e.c.as_ref())
                .is_some()
        {
            Status::compiling(
                "No extracted C files found; triggering `valerie extract` prerequisite",
            );
            let current_exe = std::env::current_exe()?;
            let _ = std::process::Command::new(current_exe)
                .arg("extract")
                .status();
        }
    }

    if let Some(dist_dir) = dist_dir_opt
        && dist_dir.is_dir()
    {
        let resolved_env =
            valerie_manifest::resolver::ResolvedEnvironment::resolve();
        let cc = resolved_env
            .clang_exe
            .as_deref()
            .unwrap_or(Path::new("clang"));
        for entry in fs::read_dir(&dist_dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "c") {
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                let obj_path = dist_dir.join(format!("{}.o", stem));

                if is_up_to_date(&path, &obj_path) {
                    Status::detail(format!(
                        "Up-to-date C artifact `{}`",
                        path.display()
                    ));
                    compiled_objs.push(obj_path);
                    continue;
                }

                any_recompiled = true;
                Status::compiling(format!(
                    "Compiling C artifact `{}`",
                    path.display()
                ));
                let mut cc_cmd = std::process::Command::new(cc);
                cc_cmd
                    .arg("-O3")
                    .arg("-c")
                    .arg(&path)
                    .arg("-o")
                    .arg(&obj_path);
                cc_cmd.arg("-I").arg(&dist_dir);
                if let Some(ref assembly_cfg) = manifest.assembly {
                    if let Some(parent) =
                        assembly_cfg.sources.first().and_then(|p| p.parent())
                    {
                        cc_cmd.arg("-I").arg(parent);
                    }
                    if let Some(ref out_dir) = assembly_cfg.output_dir {
                        cc_cmd.arg("-I").arg(out_dir);
                    }
                }
                if let Some(ref inc) = resolved_env.karamel_include {
                    cc_cmd.arg("-I").arg(inc);
                    cc_cmd.arg("-I").arg(inc.join("krml"));
                    cc_cmd.arg("-include").arg("krmllib.h");
                }
                if let Some(ref lib) = resolved_env.karamel_lib {
                    cc_cmd.arg("-I").arg(lib.join("dist/minimal"));
                    cc_cmd.arg("-I").arg(lib.join("dist/generic"));
                    cc_cmd.arg("-I").arg(lib.join("c"));
                    cc_cmd.arg("-I").arg(lib);
                    if lib.join("dist/generic/Prims.h").exists() {
                        cc_cmd.arg("-include").arg("Prims.h");
                    }
                }
                if let Some(ref ulib) = resolved_env.fstar_ulib {
                    cc_cmd.arg("-I").arg(ulib.join("c"));
                }
                if let Some(ref ext_cfg) = manifest.extraction
                    && let Some(ref c_cfg) = ext_cfg.c
                {
                    for flag in &c_cfg.cflags {
                        cc_cmd.arg(flag);
                    }
                }

                let status = cc_cmd.status()?;
                if !status.success() {
                    return Err(format!(
                        "Compilation failed for C artifact `{}`",
                        path.display()
                    )
                    .into());
                }

                compiled_objs.push(obj_path);
            }
        }

        // Compile declarative KaRaMeL runtime modules specified in manifest
        if let Some(ref ext_cfg) = manifest.extraction
            && let Some(ref c_cfg) = ext_cfg.c
            && !c_cfg.runtime.is_empty()
        {
            for rt in &c_cfg.runtime {
                let rt_name = rt.strip_suffix(".c").unwrap_or(rt);
                let mut candidate_src: Option<PathBuf> = None;

                if let Some(ref lib) = resolved_env.karamel_lib {
                    let c_path = lib.join(format!("c/{}.c", rt_name));
                    let gen_path = lib.join(format!("dist/generic/{}.c", rt_name));
                    let min_path = lib.join(format!("dist/minimal/{}.c", rt_name));
                    if c_path.exists() {
                        candidate_src = Some(c_path);
                    } else if gen_path.exists() {
                        candidate_src = Some(gen_path);
                    } else if min_path.exists() {
                        candidate_src = Some(min_path);
                    }
                }

                if candidate_src.is_none()
                    && let Some(ref ulib) = resolved_env.fstar_ulib
                {
                    let ulib_c = ulib.join(format!("c/{}.c", rt_name));
                    if ulib_c.exists() {
                        candidate_src = Some(ulib_c);
                    }
                }

                if let Some(src_path) = candidate_src {
                    let obj_path = temp_dir.join(format!("{}.o", rt_name));
                    let mut rt_cmd = std::process::Command::new(cc);
                    rt_cmd
                        .arg("-O3")
                        .arg("-c")
                        .arg(&src_path)
                        .arg("-o")
                        .arg(&obj_path);
                    rt_cmd.arg("-I").arg(&dist_dir);
                    if let Some(ref lib) = resolved_env.karamel_lib {
                        rt_cmd.arg("-I").arg(lib.join("dist/generic"));
                        rt_cmd.arg("-I").arg(lib.join("dist/minimal"));
                        rt_cmd.arg("-I").arg(lib.join("c"));
                    }
                    if let Some(ref inc) = resolved_env.karamel_include {
                        rt_cmd.arg("-I").arg(inc);
                        rt_cmd.arg("-I").arg(inc.join("krml"));
                    }
                    if let Some(ref ulib) = resolved_env.fstar_ulib {
                        rt_cmd.arg("-I").arg(ulib.join("c"));
                    }
                    for flag in &c_cfg.cflags {
                        rt_cmd.arg(flag);
                    }
                    let status = rt_cmd.status()?;
                    if !status.success() {
                        return Err(format!(
                            "Failed to compile KaRaMeL runtime dependency `{}`",
                            rt
                        )
                        .into());
                    }
                    compiled_objs.push(obj_path);
                } else {
                    return Err(format!(
                        "KaRaMeL runtime source file `{}.c` not found in toolchain library paths",
                        rt_name
                    )
                    .into());
                }
            }
        }
    }

    // 3. Assemble any pre-existing assembly files in manifest.assembly.output_dir
    if assembly_enabled
        && let Some(ref assembly_cfg) = manifest.assembly
        && let Some(ref out_dir) = assembly_cfg.output_dir
        && out_dir.is_dir()
    {
        for entry in fs::read_dir(out_dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "s") {
                if emitted_asm_files.contains(&path) {
                    continue;
                }
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                let obj_path = temp_dir.join(format!("{}.o", stem));

                if is_up_to_date(&path, &obj_path) {
                    Status::detail(format!(
                        "Up-to-date assembly `{}`",
                        path.display()
                    ));
                    compiled_objs.push(obj_path);
                    continue;
                }

                any_recompiled = true;
                let src_target =
                    auto_detect_target(&path, target.clone(), has_explicit_target);
                let asm = fs::read_to_string(&path)?;
                valerie_driver::assemble_string_with_backend(
                    &asm,
                    &obj_path,
                    &src_target,
                    backend_mode,
                )?;
                Status::assembling(format!(
                    "Assembled `{}` -> {}",
                    path.display(),
                    obj_path.display()
                ));
                compiled_objs.push(obj_path);
            }
        }
    }

    // 4. Archive objects into static library
    if !any_recompiled && dist_lib.exists() {
        Status::finished(format!(
            "Workspace static library `{}` is up-to-date ({:.2?})",
            dist_lib.display(),
            start_time.elapsed()
        ));
    } else {
        Status::linking(format!(
            "Archiving {} object(s) into `{}`",
            compiled_objs.len(),
            dist_lib.display()
        ));
        let ar_bin = std::env::var("AR")
            .map(PathBuf::from)
            .ok()
            .or_else(|| which::which("llvm-ar").ok())
            .or_else(|| which::which("ar").ok())
            .unwrap_or_else(|| PathBuf::from("ar"));
        let mut ar_cmd = std::process::Command::new(ar_bin);
        ar_cmd.arg("rcs").arg(&dist_lib);
        for obj in &compiled_objs {
            ar_cmd.arg(obj);
        }
        let status = ar_cmd.status()?;
        if !status.success() {
            return Err(format!(
                "Archive creation failed for `{}`",
                dist_lib.display()
            )
            .into());
        }

        Status::finished(format!(
            "Workspace static library `{}` built successfully in {:.2?}",
            dist_lib.display(),
            start_time.elapsed()
        ));
    }

    Ok(())
}

/// Compiles a single source file (.vaf or .s) into an object file or binary executable.
pub fn build_single_file(
    input_file: &Path,
    output: &Option<PathBuf>,
    strip: bool,
    cli: &crate::cli::args::Cli,
    mut target: TargetDescriptor,
    has_explicit_target: bool,
    backend_mode: DriverBackendMode,
    vale_bin: &Path,
    start_time: Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    target = auto_detect_target(input_file, target, has_explicit_target);
    let temp_dir = std::env::temp_dir().join("valerie_build_tmp");
    let _ = fs::create_dir_all(&temp_dir);

    let out_bin = output.clone().unwrap_or_else(|| {
        let stem = input_file.file_stem().unwrap_or_default().to_string_lossy();
        PathBuf::from(format!("target/{}.o", stem))
    });

    let is_object_output = out_bin.extension().is_some_and(|ext| ext == "o");
    let obj_path = if is_object_output {
        out_bin.clone()
    } else {
        out_bin.with_extension("o")
    };

    if cli.verbose {
        Status::diagnostic(
            "Build Plan",
            format!(
                "Compiling `{}` -> `{}`",
                input_file.display(),
                out_bin.display()
            ),
        );
        Status::detail(format!("Target ABI:           {:?}", target.abi));
        Status::detail(format!("Object-only mode:     {}", is_object_output));
        Status::detail(format!("Requested backend:    {:?}", backend_mode));
    }

    Status::compiling(format!("{} -> {}", input_file.display(), out_bin.display()));

    let with_harness = !is_object_output;
    let asm = match crate::cli::pipeline::load_or_extract_asm(
        input_file,
        &target,
        &temp_dir,
        with_harness,
        vale_bin,
    ) {
        Ok(s) => s,
        Err(e) => {
            return Err(format!("Build error: {}", e).into());
        }
    };

    if let Some(parent) = obj_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let effective_backend = valerie_driver::assemble_string_with_backend(
        &asm,
        &obj_path,
        &target,
        backend_mode,
    )?;

    if cli.verbose {
        Status::detail(format!("Effective assembler:  {:?}", effective_backend));
        if let Ok(meta) = fs::metadata(&obj_path) {
            Status::detail(format!("Object file size:     {} bytes", meta.len()));
        }
    }

    Status::assembling(format!(
        "{} (backend: {:?})",
        obj_path.display(),
        effective_backend
    ));

    if !is_object_output {
        valerie_driver::link_executable(&obj_path, &out_bin, &target)?;
        if strip {
            Status::compiling(format!("Stripping binary {}", out_bin.display()));
            valerie_driver::strip_binary(&out_bin)?;
        }
        Status::linking(format!("{}", out_bin.display()));
    }

    Status::finished(format!("Built target in {:.2?}", start_time.elapsed()));

    Ok(())
}
