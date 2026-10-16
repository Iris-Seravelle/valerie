use super::pipeline::load_or_extract_asm;
use std::fs;
use std::path::Path;
use valerie_driver::BackendMode as DriverBackendMode;
use valerie_driver::terminal::Status;
use valerie_driver::vale_bridge::{
    compile_and_extract_vaf, is_fstar_available, verify_fst_with_fstar,
};
use valerie_driver::{QemuConfig, TargetTestRunner};
use valerie_target::Architecture;
use valerie_target::TargetDescriptor;

pub fn test_single_file(
    input: &Path,
    target: &TargetDescriptor,
    backend_mode: DriverBackendMode,
    force_qemu: bool,
    cpu_model: Option<&str>,
    skip_verify: bool,
    verbose: bool,
    vale_bin: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let ext = input.extension().and_then(|s| s.to_str()).unwrap_or("");
    let is_vale = ext == "vaf";

    let temp_dir = std::env::temp_dir().join("valerie_emitter_test_tmp");
    let _ = fs::create_dir_all(&temp_dir);
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    let out_fst = temp_dir.join(format!("{}.fst", stem));
    let out_fsti = temp_dir.join(format!("{}.fsti", stem));

    if verbose {
        Status::diagnostic(
            "Test Engine",
            format!("Running verification & execution on `{}`", input.display()),
        );
        Status::detail(format!("Target architecture: {:?}", target.architecture));
        Status::detail(format!("Assembler backend:   {:?}", backend_mode));
        Status::detail(format!("Intermediate dir:    {}", temp_dir.display()));
    }

    let asm = if is_vale {
        Status::compiling(format!(
            "Vale spec {} -> formal verification -> assembly -> e2e execution",
            input.display()
        ));

        let procs = compile_and_extract_vaf(vale_bin, input, &out_fst, &out_fsti)?;

        if verbose {
            Status::detail(format!(
                "Extracted {} Vale procedure(s) into AST",
                procs.len()
            ));
            for p in &procs {
                Status::detail(format!(
                    "  - Procedure `{}`: {} instructions",
                    p.name,
                    p.instructions.len()
                ));
            }
        }

        if !skip_verify && is_fstar_available() {
            Status::testing(format!(
                "F* formal verification on {}",
                out_fst.display()
            ));
            if verbose {
                Status::detail(format!(
                    "Verification interface: {}",
                    out_fsti.display()
                ));
                Status::detail(format!(
                    "Checked cache dir:      {}",
                    temp_dir.display()
                ));
            }
            verify_fst_with_fstar(&out_fsti, &out_fst, &temp_dir)?;
            Status::finished("F* verification conditions discharged successfully");
        } else if !skip_verify {
            eprintln!("Notice: F* not available. Skipping F* lemma verification.");
        }

        load_or_extract_asm(input, target, &temp_dir, true, vale_bin)?
    } else {
        Status::compiling(format!(
            "Assembly source {} -> differential execution",
            input.display()
        ));
        fs::read_to_string(input)?
    };

    Status::assembling(format!(
        "Generated assembly -> binary execution (backend: {:?})",
        backend_mode
    ));

    let mut qemu_cfg = QemuConfig::new();
    if let Some(cpu) = cpu_model {
        qemu_cfg = qemu_cfg.with_cpu(cpu);
    }

    let test_runner =
        TargetTestRunner::new(format!("cli_test_{}", stem), target.clone())
            .backend_mode(backend_mode)
            .qemu_config(qemu_cfg)
            .from_asm(asm);

    let res = if force_qemu
        || target.architecture == Architecture::Arm32
        || target.architecture == Architecture::X86_64
    {
        if verbose {
            Status::detail("Executing test harness under QEMU user-mode emulation");
        }
        test_runner.run_qemu()?
    } else {
        if verbose {
            Status::detail(
                "Executing 3-way differential runner: Native Hardware vs QEMU Emulator",
            );
        }
        let (native_res, _) = test_runner.run_differential()?;
        native_res
    };

    res.assert_exit_code(0);
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("test");
    Status::test_suite_header(1);
    Status::test_case(stem, true, Some(res.duration));
    Status::test_suite_result(1, 0, res.duration);

    Ok(())
}
