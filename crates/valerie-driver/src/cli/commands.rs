use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use valerie_driver::vale_bridge::{ValeCompileOptions, invoke_vale_compiler};

use valerie_driver::terminal::Status;
use valerie_driver::{
    BackendMode as DriverBackendMode, HeaderConfig, QemuConfig, TargetTestRunner,
    audit_procedures, bundle_distribution, compare_emitted_vs_disassembly,
    generate_c_header_with_config, run_benchmark,
};
use valerie_manifest::Manifest;
use valerie_target::{Architecture, TargetDescriptor};

use crate::cli::execute_manifest_task;

use super::args::{Cli, Commands};
use super::pipeline::{
    auto_detect_target, extract_procedures_from_input, load_or_extract_asm,
};
use super::test_runner::test_single_file;

pub fn execute_command(
    cli: &Cli,
    mut target: TargetDescriptor,
    has_explicit_target: bool,
    backend_mode: DriverBackendMode,
    vale_bin: PathBuf,
    start_time: Instant,
    manifest_ctx: Option<&(PathBuf, Manifest)>,
    _resolved_env: Option<&valerie_manifest::resolver::ResolvedEnvironment>,
) -> Result<(), Box<dyn std::error::Error>> {
    match &cli.command {
        Commands::Emit {
            file,
            output,
            fst,
            fsti,
            pulse,
            ir,
        } => {
            crate::cli::emitter::emit_target(
                file,
                output,
                fst,
                fsti,
                pulse,
                ir,
                cli,
                target,
                has_explicit_target,
                &vale_bin,
                manifest_ctx,
            )?;
        }

        Commands::Header { file, output, desc } => {
            let temp_dir = std::env::temp_dir().join("valerie_header_tmp");
            let _ = fs::create_dir_all(&temp_dir);

            if let Some(file_p) = file {
                if cli.verbose {
                    Status::diagnostic(
                        "Header Gen",
                        format!(
                            "Generating C/C++ ABI prototypes from `{}`",
                            file_p.display()
                        ),
                    );
                }

                let procs =
                    extract_procedures_from_input(file_p, &temp_dir, &vale_bin)?;
                let stem = file_p.file_stem().unwrap_or_default().to_string_lossy();
                let config = HeaderConfig {
                    description: desc.clone(),
                    package_prefix: manifest_ctx
                        .as_ref()
                        .map(|(_, m)| m.package.name.clone()),
                };
                let header_code =
                    generate_c_header_with_config(&stem, &procs, &config);

                if cli.verbose {
                    Status::detail(format!(
                        "Prototyped {} Vale procedure(s)",
                        procs.len()
                    ));
                    Status::detail(format!(
                        "Include guard: {}",
                        if let Some(ref pkg) = config.package_prefix {
                            format!(
                                "{}_{}_H",
                                pkg.to_uppercase().replace('-', "_"),
                                stem.to_uppercase().replace('-', "_")
                            )
                        } else {
                            format!("{}_H", stem.to_uppercase().replace('-', "_"))
                        }
                    ));
                }

                if let Some(out_p) = output {
                    fs::write(out_p, &header_code)?;
                    Status::finished(format!(
                        "Generated C header: {}",
                        out_p.display()
                    ));
                } else {
                    println!("{}", header_code);
                }
            } else if let Some((_, manifest)) = manifest_ctx {
                if let Some(ref assembly_cfg) = manifest.assembly {
                    let out_dir = output
                        .clone()
                        .or_else(|| assembly_cfg.output_dir.clone())
                        .unwrap_or_else(|| PathBuf::from("src/asm"));
                    fs::create_dir_all(&out_dir)?;

                    for src in &assembly_cfg.sources {
                        let stem =
                            src.file_stem().unwrap_or_default().to_string_lossy();
                        let procs = extract_procedures_from_input(
                            src, &temp_dir, &vale_bin,
                        )?;
                        let config = HeaderConfig {
                            description: desc.clone(),
                            package_prefix: Some(manifest.package.name.clone()),
                        };
                        let header_code =
                            generate_c_header_with_config(&stem, &procs, &config);
                        let header_path = out_dir.join(format!("{}.h", stem));
                        fs::write(&header_path, &header_code)?;
                        Status::finished(format!(
                            "Generated workspace C header `{}` ({} procedure(s))",
                            header_path.display(),
                            procs.len()
                        ));
                    }
                } else {
                    return Err(
                        "manifest does not declare [assembly.sources]".into()
                    );
                }
            } else {
                return Err("`header` requires an input file when outside a valerie workspace.".into());
            }
        }

        Commands::Audit { file, require_dit } => {
            let temp_dir = std::env::temp_dir().join("valerie_audit_tmp");
            let _ = fs::create_dir_all(&temp_dir);

            Status::testing(format!(
                "Auditing constant-time properties of {}",
                file.display()
            ));
            let procs = extract_procedures_from_input(file, &temp_dir, &vale_bin)?;
            let report = audit_procedures(&procs);

            let dit_demanded = *require_dit || cli.dit;
            let arch_supports_dit = target.architecture == Architecture::AArch64;

            if cli.verbose {
                Status::diagnostic(
                    "Audit Analysis",
                    format!(
                        "Scanned {} machine instructions in {} procedure(s) (ARM DIT: {:?}, Arch support: {})",
                        report.total_instructions,
                        procs.len(),
                        report.dit_status,
                        if arch_supports_dit {
                            "Supported (AArch64)"
                        } else {
                            "Not applicable (non-ARMv8.4+)"
                        }
                    ),
                );
            }

            if dit_demanded {
                if !arch_supports_dit {
                    return Err(format!(
                        "Data Independent Timing (DIT) was demanded via flag, but target architecture {:?} does not support architectural DIT (requires AArch64 / ARMv8.4-A+).",
                        target.architecture
                    ).into());
                } else if report.dit_status
                    != valerie_driver::audit::DitStatus::Enabled
                {
                    return Err(
                        "Data Independent Timing (DIT) enforcement failed: procedure does not configure `MSR DIT, #1`.".into()
                    );
                }
            }

            if report.is_clean() {
                Status::finished(format!(
                    "Constant-time clean: 0 vulnerabilities found across {} instructions (ARM DIT {:?}).",
                    report.total_instructions, report.dit_status,
                ));
            } else {
                eprintln!("\nAudit Warnings/Errors found:");
                for finding in &report.findings {
                    eprintln!(
                        "  [{:?}] {}: instruction #{}: {:?} - {}",
                        finding.severity,
                        finding.procedure,
                        finding.instruction_index,
                        finding.opcode,
                        finding.reason
                    );
                }
                return Err(
                    "Constant-time audit failed with security findings.".into()
                );
            }
        }

        Commands::Diff {
            file,
            show_bytes,
            against,
            opt_level,
            json,
        } => {
            target = auto_detect_target(file, target, has_explicit_target);
            if cli.verbose {
                Status::diagnostic(
                    "Diff Engine",
                    format!("Disassembly parity comparison on `{}`", file.display()),
                );
                Status::detail(format!(
                    "Target architecture: {:?}",
                    target.architecture
                ));
                Status::detail(format!("Assembler backend:   {:?}", backend_mode));
                Status::detail(
                    "Parity verifier:    llvm-objdump (zero-mutation check)",
                );
                if *show_bytes {
                    Status::detail("Machine byte dumps: Enabled");
                }
                if let Some(c) = against {
                    Status::detail(format!(
                        "Differential mode:  Comparing against {} (-O{})",
                        c, opt_level
                    ));
                }
            }
            let temp_dir = std::env::temp_dir().join("valerie_diff_tmp");
            let _ = fs::create_dir_all(&temp_dir);

            if !*json {
                Status::compiling(format!(
                    "Analyzing disassembly parity on {}",
                    file.display()
                ));
            }
            let asm =
                load_or_extract_asm(file, &target, &temp_dir, false, &vale_bin)?;
            let procs =
                valerie_driver::vale_bridge::extract_procedures_from_asm(&asm);

            let obj_path = temp_dir.join("diff_target.o");
            valerie_driver::assemble_string_with_backend(
                &asm,
                &obj_path,
                &target,
                backend_mode,
            )?;

            let mut diff_entries =
                compare_emitted_vs_disassembly(&procs, &obj_path, &target)?;

            if let Some(comp) = against {
                let comp_result = valerie_driver::diff::compare_against_compiler(
                    &asm, comp, opt_level, &target, &temp_dir,
                )?;
                if let Some(first_entry) = diff_entries.first_mut() {
                    first_entry.compiler_diff = Some(comp_result);
                }
            }

            if *json {
                // Structured JSON report
                print!("{{\"file\":\"{}\",\"parity\":[", file.display());
                for (i, entry) in diff_entries.iter().enumerate() {
                    if i > 0 {
                        print!(",");
                    }
                    print!(
                        "{{\"procedure\":\"{}\",\"emitted_count\":{},\"disassembled_count\":{},\"valid\":{},\"matches\":[",
                        entry.proc_name,
                        entry.emitted_count,
                        entry.disassembled_count,
                        entry.is_parity_valid
                    );
                    for (j, m) in entry.matches.iter().enumerate() {
                        if j > 0 {
                            print!(",");
                        }
                        print!(
                            "{{\"emitted\":\"{}\",\"disassembled\":\"{}\",\"bytes\":\"{}\",\"is_exact\":{}}}",
                            m.emitted.replace('"', "\\\""),
                            m.disassembled.replace('"', "\\\""),
                            m.raw_bytes,
                            m.is_exact
                        );
                    }
                    print!("]}}");
                }
                println!("]}}");
                return Ok(());
            }

            println!("\nDisassembly Parity Report:");
            let mut all_valid = true;
            for entry in &diff_entries {
                println!("  Procedure <{}>:", entry.proc_name);
                println!("    Emitted Body Instructions: {}", entry.emitted_count);
                println!(
                    "    Total Machine Instructions: {}",
                    entry.disassembled_count
                );
                if !entry.generated_epilogue.is_empty() {
                    println!(
                        "    Generated Epilogue:         [{}]",
                        entry.generated_epilogue.join(", ")
                    );
                }
                for (idx, m) in entry.matches.iter().enumerate() {
                    let note_str = if let Some(ref note) = m.alias_note {
                        format!(" (alias: {})", note)
                    } else {
                        String::new()
                    };
                    let bytes_col = if *show_bytes {
                        format!(" [{:<12}]", m.raw_bytes)
                    } else {
                        String::new()
                    };
                    println!(
                        "      [{:02}]{} {:<24} <==> {}{}",
                        idx, bytes_col, m.emitted, m.disassembled, note_str
                    );
                }

                if let Some(ref comp) = entry.compiler_diff {
                    println!(
                        "\n    Differential Comparison against `{}`:",
                        comp.compiler_name
                    );
                    println!(
                        "      Valerie Instruction Count:  {}",
                        comp.valerie_instruction_count
                    );
                    println!(
                        "      {} Instruction Count:  {}",
                        comp.compiler_name, comp.compiler_instruction_count
                    );
                    if comp.valerie_instruction_count
                        == comp.compiler_instruction_count
                    {
                        println!(
                            "      Parity Result:              Zero-inflation parity matched."
                        );
                    } else {
                        println!(
                            "      Parity Result:              Discrepancy (Valerie: {}, {}: {})",
                            comp.valerie_instruction_count,
                            comp.compiler_name,
                            comp.compiler_instruction_count
                        );
                        println!("      Discrepancy Breakdown:");
                        let max_len = std::cmp::max(
                            entry.matches.len(),
                            comp.compiler_instructions.len(),
                        );
                        for idx in 0..max_len {
                            let val_str = entry
                                .matches
                                .get(idx)
                                .map(|m| format!("{} {}", m.emitted, m.raw_bytes))
                                .unwrap_or_else(|| "<none>".to_string());
                            let comp_str = comp
                                .compiler_instructions
                                .get(idx)
                                .map(|s| s.as_str())
                                .unwrap_or("<none>");
                            if idx < entry.matches.len()
                                && idx < comp.compiler_instructions.len()
                                && (val_str.contains(comp_str)
                                    || comp_str
                                        .contains(&entry.matches[idx].emitted))
                            {
                                continue;
                            }
                            println!(
                                "        [{:02}] Valerie: {:<32} | {}: {}",
                                idx, val_str, comp.compiler_name, comp_str
                            );
                        }
                    }
                }

                if !entry.is_parity_valid {
                    all_valid = false;
                }
            }
            println!();
            if all_valid {
                Status::finished(
                    "Disassembly verified: 100% 1:1 machine code parity confirmed (zero compiler mutations).",
                );
            } else {
                return Err("Disassembly mismatch detected! Machine code does not match emitted AST.".into());
            }
        }

        Commands::Bench { file, iters } => {
            target = auto_detect_target(file, target, has_explicit_target);
            let bench_iters = *iters;
            if cli.verbose {
                Status::diagnostic(
                    "Benchmark",
                    format!(
                        "Benchmarking `{}` across {} iterations",
                        file.display(),
                        bench_iters
                    ),
                );
                Status::detail(
                    "Timer source:       Hardware performance counter CNTVCT_EL0",
                );
                Status::detail(format!("Target triple:      {:?}", target.abi));
            }
            let temp_dir = std::env::temp_dir().join("valerie_bench_tmp");
            let _ = fs::create_dir_all(&temp_dir);

            Status::testing(format!(
                "Benchmarking {} ({} iterations)",
                file.display(),
                bench_iters
            ));
            let procs = extract_procedures_from_input(file, &temp_dir, &vale_bin)?;
            let bench_res =
                run_benchmark(&procs, bench_iters, &target, backend_mode)?;

            Status::finished(format!(
                "<{}> {:.2} cycles/op (median across {} iterations, {} warmup runs, ~{} cycles timer overhead)",
                bench_res.proc_name,
                bench_res.median_cycles,
                bench_res.iterations,
                bench_res.warmup_iterations,
                bench_res.timer_overhead_cycles
            ));
        }

        Commands::Bundle { file, output } => {
            target = auto_detect_target(file, target, has_explicit_target);
            if cli.verbose {
                Status::diagnostic(
                    "Distribution",
                    format!(
                        "Packaging production bundle into `{}`",
                        output.display()
                    ),
                );
                Status::detail(format!(
                    "Source specification: `{}`",
                    file.display()
                ));
                Status::detail(format!("Assembler backend:    {:?}", backend_mode));
                Status::detail(
                    "Static library tool:  llvm-ar / ar (standard POSIX)",
                );
            }
            let temp_dir = std::env::temp_dir().join("valerie_bundle_tmp");
            let _ = fs::create_dir_all(&temp_dir);

            Status::compiling(format!(
                "Bundling {} -> {}",
                file.display(),
                output.display()
            ));
            let procs = extract_procedures_from_input(file, &temp_dir, &vale_bin)?;
            let stem = file.file_stem().unwrap_or_default().to_string_lossy();

            let artifacts =
                bundle_distribution(&stem, &procs, output, &target, backend_mode)?;
            println!("  Emitted Assembly: {}", artifacts.asm_path.display());
            println!("  Object Binary:    {}", artifacts.obj_path.display());
            println!("  C Header:         {}", artifacts.header_path.display());
            println!("  Static Library:   {}", artifacts.archive_path.display());
            Status::finished(format!(
                "Packaged production distribution in {}",
                output.display()
            ));
        }

        Commands::Verify {
            file,
            pulse,
            spec,
            force,
        } => {
            crate::cli::handlers::FStarHandler::verify(
                file.as_deref(),
                *pulse,
                *spec,
                cli.jobs,
                *force,
                manifest_ctx,
                &vale_bin,
                cli.verbose,
                start_time,
            )?;
        }

        Commands::Check { file } => {
            let temp_dir = std::env::temp_dir().join("valerie_check_tmp");
            let _ = fs::create_dir_all(&temp_dir);

            if let Some(input_file) = file {
                let _target =
                    auto_detect_target(input_file, target, has_explicit_target);
                Status::checking(format!("{}", input_file.display()));

                let procs =
                    extract_procedures_from_input(input_file, &temp_dir, &vale_bin)?;
                Status::finished(format!(
                    "`check` passed: {} verified procedure(s) syntax & AST valid in {:.2?}",
                    procs.len(),
                    start_time.elapsed()
                ));
            } else if let Some((_, manifest)) = manifest_ctx {
                let mut checked_count = 0;
                if let Some(ref assembly_cfg) = manifest.assembly {
                    for src in &assembly_cfg.sources {
                        Status::checking(format!("{}", src.display()));
                        let _procs = extract_procedures_from_input(
                            src, &temp_dir, &vale_bin,
                        )?;
                        checked_count += 1;
                    }
                }
                Status::finished(format!(
                    "`check` passed for {} workspace file(s) in {:.2?}",
                    checked_count,
                    start_time.elapsed()
                ));
            } else {
                return Err("`check` requires an input file or an active workspace manifest (valerie.toml).".into());
            }
        }

        Commands::Build {
            file,
            output,
            lib: _,
            strip,
        } => {
            if let Some(input_file) = file {
                crate::cli::builder::build_single_file(
                    input_file,
                    output,
                    *strip,
                    cli,
                    target,
                    has_explicit_target,
                    backend_mode,
                    &vale_bin,
                    start_time,
                )?;
            } else if let Some((_, manifest)) = manifest_ctx {
                crate::cli::builder::build_workspace(
                    manifest,
                    &target,
                    has_explicit_target,
                    backend_mode,
                    &vale_bin,
                    output,
                    start_time,
                )?;
            } else {
                return Err("`build` requires an input file or an active workspace manifest (valerie.toml).".into());
            }
        }

        Commands::Extract => {
            if let Some((_, manifest)) = manifest_ctx {
                crate::cli::extractor::extract_workspace(manifest, start_time)?;
            } else {
                return Err("`extract` requires an active workspace manifest (valerie.toml).".into());
            }
        }

        Commands::Init { path, name } => {
            let target_dir = path.clone().unwrap_or_else(|| PathBuf::from("."));
            let pkg_name = name.clone().unwrap_or_else(|| {
                target_dir
                    .canonicalize()
                    .ok()
                    .and_then(|p| {
                        p.file_name().map(|n| n.to_string_lossy().to_string())
                    })
                    .unwrap_or_else(|| "my_crypto".to_string())
            });

            fs::create_dir_all(target_dir.join("src/spec"))?;
            fs::create_dir_all(target_dir.join("src/proof"))?;
            fs::create_dir_all(target_dir.join("src/pulse"))?;
            fs::create_dir_all(target_dir.join("src/vale"))?;
            fs::create_dir_all(target_dir.join("src/vale/generated"))?;
            fs::create_dir_all(target_dir.join("src/asm"))?;
            fs::create_dir_all(target_dir.join("dist"))?;

            let manifest_content = format!(
                r#"[package]
name = "{pkg_name}"
version = "0.1.0"
authors = ["Author <author@example.com>"]
edition = "2026"
description = "Verified cryptographic primitives"

[toolchain]
fstar = "auto"
vale = "auto"
karamel = "auto"
clang = "auto"

[target.aarch64-linux]
features = ["+crypto"]
dit = true
assembler = "native"

[target.x86_64-linux]
features = ["+aes", "+pclmul"]
assembler = "auto"

[target.riscv64-linux]
features = ["+zk"]
assembler = "auto"

[verification]
cache-dir = ".cache"
hints-dir = "hints"
warn-error = "-271-272-241"
already-cached = ["Prims", "FStar", "LowStar", "PulseCore", "Pulse"]
include = ["src/spec", "src/proof", "src/pulse", "src/vale/generated"]

[verification.pulse]
enabled = true

[assembly]
sources = []
emit-ast-dir = "src/vale/generated"
output-dir = "src/asm"
abi = "aapcs64"

[extraction.c]
engine = "karamel"
output-dir = "dist"
builtin-uint128 = true
minimal-headers = true
warn-error = "-2-4-9-15"
drop = ["Prims", "FStar.UInt128", "LowStar.*", "PulseCore.*"]
libraries = ["FStar.UInt128"]

[distribution]
static-lib = "dist/lib{pkg_name}.a"
headers = []
"#
            );

            let manifest_path = target_dir.join("valerie.toml");
            if manifest_path.exists() {
                return Err(format!(
                    "`{}` already exists.",
                    manifest_path.display()
                )
                .into());
            }

            fs::write(&manifest_path, manifest_content)?;
            Status::finished(format!(
                "Initialized new verified project `{}` in {}",
                pkg_name,
                target_dir.display()
            ));
        }

        Commands::Clean => {
            if let Some((_, manifest)) = manifest_ctx {
                Status::finished(format!(
                    "Purging workspace artifacts for `{}`...",
                    manifest.package.name
                ));
                if let Some(ref ver_cfg) = manifest.verification {
                    if let Some(ref c) = ver_cfg.cache_dir {
                        let _ = fs::remove_dir_all(c);
                    }
                    if let Some(ref h) = ver_cfg.hints_dir {
                        let _ = fs::remove_dir_all(h);
                    }
                }
                let _ = fs::remove_dir_all("target");
                if let Some(ref dist_cfg) = manifest.distribution
                    && let Some(ref lib) = dist_cfg.static_lib
                    && lib.exists()
                {
                    let _ = fs::remove_file(lib);
                }
                if let Some(ref ext_cfg) = manifest.extraction
                    && let Some(ref c_cfg) = ext_cfg.c
                    && let Some(ref out_dir) = c_cfg.output_dir
                    && out_dir.is_dir()
                {
                    for entry in
                        fs::read_dir(out_dir).into_iter().flatten().flatten()
                    {
                        let path = entry.path();
                        if path.extension().is_some_and(|e| e == "o" || e == "a") {
                            let _ = fs::remove_file(path);
                        }
                    }
                }
                Status::finished(
                    "Workspace build artifacts and verification caches cleared.",
                );
            } else {
                let _ = fs::remove_dir_all("target");
                Status::finished("Cleaned `target/` directory.");
            }
        }

        Commands::Test {
            filter,
            nocapture,
            qemu,
            cpu,
            e2e,
        } => {
            let filter_str = filter.as_deref().unwrap_or("");
            let maybe_path = PathBuf::from(filter_str);

            if !filter_str.is_empty() && maybe_path.is_file() {
                let fixture_target =
                    auto_detect_target(&maybe_path, target, has_explicit_target);
                if cli.verbose {
                    Status::diagnostic(
                        "Test Plan",
                        format!(
                            "Executing single-fixture test on `{}`",
                            maybe_path.display()
                        ),
                    );
                }
                test_single_file(
                    &maybe_path,
                    &fixture_target,
                    backend_mode,
                    *qemu,
                    cpu.as_deref(),
                    cli.skip_verify,
                    cli.verbose,
                    &vale_bin,
                )?;
            } else {
                let test_dirs = [
                    PathBuf::from("tests/suite"),
                    PathBuf::from("tests/fixtures/vale"),
                    PathBuf::from("../tests/fixtures/vale"),
                ];

                let mut all_files = Vec::new();
                for dir in &test_dirs {
                    if dir.is_dir()
                        && let Ok(entries) = fs::read_dir(dir)
                    {
                        for entry in entries.flatten() {
                            let p = entry.path();
                            if p.extension().is_some_and(|e| e == "vaf")
                                && !all_files.contains(&p)
                            {
                                if filter_str.is_empty()
                                    || p.to_string_lossy().contains(filter_str)
                                {
                                    all_files.push(p);
                                }
                            }
                        }
                    }
                }
                all_files.sort();

                if !all_files.is_empty() {
                    Status::test_suite_header(all_files.len());

                    let out_dir = PathBuf::from("dist/test_out");
                    let _ = fs::create_dir_all(&out_dir);

                    let jobs_num = if cli.jobs > 0 { cli.jobs } else { 1 };
                    let mut passed = 0;
                    let mut failed = 0;

                    let chunks: Vec<Vec<PathBuf>> =
                        all_files.chunks(jobs_num).map(|c| c.to_vec()).collect();

                    for chunk in chunks {
                        let mut handles = Vec::new();
                        for path in chunk {
                            let base = path
                                .file_stem()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            let fst = out_dir.join(format!("{}.fst", base));
                            let fsti = out_dir.join(format!("{}.fsti", base));
                            let pulse_out =
                                out_dir.join(format!("{}_pulse.fst", base));
                            let vale_bin_clone = vale_bin.clone();
                            let is_e2e = *e2e;
                            let manifest_ctx_clone = manifest_ctx.cloned();

                            handles.push(std::thread::spawn(move || -> (String, bool, std::time::Duration, String, Option<Vec<u8>>) {
                                let t_start = Instant::now();
                                let mut cmd = std::process::Command::new(&vale_bin_clone);
                                cmd.arg("-in")
                                    .arg(&path)
                                    .arg("-out")
                                    .arg(&fst)
                                    .arg("-outi")
                                    .arg(&fsti);

                                let res = cmd.output();
                                let dur = t_start.elapsed();

                                match res {
                                    Ok(output) if output.status.success() => {
                                        if is_e2e {
                                            let mut p_cmd = std::process::Command::new(&vale_bin_clone);
                                            p_cmd.arg("-in").arg(&path).arg("-pulse").arg(&pulse_out);
                                            match p_cmd.output() {
                                                Ok(p_out) if p_out.status.success() => {
                                                    let ver_res = crate::cli::handlers::FStarHandler::verify(
                                                        Some(&pulse_out),
                                                        true,
                                                        false,
                                                        1,
                                                        false,
                                                        manifest_ctx_clone.as_ref(),
                                                        &vale_bin_clone,
                                                        false,
                                                        Instant::now(),
                                                    );
                                                    if let Err(err) = ver_res {
                                                        return (base, false, dur, format!("Pulse verification failed: {}", err), None);
                                                    }
                                                }
                                                Ok(p_out) => {
                                                    return (base, false, dur, String::from_utf8_lossy(&p_out.stderr).to_string(), None);
                                                }
                                                Err(e) => {
                                                    return (base, false, dur, format!("Failed to run pulse emit: {}", e), None);
                                                }
                                            }
                                        }
                                        (base, true, dur, String::new(), Some(output.stdout))
                                    }
                                    Ok(output) => {
                                        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                                        (base, false, dur, stderr, None)
                                    }
                                    Err(e) => {
                                        (base, false, dur, format!("exec error: {}", e), None)
                                    }
                                }
                            }));
                        }

                        for handle in handles {
                            if let Ok((base, success, dur, err_msg, stdout_opt)) =
                                handle.join()
                            {
                                if success {
                                    passed += 1;
                                    Status::test_case(&base, true, Some(dur));
                                    if *nocapture
                                        && let Some(stdout) = stdout_opt
                                        && !stdout.is_empty()
                                    {
                                        println!(
                                            "{}",
                                            String::from_utf8_lossy(&stdout)
                                        );
                                    }
                                } else {
                                    failed += 1;
                                    Status::test_case(&base, false, Some(dur));
                                    eprintln!("  {}\n", err_msg);
                                }
                            }
                        }
                    }

                    Status::test_suite_result(passed, failed, start_time.elapsed());
                    if failed > 0 {
                        return Err(format!("{} test(s) failed", failed).into());
                    }
                } else {
                    if !filter_str.is_empty() {
                        return Err(format!(
                            "No tests matching filter `{}` found in tests/suite or tests/fixtures/vale",
                            filter_str
                        ).into());
                    }
                    return Err("No .vaf test files discovered in tests/suite or tests/fixtures/vale".into());
                }
            }
        }

        Commands::Run { file, qemu, cpu } => {
            target = auto_detect_target(file, target, has_explicit_target);
            let temp_dir = std::env::temp_dir().join("valerie_run_tmp");
            let _ = fs::create_dir_all(&temp_dir);

            let asm =
                match load_or_extract_asm(file, &target, &temp_dir, true, &vale_bin)
                {
                    Ok(s) => s,
                    Err(e) => {
                        return Err(format!("Execution error: {}", e).into());
                    }
                };

            let mut qemu_cfg = QemuConfig::new();
            if let Some(cpu_model) = cpu {
                qemu_cfg = qemu_cfg.with_cpu(cpu_model);
            }

            let stem = file.file_stem().unwrap_or_default().to_string_lossy();
            let runner =
                TargetTestRunner::new(format!("cli_run_{}", stem), target.clone())
                    .backend_mode(backend_mode)
                    .from_asm(asm)
                    .qemu_config(qemu_cfg);

            let engine_desc = if *qemu
                || target.architecture == Architecture::Arm32
                || target.architecture == Architecture::X86_64
                || target.architecture == Architecture::Riscv64
            {
                "qemu"
            } else {
                "native"
            };

            if cli.verbose {
                Status::diagnostic(
                    "Execution",
                    format!(
                        "Running `{}` via {} engine",
                        file.display(),
                        engine_desc
                    ),
                );
                Status::detail(format!("QEMU requested:       {}", *qemu));
                Status::detail(format!("CPU model:            {:?}", cpu));
                Status::detail(format!("Assembler backend:    {:?}", backend_mode));
            }

            Status::running(format!("`{}` ({})", file.display(), engine_desc));
            let result = if *qemu
                || target.architecture == Architecture::Arm32
                || target.architecture == Architecture::X86_64
                || target.architecture == Architecture::Riscv64
            {
                runner.run_qemu()?
            } else {
                runner.run_native()?
            };

            if !result.stdout.is_empty() {
                print!("{}", result.stdout);
            }
            if !result.stderr.is_empty() {
                eprint!("{}", result.stderr);
            }

            Status::finished(format!(
                "Process finished with exit code {} in {:.2?}",
                result.exit_code,
                start_time.elapsed()
            ));
        }

        Commands::Vale {
            input,
            out,
            outi,
            pulse,
            ir,
            arch,
            file,
            install,
        } => {
            if *install {
                Status::compiling("Building Vale compiler (valerie compiler)...");
                let (manifest_dir, manifest_obj) = if let Some((p, m)) = manifest_ctx
                {
                    (
                        p.parent().unwrap_or(Path::new(".")).to_path_buf(),
                        m.clone(),
                    )
                } else {
                    let sub_cand = PathBuf::from("valerie-core/valerie.toml");
                    if let Ok(m) = Manifest::from_file(&sub_cand) {
                        (PathBuf::from("valerie-core"), m)
                    } else {
                        return Err(
                            "No valerie.toml manifest found to build Vale compiler."
                                .into(),
                        );
                    }
                };

                let task_def = manifest_obj
                    .tasks
                    .get("compiler")
                    .cloned()
                    .ok_or("No `compiler` task found in valerie.toml")?;

                let orig_dir = std::env::current_dir()?;
                if manifest_dir != Path::new(".") && manifest_dir.is_dir() {
                    let _ = std::env::set_current_dir(&manifest_dir);
                }

                let build_res = execute_manifest_task(
                    "compiler",
                    &task_def,
                    &[],
                    &manifest_obj.tasks,
                    Some(&manifest_obj),
                );

                let _ = std::env::set_current_dir(&orig_dir);
                build_res?;

                // Locate built binary in dist/valerie-compiler or dist/vale-compiler
                let dist_candidates = [
                    manifest_dir.join("dist/valerie-compiler"),
                    manifest_dir.join("dist/vale-compiler"),
                    PathBuf::from("dist/valerie-compiler"),
                    PathBuf::from("dist/vale-compiler"),
                    PathBuf::from("valerie-core/dist/valerie-compiler"),
                    PathBuf::from("valerie-core/dist/vale-compiler"),
                ];
                let built_bin =
                    dist_candidates.into_iter().find(|p| p.is_file()).ok_or(
                        "Could not find compiled binary at dist/valerie-compiler",
                    )?;

                let home =
                    std::env::var("HOME")
                        .map(PathBuf::from)
                        .unwrap_or_else(|_| {
                            PathBuf::from("/data/data/com.termux/files/home")
                        });
                let local_bin = home.join(".local/bin");
                fs::create_dir_all(&local_bin)?;

                let dest_compiler = local_bin.join("valerie-compiler");
                let dest_valy = local_bin.join("valy");
                let dest_vale = local_bin.join("vale");
                let dest_vale_exe = local_bin.join("vale.exe");

                // Atomic copy to prevent ETXTBSY ("Text file busy") if binaries are cached/in use
                let atomic_install =
                    |src: &Path,
                     dst: &Path|
                     -> Result<(), Box<dyn std::error::Error>> {
                        let tmp = dst.with_extension("tmp");
                        fs::copy(src, &tmp)?;
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = fs::set_permissions(
                                &tmp,
                                fs::Permissions::from_mode(0o755),
                            );
                        }
                        fs::rename(&tmp, dst)?;
                        Ok(())
                    };

                atomic_install(&built_bin, &dest_compiler)?;

                // Use symlinks for aliases so we never duplicate disk space
                #[cfg(unix)]
                {
                    use std::os::unix::fs::symlink;
                    let _ = fs::remove_file(&dest_valy);
                    let _ = symlink("valerie-compiler", &dest_valy);
                    let _ = fs::remove_file(&dest_vale);
                    let _ = symlink("valerie-compiler", &dest_vale);
                    let _ = fs::remove_file(&dest_vale_exe);
                    let _ = symlink("valerie-compiler", &dest_vale_exe);
                }
                #[cfg(not(unix))]
                {
                    let _ = fs::copy(&dest_compiler, &dest_valy);
                    let _ = fs::copy(&dest_compiler, &dest_vale);
                    let _ = fs::copy(&dest_compiler, &dest_vale_exe);
                }

                Status::finished(format!(
                    "Installed `{}` to `{}` (and aliases `{}`, `{}`) in {:.2?}",
                    built_bin.display(),
                    dest_compiler.display(),
                    dest_valy.display(),
                    dest_vale.display(),
                    start_time.elapsed()
                ));
                return Ok(());
            }

            let src = input.as_deref().or(file.as_deref()).ok_or(
                "No input .vaf file specified. Usage: valerie vale -in <source.vaf> [options] or valerie vale --install"
            )?;

            let arch_name = arch.as_deref().unwrap_or(match target.architecture {
                Architecture::AArch64 => "aarch64",
                Architecture::Arm32 => "arm32",
                Architecture::X86_64 => "x64",
                Architecture::Riscv64 => "riscv64",
            });

            let mut opts = ValeCompileOptions::new(src);
            opts.output_fst = out.as_deref();
            opts.output_fsti = outi.as_deref();
            opts.output_pulse = pulse.as_deref();
            opts.output_ir = ir.as_deref();
            opts.arch = Some(arch_name);

            // Ensure output directories exist
            for p in [
                out.as_deref(),
                outi.as_deref(),
                pulse.as_deref(),
                ir.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                if let Some(parent) = p.parent() {
                    let _ = fs::create_dir_all(parent);
                }
            }

            Status::compiling(format!("{} (via Vale)", src.display()));
            let output = invoke_vale_compiler(&vale_bin, &opts)?;

            let stdout_str = String::from_utf8_lossy(&output.stdout);
            for line in stdout_str.lines() {
                if !line.trim().is_empty() {
                    Status::detail(line.trim());
                }
            }

            Status::finished(format!(
                "Vale compilation finished in {:.2?}",
                start_time.elapsed()
            ));
        }

        Commands::Fuzz { iterations: _ } => {
            Status::finished(
                "Differential fuzzing retired; using native Vale/F* formal verification pipeline.",
            );
        }

        Commands::Completions { shell, install } => {
            use clap::CommandFactory;
            let mut cmd = Cli::command();

            let mut buf = Vec::new();
            clap_complete::generate(*shell, &mut cmd, "valerie", &mut buf);
            let mut script = String::from_utf8(buf)?;

            // Clap_complete generates subcommands for zsh by exact name ("(build)").
            // Patch in aliases so `valerie b <TAB>`, `valerie c <TAB>`, etc. are recognized.
            if *shell == clap_complete::Shell::Zsh {
                let aliases = [
                    ("(build)", "(build|b)"),
                    ("(check)", "(check|c)"),
                    ("(test)", "(test|t)"),
                    ("(run)", "(run|r)"),
                    ("(verify)", "(verify|v)"),
                ];
                for (from, to) in aliases {
                    script = script.replace(from, to);
                }
            }

            if *install {
                let home = std::env::var("HOME").map(PathBuf::from).map_err(|_| {
                    "HOME environment variable not set, cannot locate shell config"
                })?;

                match shell {
                    clap_complete::Shell::Bash => {
                        let target_path = home.join(
                            ".local/share/bash-completion/completions/valerie",
                        );
                        let _ = fs::create_dir_all(target_path.parent().unwrap());
                        fs::write(&target_path, &script)?;
                        Status::finished(format!(
                            "Installed bash completion to `{}`",
                            target_path.display()
                        ));
                    }
                    clap_complete::Shell::Zsh => {
                        // Install to both standard site-functions and oh-my-zsh completion paths if present
                        let primary =
                            home.join(".local/share/zsh/site-functions/_valerie");
                        let _ = fs::create_dir_all(primary.parent().unwrap());
                        fs::write(&primary, &script)?;

                        let omz = home.join(".oh-my-zsh/completions/_valerie");
                        if let Some(parent) = omz.parent() {
                            let _ = fs::create_dir_all(parent);
                            let _ = fs::write(&omz, &script);
                        }

                        Status::finished(format!(
                            "Installed zsh completion to `{}`",
                            primary.display()
                        ));
                    }
                    clap_complete::Shell::Fish => {
                        let target_path =
                            home.join(".config/fish/completions/valerie.fish");
                        let _ = fs::create_dir_all(target_path.parent().unwrap());
                        fs::write(&target_path, &script)?;
                        Status::finished(format!(
                            "Installed fish completion to `{}`",
                            target_path.display()
                        ));
                    }
                    other => {
                        Status::warning(format!(
                            "Auto-install not implemented for {:?}. Please redirect stdout manually.",
                            other
                        ));
                        use std::io::Write;
                        std::io::stdout().write_all(script.as_bytes())?;
                    }
                }
            } else {
                use std::io::Write;
                std::io::stdout().write_all(script.as_bytes())?;
            }
        }
    }

    Ok(())
}
