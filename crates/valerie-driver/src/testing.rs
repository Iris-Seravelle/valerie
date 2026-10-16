//! Unified testing and execution harness for Vale backends across Native and QEMU.
//!
//! Provides customizable, fluent builders and runner handlers for:
//! - Compiling and linking assembly source text or pre-existing `.s` test files
//! - Executing natively or under architecture-specific QEMU emulators (`qemu-aarch64`, `qemu-arm`)
//! - Capturing stdout, stderr, execution duration, and exit status
//! - Configuring custom QEMU arguments, CPU models, sysroots, environment variables, and timeouts
//! - Fluent assertions (`assert_exit_code`, `assert_stdout_contains`, `assert_native_and_qemu_match`)

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

use valerie_target::{Architecture, TargetDescriptor};

use crate::driver::{DriverError, link_executable};

/// Execution mode for a test case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionEngine {
    /// Execute directly on the host operating system.
    Native,
    /// Execute inside QEMU user-mode emulation (`qemu-aarch64`, `qemu-arm`, etc.).
    Qemu,
    /// Automatically determine best engine: Native if host matches target, else QEMU.
    Auto,
}

/// Detailed result of executing an assembled binary.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Exit code from process (e.g. 0 for success).
    pub exit_code: i32,
    /// Standard output captured as a UTF-8 string.
    pub stdout: String,
    /// Standard error captured as a UTF-8 string.
    pub stderr: String,
    /// Process exit status.
    pub status: ExitStatus,
    /// Elapsed wall-clock execution time.
    pub duration: Duration,
    /// Engine that performed execution.
    pub engine: ExecutionEngine,
}

impl ExecutionResult {
    /// Asserts that the process exited with the expected integer exit code.
    pub fn assert_exit_code(&self, expected: i32) -> &Self {
        assert_eq!(
            self.exit_code, expected,
            "Execution exit code mismatch! Expected {}, got {}.\nSTDOUT: {}\nSTDERR: {}",
            expected, self.exit_code, self.stdout, self.stderr
        );
        self
    }

    /// Asserts that stdout contains a given substring.
    pub fn assert_stdout_contains(&self, substring: &str) -> &Self {
        assert!(
            self.stdout.contains(substring),
            "Expected stdout to contain {:?}, but got:\n{}",
            substring,
            self.stdout
        );
        self
    }

    /// Asserts that stderr contains a given substring.
    pub fn assert_stderr_contains(&self, substring: &str) -> &Self {
        assert!(
            self.stderr.contains(substring),
            "Expected stderr to contain {:?}, but got:\n{}",
            substring,
            self.stderr
        );
        self
    }

    /// Asserts that the execution succeeded (exit code == 0).
    pub fn assert_success(&self) -> &Self {
        self.assert_exit_code(0)
    }
}

/// Customizable QEMU runner handler and configuration.
#[derive(Debug, Clone, Default)]
pub struct QemuConfig {
    /// Custom path to qemu binary (e.g. `qemu-aarch64` or custom path).
    pub qemu_binary: Option<PathBuf>,
    /// CPU model passed to `-cpu` flag (e.g. `cortex-a53`, `max`).
    pub cpu: Option<String>,
    /// Sysroot directory passed to `-L` flag for shared library resolution.
    pub sysroot: Option<PathBuf>,
    /// GDB server port or argument passed to `-g` flag for remote debugging.
    pub gdb_port: Option<u16>,
    /// Extra flags forwarded directly to QEMU before the binary path.
    pub extra_qemu_args: Vec<String>,
    /// Environment variables to pass to QEMU (`-E KEY=VALUE`).
    pub env_vars: HashMap<String, String>,
}

impl QemuConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_cpu(mut self, cpu: impl Into<String>) -> Self {
        self.cpu = Some(cpu.into());
        self
    }

    pub fn with_sysroot(mut self, sysroot: impl AsRef<Path>) -> Self {
        self.sysroot = Some(sysroot.as_ref().to_path_buf());
        self
    }

    pub fn with_gdb(mut self, port: u16) -> Self {
        self.gdb_port = Some(port);
        self
    }

    pub fn with_arg(mut self, arg: impl Into<String>) -> Self {
        self.extra_qemu_args.push(arg.into());
        self
    }

    pub fn with_env(
        mut self,
        key: impl Into<String>,
        val: impl Into<String>,
    ) -> Self {
        self.env_vars.insert(key.into(), val.into());
        self
    }

    /// Selects default QEMU emulator binary name for a given architecture.
    pub fn default_binary_name(arch: Architecture) -> &'static str {
        match arch {
            Architecture::AArch64 => "qemu-aarch64",
            Architecture::Arm32 => "qemu-arm",
            Architecture::X86_64 => "qemu-x86_64",
            Architecture::Riscv64 => "qemu-riscv64",
        }
    }
}

