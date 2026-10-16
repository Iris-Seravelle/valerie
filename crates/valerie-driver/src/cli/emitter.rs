//! Code emission pipeline for assembly, F*, Pulse, and IR.

use std::fs;
use std::path::{Path, PathBuf};
use valerie_driver::terminal::Status;
use valerie_driver::vale_bridge::{ValeCompileOptions, invoke_vale_compiler};
use valerie_manifest::Manifest;
use valerie_target::{Architecture, TargetDescriptor};

use crate::cli::args::Cli;
use crate::cli::pipeline::{auto_detect_target, load_or_extract_asm};

pub fn emit_target(
    file: &Option<PathBuf>,
    output: &Option<PathBuf>,
    fst: &Option<PathBuf>,
    fsti: &Option<PathBuf>,
    pulse: &Option<PathBuf>,
    ir: &Option<PathBuf>,
    cli: &Cli,
    mut target: TargetDescriptor,
    has_explicit_target: bool,
    vale_bin: &Path,
    manifest_ctx: Option<&(PathBuf, Manifest)>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(input_file) = file {
        target = auto_detect_target(input_file, target, has_explicit_target);

        // If any Vale proof/IR artifact output is requested (.fst, .fsti, pulse, ir)
        if fst.is_some() || fsti.is_some() || pulse.is_some() || ir.is_some() {
            let arch_str = match target.architecture {
                Architecture::AArch64 => "aarch64",
                Architecture::Arm32 => "arm32",
                Architecture::X86_64 => "x64",
                Architecture::Riscv64 => "riscv64",
            };

            let mut opts = ValeCompileOptions::new(input_file);
            opts.output_fst = fst.as_deref();
            opts.output_fsti = fsti.as_deref();
            opts.output_pulse = pulse.as_deref();
            opts.output_ir = ir.as_deref();
            opts.arch = Some(arch_str);

            if cli.verbose {
                Status::diagnostic(
                    "Vale Flags",
                    format!(
                        "-in {} {}{}{}{}-arch {}",
                        input_file.display(),
                        fst.as_ref()
                            .map(|p| format!("-out {} ", p.display()))
                            .unwrap_or_default(),
                        fsti.as_ref()
                            .map(|p| format!("-outi {} ", p.display()))
                            .unwrap_or_default(),
                        pulse
                            .as_ref()
                            .map(|p| format!("-pulse {} ", p.display()))
                            .unwrap_or_default(),
                        ir.as_ref()
                            .map(|p| format!("-ir {} ", p.display()))
                            .unwrap_or_default(),
                        arch_str
                    ),
                );
            }

            invoke_vale_compiler(vale_bin, &opts)?;

            if let Some(p) = fst {
                Status::finished(format!(
                    "Wrote F* implementation to {}",
                    p.display()
                ));
            }
            if let Some(p) = fsti {
                Status::finished(format!("Wrote F* interface to {}", p.display()));
            }
            if let Some(p) = pulse {
                Status::finished(format!("Wrote Pulse module to {}", p.display()));
            }
            if let Some(p) = ir {
                Status::finished(format!("Wrote Vale IR to {}", p.display()));
            }
        }

        // If assembly output or stdout is requested
        if output.is_some()
            || (fst.is_none() && fsti.is_none() && pulse.is_none() && ir.is_none())
        {
            let temp_dir = std::env::temp_dir().join("valerie_emitter_tmp");
            let _ = fs::create_dir_all(&temp_dir);

            let asm = match load_or_extract_asm(
                input_file, &target, &temp_dir, false, vale_bin,
            ) {
                Ok(s) => s,
                Err(e) => {
                    return Err(format!("Error extracting assembly: {}", e).into());
                }
            };

            if let Some(out_p) = output {
                if let Some(parent) = out_p.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::write(out_p, &asm)?;
                Status::finished(format!("Emitted assembly to {}", out_p.display()));
            } else if fst.is_none()
                && fsti.is_none()
                && pulse.is_none()
                && ir.is_none()
            {
                println!("{}", asm);
            }
        }
    } else if let Some((_, manifest)) = manifest_ctx {
        // Workspace mode: emit all sources in manifest.assembly.sources
        let assembly_cfg = manifest
            .assembly
            .as_ref()
            .ok_or("Manifest is missing [assembly] table")?;
        let out_dir = assembly_cfg
            .output_dir
            .as_deref()
            .unwrap_or(Path::new("src/asm"));
        fs::create_dir_all(out_dir)?;
        let temp_dir = std::env::temp_dir().join("valerie_emitter_workspace");
        let _ = fs::create_dir_all(&temp_dir);

        let total_sources = assembly_cfg.sources.len();
        for (idx, src) in assembly_cfg.sources.iter().enumerate() {
            Status::compiling(format!(
                "[{}/{}] Lowering {} -> assembly",
                idx + 1,
                total_sources,
                src.display()
            ));
            let src_target =
                auto_detect_target(src, target.clone(), has_explicit_target);
            let asm = match load_or_extract_asm(
                src,
                &src_target,
                &temp_dir,
                false,
                vale_bin,
            ) {
                Ok(s) => s,
                Err(e) => {
                    return Err(format!(
                        "Error extracting assembly for `{}`: {}",
                        src.display(),
                        e
                    )
                    .into());
                }
            };
            let stem = src.file_stem().unwrap_or_default().to_string_lossy();
            let out_path = out_dir.join(format!("{}.s", stem));
            fs::write(&out_path, &asm)?;
            Status::finished(format!("Emitted assembly to {}", out_path.display()));
        }
    } else {
        return Err(
            "`emit` requires an input file when not in a valerie workspace.".into(),
        );
    }
    Ok(())
}
