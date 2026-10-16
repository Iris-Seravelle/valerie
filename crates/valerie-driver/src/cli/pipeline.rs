use super::args::Cli;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use valerie_driver::BackendMode as DriverBackendMode;
use valerie_driver::terminal::Status;
use valerie_driver::vale_bridge::ValeProcedure;
use valerie_driver::vale_bridge::compile_and_extract_vaf;
use valerie_manifest::resolver::ResolvedEnvironment;
use valerie_target::{Architecture, TargetDescriptor};

pub fn parse_target(triple: &str) -> TargetDescriptor {
    if triple.contains("arm32") || triple.contains("armv7") {
        TargetDescriptor::arm32_linux_gnueabihf()
    } else if triple.contains("riscv64") || triple.contains("rv64") {
        TargetDescriptor::riscv64_linux()
    } else if triple.contains("x86_64")
        || triple.contains("amd64")
        || triple.contains("x64")
    {
        if triple.contains("windows") || triple.contains("msvc") {
            TargetDescriptor::x86_64_windows()
        } else {
            TargetDescriptor::x86_64_linux()
        }
    } else if triple.contains("macos") || triple.contains("darwin") {
        TargetDescriptor::macos()
    } else {
        TargetDescriptor::aarch64_linux()
    }
}

/// Auto-detect architecture and crypto extensions from fixture/file names.
pub fn auto_detect_target(
    path: &Path,
    default_target: TargetDescriptor,
    explicit_target: bool,
) -> TargetDescriptor {
    let name = path.to_string_lossy().to_lowercase();
    let base_target = if !explicit_target {
        if name.contains("riscv") || name.contains("rv64") {
            TargetDescriptor::riscv64_linux()
        } else if name.contains("x86") || name.contains("amd64") {
            TargetDescriptor::x86_64_linux()
        } else if name.contains("arm32") || name.contains("armv7") {
            TargetDescriptor::arm32_linux_gnueabihf()
        } else {
            default_target
        }
    } else {
        default_target
    };

    if name.contains("pmull")
        || name.contains("crypto")
        || name.contains("aes")
        || name.contains("sha")
        || name.contains("ghash")
        || name.contains("polyval")
    {
        base_target.with_crypto()
    } else {
        base_target
    }
}

/// Prints comprehensive execution environment, tool discovery, and target specs when verbose mode is enabled.
pub fn print_verbose_header(
    target: &TargetDescriptor,
    _backend: DriverBackendMode,
    cli: &Cli,
    env: &ResolvedEnvironment,
) {
    if !cli.verbose {
        return;
    }

    Status::diagnostic(
        "Target",
        format!(
            "{:?} (pointer: {} bits, stack alignment: {}B)",
            target.architecture,
            target.architecture.pointer_width_bits(),
            target.architecture.stack_alignment_bytes()
        ),
    );
    Status::detail(format!("ABI:              {:?}", target.abi));
    Status::detail(format!("Object format:    {:?}", target.object_format));
    if target.architecture == Architecture::X86_64 {
        Status::detail(format!(
            "Features:         AES-NI: {}, SHA-NI: {}, PCLMUL: {}, AVX2: {}, DOITM: {}",
            target.features.has_aesni,
            target.features.has_sha_ni,
            target.features.has_pclmulqdq,
            target.features.has_avx2,
            target.features.has_doitm,
        ));
    } else {
        Status::detail(format!(
            "Features:         Crypto: {}, LSE: {}, DIT: {}, Thumb2: {}",
            target.features.has_crypto,
            target.features.has_lse,
            target.features.has_dit,
            target.features.is_thumb2
        ));
    }

    let vale_bin = env
        .vale_exe
        .as_ref()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("vale"));
    let vale_avail = valerie_driver::vale_bridge::is_vale_available(&vale_bin);
    let fstar_avail =
        env.fstar_exe.is_some() || valerie_driver::vale_bridge::is_fstar_available();
    let clang_ver = env
        .clang_exe
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .or_else(|| {
            Command::new("clang")
                .arg("--version")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.lines().next().map(|l| l.to_string()))
        })
        .unwrap_or_else(|| "clang not detected (using system cc)".to_string());

    let qemu_bin = match target.architecture {
        Architecture::Arm32 => "qemu-arm",
        Architecture::X86_64 => "qemu-x86_64",
        Architecture::Riscv64 => "qemu-riscv64",
        Architecture::AArch64 => "qemu-aarch64",
    };

    Status::diagnostic(
        "Toolchain",
        format!(
            "Vale: {}, F*: {}, CC: {}",
            if vale_avail { "available" } else { "missing" },
            if fstar_avail { "available" } else { "missing" },
            clang_ver
        ),
    );

    let engine = if target.is_host() {
        "Native Hardware Execution (with Differential Verification)".to_string()
    } else {
        format!("QEMU User Emulation [binary: {}]", qemu_bin)
    };
    Status::diagnostic("Engine", engine);
}

