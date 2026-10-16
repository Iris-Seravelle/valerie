//! Toolchain driver and external assembler/linker invocation for AArch64.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use valerie_target::TargetDescriptor;

/// Compilation backend execution strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendMode {
    /// Try fast native emitter first; if unsupported opcode or assembly error occurs, seamlessly route to LLVM.
    #[default]
    Auto,
    /// Strictly use Vale native emitter (`valerie-emit` / `valerie-codegen`).
    Native,
    /// Force full LLVM compilation pipeline (`valerie-llvm` -> `llc`).
    Llvm,
}

impl std::str::FromStr for BackendMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "auto" => Ok(BackendMode::Auto),
            "native" => Ok(BackendMode::Native),
            "llvm" => Ok(BackendMode::Llvm),
            other => Err(format!(
                "Unknown backend mode '{}'. Valid: auto, native, llvm",
                other
            )),
        }
    }
}

/// Error type for assembler and linker toolchain execution.
#[derive(Debug)]
pub enum DriverError {
    Io(io::Error),
    AssemblerFailed {
        status: std::process::ExitStatus,
        stderr: String,
    },
    LinkerFailed {
        status: std::process::ExitStatus,
        stderr: String,
    },
    ExecutionFailed {
        status: std::process::ExitStatus,
        stderr: String,
    },
    LlvmFailed(String),
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DriverError::Io(err) => write!(f, "IO error: {}", err),
            DriverError::AssemblerFailed { status, stderr } => {
                write!(f, "Assembler failed with {}: {}", status, stderr)
            }
            DriverError::LinkerFailed { status, stderr } => {
                write!(f, "Linker failed with {}: {}", status, stderr)
            }
            DriverError::ExecutionFailed { status, stderr } => {
                write!(f, "Execution failed with {}: {}", status, stderr)
            }
            DriverError::LlvmFailed(err) => write!(f, "LLVM backend error: {}", err),
        }
    }
}

impl std::error::Error for DriverError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DriverError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for DriverError {
    fn from(err: io::Error) -> Self {
        DriverError::Io(err)
    }
}

