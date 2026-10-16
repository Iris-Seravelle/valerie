//! Bridge for Project Everest Vale / valerie-core toolchain.

use std::fmt;
use std::path::Path;
use std::process::Command;

#[derive(Debug)]
pub enum ValeBridgeError {
    Io(std::io::Error),
    ToolNotFound { tool: String, install_hint: String },
    ValeFailed { code: Option<i32>, stderr: String },
    FStarFailed { code: Option<i32>, stderr: String },
    ParseError { proc_name: String, reason: String },
    UnsupportedInstruction(String),
}

impl std::error::Error for ValeBridgeError {}

impl fmt::Display for ValeBridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValeBridgeError::Io(e) => {
                write!(f, "I/O error during execution: {}", e)
            }
            ValeBridgeError::ToolNotFound { tool, install_hint } => {
                write!(
                    f,
                    "Required tool '{}' not found in PATH or environment.\n  => {}\n",
                    tool, install_hint
                )
            }
            ValeBridgeError::ValeFailed { code: _, stderr } => {
                let formatted = crate::terminal::format_tool_diagnostics(stderr);
                write!(f, "Vale compilation failed:\n\n{}", formatted)
            }
            ValeBridgeError::FStarFailed { code: _, stderr } => {
                let formatted = crate::terminal::format_tool_diagnostics(stderr);
                write!(f, "F* verification failed:\n\n{}", formatted)
            }
            ValeBridgeError::ParseError { proc_name, reason } => {
                write!(
                    f,
                    "Failed to parse Vale procedure `{}`: {}",
                    proc_name, reason
                )
            }
            ValeBridgeError::UnsupportedInstruction(s) => {
                write!(f, "Unsupported instruction in Vale AST: `{}`", s)
            }
        }
    }
}

/// Checks whether the Project Everest Vale executable is installed and available.
pub fn is_vale_available(vale_bin: &Path) -> bool {
    Command::new(vale_bin)
        .arg("-in")
        .arg("/nonexistent_dummy")
        .output()
        .map(|out| {
            !out.stderr.is_empty()
                || !out.stdout.is_empty()
                || out.status.code().is_some()
        })
        .unwrap_or(false)
}

/// Checks whether the F* formal verification engine (`fstar.exe` or `fstar`) is available.
pub fn is_fstar_available() -> bool {
    Command::new("fstar.exe")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
        || Command::new("fstar")
            .arg("--version")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
}

impl From<std::io::Error> for ValeBridgeError {
    fn from(err: std::io::Error) -> Self {
        ValeBridgeError::Io(err)
    }
}

/// Represents a procedure extracted from Vale output.
#[derive(Debug, Clone, Default)]
pub struct ValeProcedure {
    pub name: String,
    pub instructions: Vec<String>,
}

/// Options passed to the Vale compiler.
#[derive(Debug, Clone)]
pub struct ValeCompileOptions<'a> {
    pub input_vaf: &'a Path,
    pub output_fst: Option<&'a Path>,
    pub output_fsti: Option<&'a Path>,
    pub output_pulse: Option<&'a Path>,
    pub output_ir: Option<&'a Path>,
    pub output_asm: Option<&'a Path>,
    pub arch: Option<&'a str>,
    pub fstar_text: bool,
}

impl<'a> ValeCompileOptions<'a> {
    pub fn new(input_vaf: &'a Path) -> Self {
        Self {
            input_vaf,
            output_fst: None,
            output_fsti: None,
            output_pulse: None,
            output_ir: None,
            output_asm: None,
            arch: None,
            fstar_text: false,
        }
    }
}