/// Extracts Vale procedures from an input file (.vaf or .fst).
pub fn extract_procedures_from_input(
    input_path: &Path,
    temp_dir: &Path,
    vale_bin: &Path,
) -> Result<Vec<ValeProcedure>, Box<dyn std::error::Error>> {
    let ext = input_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    match ext {
        "vaf" => {
            let stem = input_path.file_stem().unwrap_or_default().to_string_lossy();
            let out_fst = temp_dir.join(format!("{}.fst", stem));
            let out_fsti = temp_dir.join(format!("{}.fsti", stem));
            let procs = compile_and_extract_vaf(vale_bin, input_path, &out_fst, &out_fsti)?;
            Ok(procs)
        }
        "fst" => {
            let fst_content = fs::read_to_string(input_path)?;
            let procs = valerie_driver::vale_bridge::extract_procedures_from_fst(&fst_content)?;
            Ok(procs)
        }
        _ => Err(format!(
            "File `{}` does not contain structured Vale procedures (.vaf or .fst expected)",
            input_path.display()
        )
        .into()),
    }
}

/// Loads an input file (.vaf, .fst, or .s) and converts it to assembly text.
pub fn load_or_extract_asm(
    input_path: &Path,
    target: &TargetDescriptor,
    temp_dir: &Path,
    with_harness: bool,
    vale_bin: &Path,
) -> Result<String, Box<dyn std::error::Error>> {
    load_or_extract_asm_with_aliases(
        input_path,
        target,
        temp_dir,
        with_harness,
        vale_bin,
        &std::collections::HashMap::new(),
    )
}

/// Loads an input file (.vaf, .fst, or .s) and converts it to assembly text with alias definitions.
pub fn load_or_extract_asm_with_aliases(
    input_path: &Path,
    target: &TargetDescriptor,
    temp_dir: &Path,
    _with_harness: bool,
    vale_bin: &Path,
    _aliases: &std::collections::HashMap<String, Vec<String>>,
) -> Result<String, Box<dyn std::error::Error>> {
    let ext = input_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    if ext == "s" || ext == "asm" {
        return Ok(fs::read_to_string(input_path)?);
    }

    if ext == "vaf" {
        let stem = input_path.file_stem().unwrap_or_default().to_string_lossy();
        let out_asm = temp_dir.join(format!("{}.s", stem));
        let arch_str = match target.architecture {
            Architecture::AArch64 => "aarch64",
            Architecture::Arm32 => "arm32",
            Architecture::X86_64 => "x64",
            Architecture::Riscv64 => "riscv64",
        };
        let mut opts =
            valerie_driver::vale_bridge::ValeCompileOptions::new(input_path);
        opts.output_asm = Some(&out_asm);
        opts.arch = Some(arch_str);
        valerie_driver::vale_bridge::invoke_vale_compiler(vale_bin, &opts)?;
        if out_asm.exists() {
            let asm_content = fs::read_to_string(&out_asm)?;
            return Ok(asm_content);
        }
    }

    Err(format!(
        "Cannot load or extract assembly from `{}`: unsupported file format",
        input_path.display()
    )
    .into())
}