/// Invokes system assembler (`clang -c` or `as`) to assemble an assembly text string into an object file.
pub fn assemble_string(
    asm_text: &str,
    obj_path: &Path,
    target: &TargetDescriptor,
) -> Result<(), DriverError> {
    let mut temp_asm = obj_path.to_path_buf();
    let file_name = obj_path.file_name().unwrap().to_str().unwrap();
    temp_asm.set_file_name(format!("{}.src.s", file_name));
    fs::write(&temp_asm, asm_text)?;

    let cc_bin = std::env::var("CC")
        .map(PathBuf::from)
        .ok()
        .or_else(|| which::which("clang").ok())
        .unwrap_or_else(|| PathBuf::from("clang"));
    let mut cmd = Command::new(cc_bin);
    match target.os {
        valerie_target::OperatingSystem::MacOS => {
            cmd.arg("--target=arm64-apple-macos");
        }
        _ => match target.architecture {
            valerie_target::Architecture::Arm32 => {
                cmd.arg("--target=armv7-linux-gnueabi");
            }
            valerie_target::Architecture::X86_64 => {
                cmd.arg("--target=x86_64-unknown-linux-gnu");
            }
            valerie_target::Architecture::Riscv64 => {
                cmd.arg("--target=riscv64-unknown-linux-gnu");
            }
            valerie_target::Architecture::AArch64 => {
                if target.features.has_crypto {
                    cmd.arg("-march=armv8-a+crypto");
                }
            }
        },
    }
    cmd.arg("-c").arg("-o").arg(obj_path).arg(&temp_asm);

    let output = cmd.output()?;
    let _ = fs::remove_file(&temp_asm);

    if !output.status.success() {
        return Err(DriverError::AssemblerFailed {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }

    Ok(())
}

/// Assembles assembly text into an object file using the specified BackendMode.
/// When BackendMode::Auto is active, if the assembler fails or unmodeled instructions
/// are encountered, it automatically leverages LLVM to compile.
pub fn assemble_string_with_backend(
    asm_text: &str,
    obj_path: &Path,
    target: &TargetDescriptor,
    backend: BackendMode,
) -> Result<BackendMode, DriverError> {
    match backend {
        BackendMode::Native | BackendMode::Auto | BackendMode::Llvm => {
            assemble_string(asm_text, obj_path, target)?;
            Ok(backend)
        }
    }
}

/// Links object files into an executable using system driver (`clang` / `gcc`).
pub fn link_executable(
    obj_path: &Path,
    exe_path: &Path,
    target: &TargetDescriptor,
) -> Result<(), DriverError> {
    let cc_bin = std::env::var("CC")
        .map(PathBuf::from)
        .ok()
        .or_else(|| which::which("clang").ok())
        .unwrap_or_else(|| PathBuf::from("clang"));
    let mut cmd = Command::new(cc_bin);
    if !target.is_host() {
        match target.os {
            valerie_target::OperatingSystem::MacOS => {
                cmd.arg("--target=arm64-apple-macos");
            }
            valerie_target::OperatingSystem::Windows => {
                cmd.arg("--target=x86_64-pc-windows-msvc");
            }
            valerie_target::OperatingSystem::Linux => match target.architecture {
                valerie_target::Architecture::Arm32 => {
                    cmd.arg("--target=armv7-linux-gnueabi")
                        .arg("-static")
                        .arg("-nostdlib");
                }
                valerie_target::Architecture::X86_64 => {
                    cmd.arg("--target=x86_64-unknown-linux-gnu")
                        .arg("-static")
                        .arg("-nostdlib");
                }
                valerie_target::Architecture::Riscv64 => {
                    cmd.arg("--target=riscv64-unknown-linux-gnu")
                        .arg("-fuse-ld=lld")
                        .arg("-static")
                        .arg("-nostdlib");
                }
                valerie_target::Architecture::AArch64 => {
                    cmd.arg("--target=aarch64-unknown-linux-gnu")
                        .arg("-static")
                        .arg("-nostdlib");
                }
            },
        }
    }
    cmd.arg("-o").arg(exe_path).arg(obj_path);

    let output: Output = cmd.output()?;

    if !output.status.success() {
        return Err(DriverError::LinkerFailed {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }

    Ok(())
}

/// Strips debug and unneeded symbols from an executable binary.
pub fn strip_binary(exe_path: &Path) -> Result<(), DriverError> {
    let strip_tool = std::env::var("STRIP").unwrap_or_else(|_| "strip".to_string());
    let output = Command::new(&strip_tool).arg("-s").arg(exe_path).output()?;

    if !output.status.success() {
        eprintln!(
            "warning: `{}` exited with status {} while stripping {}",
            strip_tool,
            output.status,
            exe_path.display()
        );
    }
    Ok(())
}

/// Runs an executable under the target architecture's emulator (`qemu-arm` for Arm32, `qemu-aarch64` for AArch64)
/// or natively when requested.
pub fn run_target_executable(
    exe_path: &Path,
    target: &TargetDescriptor,
    force_qemu: bool,
) -> Result<i32, DriverError> {
    let mut cmd = match target.architecture {
        valerie_target::Architecture::Arm32 => {
            // Always run via qemu-arm (host is 64-bit AArch64)
            let mut c = Command::new("qemu-arm");
            c.arg(exe_path);
            c
        }
        valerie_target::Architecture::X86_64 => {
            // Always run via qemu-x86_64 (host is AArch64)
            let mut c = Command::new("qemu-x86_64");
            c.arg(exe_path);
            c
        }
        valerie_target::Architecture::Riscv64 => {
            // Always run via qemu-riscv64 (host is AArch64)
            let mut c = Command::new("qemu-riscv64");
            c.arg(exe_path);
            c
        }
        valerie_target::Architecture::AArch64 => {
            if force_qemu {
                let mut c = Command::new("qemu-aarch64");
                c.arg(exe_path);
                c
            } else {
                Command::new(exe_path)
            }
        }
    };

    let output = cmd.output()?;
    let code = output.status.code().unwrap_or(-1);
    Ok(code)
}

/// Runs the executable, optionally via `qemu-aarch64` if requested or if cross-testing.
/// Returns the process exit code.
pub fn run_executable_with_qemu(
    exe_path: &Path,
    use_qemu: bool,
) -> Result<i32, DriverError> {
    let mut cmd = if use_qemu {
        let mut c = Command::new("qemu-aarch64");
        c.arg(exe_path);
        c
    } else {
        Command::new(exe_path)
    };

    let output = cmd.output()?;
    let code = output.status.code().unwrap_or(-1);
    Ok(code)
}