/// Fluent test case runner configuring compilation, linking, and execution.
pub struct TargetTestRunner {
    target: TargetDescriptor,
    source_asm: Option<String>,
    source_file: Option<PathBuf>,
    extra_c_sources: Vec<PathBuf>,
    output_dir: PathBuf,
    name: String,
    program_args: Vec<String>,
    qemu_config: QemuConfig,
    backend_mode: crate::driver::BackendMode,
    clean_artifacts: bool,
}

impl TargetTestRunner {
    /// Creates a new test runner for the specified target descriptor and test name.
    pub fn new(name: impl Into<String>, target: TargetDescriptor) -> Self {
        let name_str = name.into();
        let default_dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("target")
            .join("vale_test_artifacts");
        Self {
            target,
            source_asm: None,
            source_file: None,
            extra_c_sources: Vec::new(),
            output_dir: default_dir,
            name: name_str,
            program_args: Vec::new(),
            qemu_config: QemuConfig::default(),
            backend_mode: crate::driver::BackendMode::Auto,
            clean_artifacts: true,
        }
    }

    /// Set backend mode (Native, Llvm, Auto).
    pub fn backend_mode(mut self, mode: crate::driver::BackendMode) -> Self {
        self.backend_mode = mode;
        self
    }

    /// Provide raw assembly text source code.
    pub fn from_asm(mut self, asm_text: impl Into<String>) -> Self {
        self.source_asm = Some(asm_text.into());
        self
    }

    /// Provide a path to an existing assembly source file (`.s`).
    pub fn from_file(mut self, path: impl AsRef<Path>) -> Self {
        self.source_file = Some(path.as_ref().to_path_buf());
        self
    }

    /// Add a C source file to be co-compiled and linked (e.g. for C ABI torture testing).
    pub fn with_c_source(mut self, c_path: impl AsRef<Path>) -> Self {
        self.extra_c_sources.push(c_path.as_ref().to_path_buf());
        self
    }

