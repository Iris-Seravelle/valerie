//! Binary disassembly parity diff engine for Valerie.
//!
//! Compiles assembly to an object file and compares emitted instructions against
//! disassembler output (`llvm-objdump` or `objdump`) to guarantee zero compiler mutations.
//! Supports:
//! - Multi-architecture canonicalization (AArch64, Arm32, x86_64, RISC-V 64)
//! - Instruction mnemonic, register, and numeric immediate normalization
//! - Machine byte extraction (`show_bytes`)
//! - Differential compiler comparison (`--against clang` / `--against gcc`)

use crate::driver::DriverError;
use crate::vale_bridge::ValeProcedure;
use std::path::Path;
use std::process::Command;
use valerie_target::{Architecture, TargetDescriptor};

#[derive(Debug, Clone)]
pub struct DiffInstructionMatch {
    pub emitted: String,
    pub disassembled: String,
    pub raw_bytes: String,
    pub is_exact: bool,
    pub alias_note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DiffEntry {
    pub proc_name: String,
    pub emitted_count: usize,
    pub disassembled_count: usize,
    pub matches: Vec<DiffInstructionMatch>,
    pub generated_epilogue: Vec<String>,
    pub is_parity_valid: bool,
    pub compiler_diff: Option<CompilerDiffComparison>,
}

#[derive(Debug, Clone)]
pub struct CompilerDiffComparison {
    pub compiler_name: String,
    pub valerie_instruction_count: usize,
    pub compiler_instruction_count: usize,
    pub compiler_instructions: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DisassembledInsn {
    pub address: u64,
    pub raw_bytes: String,
    pub mnemonic: String,
    pub operands: String,
    pub full_text: String,
}

/// Disassembles an object file with `llvm-objdump` or `objdump`.
pub fn disassemble_object_lines(obj_path: &Path) -> Result<String, DriverError> {
    let mut cmd = if which_exists("llvm-objdump") {
        Command::new("llvm-objdump")
    } else {
        Command::new("objdump")
    };

    cmd.arg("-d").arg(obj_path);
    let out = cmd.output()?;
    if !out.status.success() {
        return Err(DriverError::AssemblerFailed {
            status: out.status,
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        });
    }

    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Backward compatibility shim.
pub fn disassemble_object(obj_path: &Path) -> Result<String, DriverError> {
    disassemble_object_lines(obj_path)
}

fn which_exists(binary: &str) -> bool {
    Command::new("which")
        .arg(binary)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Parses disassembly lines of a procedure into structured `DisassembledInsn` items.
pub fn parse_procedure_disasm(
    disasm_text: &str,
    proc_name: &str,
) -> Vec<DisassembledInsn> {
    let mut insns = Vec::new();
    let target_sym = format!("<{}>:", proc_name);
    let mut capturing = false;

    for line in disasm_text.lines() {
        if line.contains(&target_sym) {
            capturing = true;
            continue;
        }
        if capturing {
            if line.trim().is_empty() || (line.contains('<') && line.ends_with(">:"))
            {
                break;
            }
            let clean = line.trim();
            // Typical format:
            // 0: 48 89 d8      movq %rbx, %rax
            // 4: 02a50513      addi a0, a0, 0x2a
            if let Some(colon_pos) = clean.find(':') {
                let addr_str = clean[..colon_pos].trim();
                let addr = u64::from_str_radix(addr_str, 16).unwrap_or(0);
                let rest = clean[colon_pos + 1..].trim();

                // rest has hex bytes followed by tab/whitespace then mnemonic + operands
                // Split by tabs or double spaces
                let parts: Vec<&str> = rest.split('\t').collect();
                let (bytes_str, insn_text) = if parts.len() >= 2 {
                    (parts[0].trim(), parts[1..].join(" ").trim().to_string())
                } else {
                    // Try splitting by 2 or more spaces
                    let tokens: Vec<&str> = rest.split_whitespace().collect();
                    if tokens.is_empty() {
                        continue;
                    }
                    // Find first non-hex token
                    let mut hex_end = 0;
                    for (i, tok) in tokens.iter().enumerate() {
                        if tok.chars().all(|c| c.is_ascii_hexdigit())
                            && tok.len() <= 8
                        {
                            hex_end = i + 1;
                        } else {
                            break;
                        }
                    }
                    if hex_end > 0 && hex_end < tokens.len() {
                        (
                            tokens[..hex_end].join(" ").leak() as &str,
                            tokens[hex_end..].join(" "),
                        )
                    } else {
                        ("", rest.to_string())
                    }
                };

                let mut insn_tokens = insn_text.split_whitespace();
                let mnemonic = insn_tokens.next().unwrap_or("").to_string();
                let operands = insn_tokens.collect::<Vec<_>>().join(" ");

                insns.push(DisassembledInsn {
                    address: addr,
                    raw_bytes: bytes_str.to_string(),
                    mnemonic,
                    operands,
                    full_text: insn_text,
                });
            }
        }
    }

    insns
}

/// Normalizes an operand string for cross-architecture parity comparison:
/// - Strips register prefixes (`%`, `x`, `w`, `$`, `#`)
/// - Canonicalizes registers (`rax` == `%rax`, `x0` == `zero`, `x1` == `ra`, `x2` == `sp`)
/// - Normalizes hex and decimal integer values (`0x10` == `16`)
pub fn normalize_tokens(s: &str, arch: Architecture) -> Vec<String> {
    s.replace([',', '(', ')'], " ")
        .split_whitespace()
        .map(|t| {
            let lower = t.to_lowercase();
            let trimmed = lower.trim_start_matches(['$', '#', '%']);

            // Parse hex vs decimal immediates
            if let Some(hex_part) = trimmed.strip_prefix("0x") {
                if let Ok(val) = i64::from_str_radix(hex_part, 16) {
                    return val.to_string();
                }
            } else if let Ok(val) = trimmed.parse::<i64>() {
                return val.to_string();
            }

            // Architecture-specific register aliases
            match arch {
                Architecture::Riscv64 => match trimmed {
                    "zero" | "x0" => "x0".to_string(),
                    "ra" | "x1" => "x1".to_string(),
                    "sp" | "x2" => "x2".to_string(),
                    "gp" | "x3" => "x3".to_string(),
                    "tp" | "x4" => "x4".to_string(),
                    "t0" | "x5" => "x5".to_string(),
                    "t1" | "x6" => "x6".to_string(),
                    "t2" | "x7" => "x7".to_string(),
                    "s0" | "fp" | "x8" => "x8".to_string(),
                    "s1" | "x9" => "x9".to_string(),
                    "a0" | "x10" => "x10".to_string(),
                    "a1" | "x11" => "x11".to_string(),
                    "a2" | "x12" => "x12".to_string(),
                    "a3" | "x13" => "x13".to_string(),
                    "a4" | "x14" => "x14".to_string(),
                    "a5" | "x15" => "x15".to_string(),
                    "a6" | "x16" => "x16".to_string(),
                    "a7" | "x17" => "x17".to_string(),
                    "s2" | "x18" => "x18".to_string(),
                    "s3" | "x19" => "x19".to_string(),
                    "s4" | "x20" => "x20".to_string(),
                    "s5" | "x21" => "x21".to_string(),
                    "s6" | "x22" => "x22".to_string(),
                    "s7" | "x23" => "x23".to_string(),
                    "s8" | "x24" => "x24".to_string(),
                    "s9" | "x25" => "x25".to_string(),
                    "s10" | "x26" => "x26".to_string(),
                    "s11" | "x27" => "x27".to_string(),
                    "t3" | "x28" => "x28".to_string(),
                    "t4" | "x29" => "x29".to_string(),
                    "t5" | "x30" => "x30".to_string(),
                    "t6" | "x31" => "x31".to_string(),
                    other => other.to_string(),
                },
                _ => trimmed.to_string(),
            }
        })
        .collect()
}

/// Checks comprehensive architectural equivalence between emitted and disassembled instructions.
pub fn check_instruction_equivalence(
    emitted_full: &str,
    disassembled_insn: &DisassembledInsn,
    arch: Architecture,
) -> (bool, Option<String>) {
    let em_trimmed = emitted_full.trim();
    let dis_trimmed = disassembled_insn.full_text.trim();

    let em_tokens: Vec<&str> = em_trimmed.split_whitespace().collect();
    let em_mnemonic = em_tokens.first().copied().unwrap_or("").to_lowercase();
    let dis_mnemonic = disassembled_insn.mnemonic.to_lowercase();

    // 1. Mnemonic equivalence / aliases
    let (mnemonic_matches, alias_note) = match arch {
        Architecture::AArch64 => match (em_mnemonic.as_str(), dis_mnemonic.as_str())
        {
            (a, b) if a == b => (true, None),
            ("veor", "eor") => (true, Some("canonical SIMD XOR".to_string())),
            ("veor3", "eor") => (true, Some("3-reg SIMD XOR".to_string())),
            ("vext" | "vext8", "ext") => {
                (true, Some("canonical vector EXT".to_string()))
            }
            ("pmull" | "pmull2", "pmull" | "pmull2") => (true, None),
            ("vadd", "add") => (true, Some("canonical SIMD ADD".to_string())),
            ("vsub", "sub") => (true, Some("canonical SIMD SUB".to_string())),
            ("vand", "and") => (true, Some("canonical SIMD AND".to_string())),
            ("vorr", "orr") => (true, Some("canonical SIMD ORR".to_string())),
            ("ins" | "umov" | "smov", "mov") => {
                (true, Some("vector MOV lane".to_string()))
            }
            _ => (false, None),
        },
        Architecture::X86_64 => {
            // Strip AT&T size suffixes ('q', 'l', 'w', 'b')
            let em_base = em_mnemonic
                .strip_suffix(['q', 'l', 'w', 'b'])
                .unwrap_or(&em_mnemonic);
            let dis_base = dis_mnemonic
                .strip_suffix(['q', 'l', 'w', 'b'])
                .unwrap_or(&dis_mnemonic);
            if em_mnemonic == dis_mnemonic || em_base == dis_base {
                (true, None)
            } else {
                (false, None)
            }
        }
        Architecture::Riscv64 => match (em_mnemonic.as_str(), dis_mnemonic.as_str())
        {
            (a, b) if a == b => (true, None),
            ("mv", "addi") => {
                (true, Some("canonical mv -> addi rd, rs, 0".to_string()))
            }
            ("nop", "addi") => {
                (true, Some("canonical nop -> addi x0, x0, 0".to_string()))
            }
            ("ret", "jalr") => {
                (true, Some("canonical ret -> jalr x0, 0(x1)".to_string()))
            }
            _ => (false, None),
        },
        _ => (em_mnemonic == dis_mnemonic, None),
    };

    if !mnemonic_matches {
        return (false, None);
    }

    // 2. Operands and Immediates Check
    let em_norm = normalize_tokens(em_trimmed, arch);
    let dis_norm = normalize_tokens(dis_trimmed, arch);

    // Filter out mnemonic token itself from comparison
    let em_ops: Vec<_> = em_norm.iter().skip(1).collect();
    let dis_ops: Vec<_> = dis_norm.iter().skip(1).collect();

    // In x86 AT&T syntax, instructions match if all operand registers and numbers appear
    let operands_match = if em_ops.len() == dis_ops.len() {
        em_ops == dis_ops
    } else {
        // Handle implicit zero operands or compressed instructions
        em_ops.iter().all(|op| dis_ops.contains(op))
    };

    if operands_match {
        (true, alias_note)
    } else {
        (false, Some("operand variation".to_string()))
    }
}

/// Generates side-by-side comparison between emitted Vale MIR instructions and disassembled machine instructions.
pub fn compare_emitted_vs_disassembly(
    procs: &[ValeProcedure],
    obj_path: &Path,
    target: &TargetDescriptor,
) -> Result<Vec<DiffEntry>, DriverError> {
    let disasm = disassemble_object_lines(obj_path)?;
    let mut entries = Vec::new();

    for proc in procs {
        // Collect emitted instruction text strings
        let mut emitted_lines = Vec::new();
        for inst in &proc.instructions {
            let trimmed = inst.trim();
            if !trimmed.is_empty()
                && !trimmed.starts_with('.')
                && !trimmed.ends_with(':')
            {
                emitted_lines.push(trimmed.to_string());
            }
        }

        let dis_insns = parse_procedure_disasm(&disasm, &proc.name);
        let mut matches = Vec::new();
        let mut parity_valid = true;

        for (em, dis) in emitted_lines.iter().zip(dis_insns.iter()) {
            let (is_eq, alias_note) =
                check_instruction_equivalence(em, dis, target.architecture);
            if !is_eq {
                parity_valid = false;
            }
            matches.push(DiffInstructionMatch {
                emitted: em.clone(),
                disassembled: dis.full_text.clone(),
                raw_bytes: dis.raw_bytes.clone(),
                is_exact: is_eq && alias_note.is_none(),
                alias_note,
            });
        }

        // Epilogue detection (instructions appearing after emitted body)
        let mut generated_epilogue = Vec::new();
        if dis_insns.len() > emitted_lines.len() {
            for extra in &dis_insns[emitted_lines.len()..] {
                generated_epilogue.push(extra.full_text.clone());
            }
        }

        if emitted_lines.len() != dis_insns.len() {
            // If the only difference is standard ABI function return ('ret' / 'retq'), count as valid
            if dis_insns.len() == emitted_lines.len() + 1 {
                if let Some(last) = dis_insns.last() {
                    let m = last.mnemonic.to_lowercase();
                    if m == "ret" || m == "retq" {
                        // Return epilogue is acceptable
                    } else {
                        parity_valid = false;
                    }
                }
            } else {
                parity_valid = false;
            }
        }

        entries.push(DiffEntry {
            proc_name: proc.name.clone(),
            emitted_count: emitted_lines.len(),
            disassembled_count: dis_insns.len(),
            matches,
            generated_epilogue,
            is_parity_valid: parity_valid,
            compiler_diff: None,
        });
    }

    Ok(entries)
}

/// Diffs Valerie emitted assembly against optimizing compiler output (e.g. `clang -O3` or `gcc -O2`).
pub fn compare_against_compiler(
    asm_source: &str,
    compiler_name: &str,
    opt_level: &str,
    target: &TargetDescriptor,
    temp_dir: &Path,
) -> Result<CompilerDiffComparison, DriverError> {
    let src_path = temp_dir.join("valerie_emitted.s");
    let comp_obj_path = temp_dir.join("compiler_compiled.o");
    std::fs::write(&src_path, asm_source)?;

    let mut cmd = Command::new(compiler_name);
    let opt_flag = if opt_level.starts_with('-') {
        opt_level.to_string()
    } else {
        format!("-O{}", opt_level)
    };
    cmd.arg(&opt_flag)
        .arg("-c")
        .arg(&src_path)
        .arg("-o")
        .arg(&comp_obj_path);

    match target.architecture {
        Architecture::AArch64 => {
            cmd.arg("--target=aarch64-linux-gnu");
        }
        Architecture::Arm32 => {
            cmd.arg("--target=armv7-linux-gnueabihf");
        }
        Architecture::X86_64 => {
            cmd.arg("--target=x86_64-linux-gnu");
        }
        Architecture::Riscv64 => {
            cmd.arg("--target=riscv64-unknown-linux-gnu")
                .arg("-march=rv64gc_zbb1p0_zk1p0");
        }
    }

    let out = cmd.output()?;
    if !out.status.success() {
        return Err(DriverError::AssemblerFailed {
            status: out.status,
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        });
    }

    let comp_disasm = disassemble_object_lines(&comp_obj_path)?;
    let mut comp_insns = Vec::new();
    for line in comp_disasm.lines() {
        let clean = line.trim();
        if let Some(pos) = clean.find(':') {
            let addr_part = clean[..pos].trim();
            // Valid disassembly lines start with a hex address before the colon
            if !addr_part.is_empty()
                && addr_part.chars().all(|c| c.is_ascii_hexdigit())
            {
                let text = clean[pos + 1..].trim();
                comp_insns.push(text.to_string());
            }
        }
    }

    let valerie_count = asm_source
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty()
                && !t.starts_with('.')
                && !t.ends_with(':')
                && !t.starts_with('#')
        })
        .count();

    Ok(CompilerDiffComparison {
        compiler_name: compiler_name.to_string(),
        valerie_instruction_count: valerie_count,
        compiler_instruction_count: comp_insns.len(),
        compiler_instructions: comp_insns,
    })
}