/// Invokes the Vale compiler CLI with fine-grained output control.
pub fn invoke_vale_compiler(
    vale_bin: &Path,
    opts: &ValeCompileOptions,
) -> Result<std::process::Output, ValeBridgeError> {
    if !is_vale_available(vale_bin) {
        return Err(ValeBridgeError::ToolNotFound {
            tool: vale_bin.display().to_string(),
            install_hint: "Vale compiler not found. Please ensure vale-compiler is in PATH or configure via --vale-bin.".to_string(),
        });
    }

    let mut cmd = Command::new(vale_bin);
    if opts.fstar_text {
        cmd.arg("-fstarText");
    }
    cmd.arg("-in").arg(opts.input_vaf);

    if let Some(fst) = opts.output_fst {
        cmd.arg("-out").arg(fst);
    }
    if let Some(fsti) = opts.output_fsti {
        cmd.arg("-outi").arg(fsti);
    }
    if let Some(pulse) = opts.output_pulse {
        cmd.arg("-pulse").arg(pulse);
    }
    if let Some(ir) = opts.output_ir {
        cmd.arg("-ir").arg(ir);
    }
    if let Some(asm) = opts.output_asm {
        cmd.arg("-asm").arg(asm);
    }
    if let Some(arch) = opts.arch {
        cmd.arg("-arch").arg(arch);
    }

    let output = match cmd.output() {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ValeBridgeError::ToolNotFound {
                tool: vale_bin.display().to_string(),
                install_hint: "Vale compiler not found. Please ensure vale-compiler is in PATH.".to_string(),
            });
        }
        Err(e) => return Err(ValeBridgeError::Io(e)),
    };

    if !output.status.success() {
        return Err(ValeBridgeError::ValeFailed {
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }

    Ok(output)
}

pub fn compile_and_extract_vaf(
    vale_bin: &Path,
    input_vaf: &Path,
    output_fst: &Path,
    output_fsti: &Path,
) -> Result<Vec<ValeProcedure>, ValeBridgeError> {
    let mut opts = ValeCompileOptions::new(input_vaf);
    opts.output_fst = Some(output_fst);
    opts.output_fsti = Some(output_fsti);
    invoke_vale_compiler(vale_bin, &opts)?;

    let fst_content = std::fs::read_to_string(output_fst).unwrap_or_default();
    extract_procedures_from_fst(&fst_content)
}

/// Discharges proof obligations using F* on an interface/implementation pair.
pub fn verify_fst_with_fstar(
    output_fsti: &Path,
    output_fst: &Path,
    cache_dir: &Path,
) -> Result<(), ValeBridgeError> {
    let fstar_bin = if Command::new("fstar.exe")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        "fstar.exe"
    } else {
        "fstar"
    };

    let res_i = Command::new(fstar_bin)
        .arg("--warn_error")
        .arg("-271-272-241")
        .arg("--cache_checked_modules")
        .arg("--cache_dir")
        .arg(cache_dir)
        .arg(output_fsti)
        .output()?;

    if !res_i.status.success() {
        return Err(ValeBridgeError::FStarFailed {
            code: res_i.status.code(),
            stderr: String::from_utf8_lossy(&res_i.stderr).to_string(),
        });
    }

    let res_m = Command::new(fstar_bin)
        .arg("--warn_error")
        .arg("-271-272-241")
        .arg("--cache_checked_modules")
        .arg("--cache_dir")
        .arg(cache_dir)
        .arg(output_fst)
        .output()?;

    if !res_m.status.success() {
        return Err(ValeBridgeError::FStarFailed {
            code: res_m.status.code(),
            stderr: String::from_utf8_lossy(&res_m.stderr).to_string(),
        });
    }

    Ok(())
}

/// Parses procedure definitions from generated Vale F* text.
pub fn extract_procedures_from_fst(
    fst_content: &str,
) -> Result<Vec<ValeProcedure>, ValeBridgeError> {
    let mut procedures = Vec::new();
    let mut current_idx = 0;
    while let Some(start_pos) = fst_content[current_idx..].find("let va_code_") {
        let abs_start = current_idx + start_pos;
        let line_rest = &fst_content[abs_start..];
        let name_start = "let va_code_".len();
        let name_end = line_rest[name_start..]
            .find(|c: char| c.is_whitespace() || c == '(' || c == ':')
            .unwrap_or(0);
        let proc_name = line_rest[name_start..name_start + name_end]
            .trim()
            .to_string();
        procedures.push(ValeProcedure {
            name: proc_name,
            instructions: Vec::new(),
        });
        current_idx = abs_start + name_start + name_end.max(1);
    }
    Ok(procedures)
}

/// Parses procedures and instructions from emitted assembly text.
pub fn extract_procedures_from_asm(asm_text: &str) -> Vec<ValeProcedure> {
    let mut procs = Vec::new();
    let mut current_proc: Option<ValeProcedure> = None;

    for line in asm_text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(".global ") || trimmed.starts_with(".globl ") {
            let name = trimmed.split_whitespace().nth(1).unwrap_or("").to_string();
            if let Some(p) = current_proc.take() {
                if !p.instructions.is_empty() {
                    procs.push(p);
                }
            }
            current_proc = Some(ValeProcedure {
                name,
                instructions: Vec::new(),
            });
        } else if let Some(ref mut p) = current_proc {
            if trimmed.ends_with(':') && !trimmed.starts_with('.') {
                let name = trimmed.trim_end_matches(':');
                p.name = name.to_string();
            } else if !trimmed.is_empty()
                && !trimmed.starts_with('.')
                && !trimmed.starts_with("//")
                && !trimmed.starts_with('#')
            {
                p.instructions.push(trimmed.to_string());
            }
        }
    }
    if let Some(p) = current_proc {
        if !p.instructions.is_empty() {
            procs.push(p);
        }
    }
    procs
}