    /// Set custom output directory for intermediate `.o` and final executables.
    pub fn output_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.output_dir = dir.as_ref().to_path_buf();
        self
    }

    /// Configure runtime arguments passed to the executed binary.
    pub fn with_arg(mut self, arg: impl Into<String>) -> Self {
        self.program_args.push(arg.into());
        self
    }

    /// Customize QEMU options.
    pub fn qemu_config(mut self, config: QemuConfig) -> Self {
        self.qemu_config = config;
        self
    }

    /// Retain or delete generated `.o` and binary files after test execution (default: true = delete).
    pub fn clean_artifacts(mut self, clean: bool) -> Self {
        self.clean_artifacts = clean;
        self
    }

    /// Compiles and links the binary, returning paths to the object and executable files.
    pub fn build(&self) -> Result<PathBuf, DriverError> {
        fs::create_dir_all(&self.output_dir)?;

        let obj_path = self.output_dir.join(format!("{}.o", self.name));
        let exe_path = self.output_dir.join(&self.name);

        let asm_content = if let Some(ref text) = self.source_asm {
            text.clone()
        } else if let Some(ref path) = self.source_file {
            fs::read_to_string(path)?
        } else {
            return Err(DriverError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "No assembly source provided to TargetTestRunner",
            )));
        };

        let has_entrypoint = asm_content.contains("main:")
            || asm_content.contains("_start:")
            || asm_content.contains(".globl main")
            || asm_content.contains(".global main")
            || asm_content.contains(".globl _start")
            || asm_content.contains(".global _start");

        if !has_entrypoint && self.extra_c_sources.is_empty() {
            // Standalone cryptographic procedure without an entrypoint:
            // Synthesize a freestanding Linux _start entrypoint with direct system exit.
            let procs =
                crate::vale_bridge::extract_procedures_from_asm(&asm_content);
            let target_fn =
                procs.first().map(|p| p.name.as_str()).unwrap_or(&self.name);

            let entry_stub = match self.target.architecture {
                Architecture::X86_64 => format!(
                    "\n.text\n.globl _start\n_start:\n    subq $136, %rsp\n    movq %rsp, %rdi\n    call {}\n    movq $60, %rax\n    xorq %rdi, %rdi\n    syscall\n",
                    target_fn
                ),
                Architecture::Riscv64 => format!(
                    "\n.text\n.globl _start\n_start:\n    addi sp, sp, -144\n    addi a0, sp, 16\n    call {}\n    li a7, 93\n    li a0, 0\n    ecall\n",
                    target_fn
                ),
                Architecture::Arm32 => format!(
                    "\n.text\n.globl _start\n_start:\n    sub sp, sp, #128\n    mov r0, sp\n    bl {}\n    mov r7, #1\n    mov r0, #0\n    swi 0\n",
                    target_fn
                ),
                Architecture::AArch64 => format!(
                    "\n.text\n.globl _start\n_start:\n    sub sp, sp, #128\n    mov x0, sp\n    bl {}\n    mov x8, #93\n    mov x0, #0\n    svc 0\n",
                    target_fn
                ),
            };

            let full_asm = format!("{}\n{}", asm_content, entry_stub);
            crate::driver::assemble_string_with_backend(
                &full_asm,
                &obj_path,
                &self.target,
                self.backend_mode,
            )?;

            let mut cmd = Command::new("clang");
            match self.target.architecture {
                Architecture::Arm32 => {
                    cmd.arg("--target=armv7-linux-gnueabi")
                        .arg("-fuse-ld=lld")
                        .arg("-static")
                        .arg("-nostdlib");
                }
                Architecture::X86_64 => {
                    cmd.arg("--target=x86_64-unknown-linux-gnu")
                        .arg("-fuse-ld=lld")
                        .arg("-static")
                        .arg("-nostdlib");
                }
                Architecture::Riscv64 => {
                    cmd.arg("--target=riscv64-unknown-linux-gnu")
                        .arg("-fuse-ld=lld")
                        .arg("-static")
                        .arg("-nostdlib");
                }
                Architecture::AArch64 => {
                    cmd.arg("--target=aarch64-unknown-linux-gnu")
                        .arg("-fuse-ld=lld")
                        .arg("-static")
                        .arg("-nostdlib");
                }
            }
            cmd.arg(&obj_path).arg("-o").arg(&exe_path);

            let out = cmd.output()?;

            if !out.status.success() {
                return Err(DriverError::LinkerFailed {
                    status: out.status,
                    stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                });
            }
        } else if !self.extra_c_sources.is_empty() {
            // Mixed C + Assembly compilation via Clang driver
            let mut cmd = Command::new("clang");
            if !self.target.is_host() {
                match self.target.os {
                    valerie_target::OperatingSystem::Linux => {
                        match self.target.architecture {
                            Architecture::Arm32 => {
                                cmd.arg("--target=armv7-linux-gnueabi");
                            }
                            Architecture::X86_64 => {
                                cmd.arg("--target=x86_64-unknown-linux-gnu");
                            }
                            Architecture::Riscv64 => {
                                cmd.arg("--target=riscv64-unknown-linux-gnu");
                            }
                            Architecture::AArch64 => {
                                cmd.arg("--target=aarch64-unknown-linux-gnu");
                            }
                        }
                    }
                    _ => {}
                }
            }
            for c_src in &self.extra_c_sources {
                cmd.arg(c_src);
            }
            if let Some(ref path) = self.source_file {
                cmd.arg(path);
            } else {
                let temp_asm = self.output_dir.join(format!("{}.s", self.name));
                fs::write(&temp_asm, &asm_content)?;
                cmd.arg(&temp_asm);
            }
            cmd.arg("-o").arg(&exe_path);
            let out = cmd.output()?;
            if !out.status.success() {
                return Err(DriverError::LinkerFailed {
                    status: out.status,
                    stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                });
            }
        } else {
            // Standard assemble and link standalone assembly binary
            crate::driver::assemble_string_with_backend(
                &asm_content,
                &obj_path,
                &self.target,
                self.backend_mode,
            )?;
            link_executable(&obj_path, &exe_path, &self.target)?;
        }

        Ok(exe_path)
    }

    /// Executes the test case under the chosen execution engine.
    pub fn run(
        &self,
        engine: ExecutionEngine,
    ) -> Result<ExecutionResult, DriverError> {
        let exe_path = self.build()?;
        let result = self.execute_binary(&exe_path, engine);

        if self.clean_artifacts {
            let _ = fs::remove_file(&exe_path);
            let obj_path = self.output_dir.join(format!("{}.o", self.name));
            let _ = fs::remove_file(obj_path);
            let temp_s = self.output_dir.join(format!("{}.s", self.name));
            let _ = fs::remove_file(temp_s);
        }

        result
    }

    /// Executes natively.
    pub fn run_native(&self) -> Result<ExecutionResult, DriverError> {
        self.run(ExecutionEngine::Native)
    }

    /// Executes under QEMU user-mode emulator.
    pub fn run_qemu(&self) -> Result<ExecutionResult, DriverError> {
        self.run(ExecutionEngine::Qemu)
    }

    /// Runs under BOTH Native and QEMU, asserting identical exit codes and outputs.
    pub fn run_differential(
        &self,
    ) -> Result<(ExecutionResult, ExecutionResult), DriverError> {
        let exe_path = self.build()?;

        let native_res = self.execute_binary(&exe_path, ExecutionEngine::Native)?;
        let qemu_res = self.execute_binary(&exe_path, ExecutionEngine::Qemu)?;

        assert_eq!(
            native_res.exit_code, qemu_res.exit_code,
            "Differential execution divergence! Native exit code ({}) != QEMU exit code ({})",
            native_res.exit_code, qemu_res.exit_code
        );
        assert_eq!(
            native_res.stdout, qemu_res.stdout,
            "Differential execution divergence in stdout!\nNative:\n{}\nQEMU:\n{}",
            native_res.stdout, qemu_res.stdout
        );

        if self.clean_artifacts {
            let _ = fs::remove_file(&exe_path);
            let obj_path = self.output_dir.join(format!("{}.o", self.name));
            let _ = fs::remove_file(obj_path);
        }

        Ok((native_res, qemu_res))
    }

    fn execute_binary(
        &self,
        exe_path: &Path,
        engine: ExecutionEngine,
    ) -> Result<ExecutionResult, DriverError> {
        let start = Instant::now();
        let mut cmd = match engine {
            ExecutionEngine::Native => {
                let mut c = Command::new(exe_path);
                c.args(&self.program_args);
                c
            }
            ExecutionEngine::Qemu | ExecutionEngine::Auto => {
                let bin_name =
                    self.qemu_config.qemu_binary.clone().unwrap_or_else(|| {
                        PathBuf::from(QemuConfig::default_binary_name(
                            self.target.architecture,
                        ))
                    });
                let mut c = Command::new(bin_name);

                // Add QEMU configurations
                if let Some(ref cpu) = self.qemu_config.cpu {
                    c.arg("-cpu").arg(cpu);
                }
                if let Some(ref sysroot) = self.qemu_config.sysroot {
                    c.arg("-L").arg(sysroot);
                }
                if let Some(port) = self.qemu_config.gdb_port {
                    c.arg("-g").arg(port.to_string());
                }
                for (k, v) in &self.qemu_config.env_vars {
                    c.arg("-E").arg(format!("{}={}", k, v));
                }
                for extra in &self.qemu_config.extra_qemu_args {
                    c.arg(extra);
                }

                c.arg(exe_path);
                c.args(&self.program_args);
                c
            }
        };

        let output = cmd.output()?;
        let duration = start.elapsed();
        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        Ok(ExecutionResult {
            exit_code,
            stdout,
            stderr,
            status: output.status,
            duration,
            engine,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qemu_config_builder() {
        let config = QemuConfig::new()
            .with_cpu("cortex-a57")
            .with_sysroot("/usr/arm-linux-gnueabi")
            .with_gdb(1234)
            .with_arg("-singlestep")
            .with_env("VALE_TEST", "1");

        assert_eq!(config.cpu.as_deref(), Some("cortex-a57"));
        assert_eq!(
            config.sysroot,
            Some(PathBuf::from("/usr/arm-linux-gnueabi"))
        );
        assert_eq!(config.gdb_port, Some(1234));
        assert!(config.extra_qemu_args.contains(&"-singlestep".to_string()));
        assert_eq!(
            config.env_vars.get("VALE_TEST").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn test_target_runner_from_asm_native_and_qemu() {
        let target = TargetDescriptor::aarch64_linux();
        let asm = r#"
    .global main
    .type main, %function
main:
    mov w0, #42
    ret
    .size main, .-main
    .section .note.GNU-stack,"",@progbits
"#;

        let runner =
            TargetTestRunner::new("test_runner_simple", target).from_asm(asm);

        // Native run
        let native_res = runner.run_native().expect("Failed native run");
        native_res.assert_exit_code(42);

        // QEMU run
        let qemu_res = runner.run_qemu().expect("Failed QEMU run");
        qemu_res.assert_exit_code(42);

        // Differential assertion
        runner.run_differential().expect("Failed differential run");
    }

    #[test]
    fn test_target_runner_x86_qemu() {
        let target = TargetDescriptor::x86_64_linux();
        let asm = r#"
    .globl _start
_start:
    mov $60, %rax
    mov $42, %rdi
    syscall
"#;
        let runner = TargetTestRunner::new("test_runner_x86", target).from_asm(asm);
        let qemu_res = runner.run_qemu().expect("Failed x86 QEMU run");
        qemu_res.assert_exit_code(42);
    }

    #[test]
    fn test_target_runner_riscv_qemu() {
        let target = TargetDescriptor::riscv64_linux();
        let asm = r#"
    .globl _start
_start:
    li a7, 93
    li a0, 42
    ecall
"#;
        let runner =
            TargetTestRunner::new("test_runner_riscv", target).from_asm(asm);
        let qemu_res = runner.run_qemu().expect("Failed RISC-V QEMU run");
        qemu_res.assert_exit_code(42);
    }
}
