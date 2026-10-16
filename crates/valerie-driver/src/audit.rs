//! Constant-time leakage audit engine for Valerie.
//!
//! Scans unoptimized machine instructions for secret-dependent timing channels:
//! 1. Non-constant-time arithmetic instructions (e.g. variable-latency integer division sdiv/udiv).
//! 2. Secret-dependent conditional branches (b.cond, cbz, cbnz, tbz, tbnz).
//! 3. Variable-latency floating-point comparisons and division.
//! 4. Secret-dependent address calculations and cache-timing side-channels (register-indexed table lookups).
//! 5. Data Independent Timing (DIT) state tracking (ARMv8.4-A+).

use crate::vale_bridge::ValeProcedure;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct AuditFinding {
    pub procedure: String,
    pub instruction_index: usize,
    pub opcode: String,
    pub severity: AuditSeverity,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DitStatus {
    Enabled,
    NotConfigured,
}

pub struct AuditReport {
    pub total_instructions: usize,
    pub findings: Vec<AuditFinding>,
    pub dit_status: DitStatus,
}

impl AuditReport {
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }

    pub fn error_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == AuditSeverity::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == AuditSeverity::Warning)
            .count()
    }

    pub fn has_errors(&self) -> bool {
        self.error_count() > 0
    }
}

/// Audits an array of Vale procedures for timing attack vulnerabilities.
pub fn audit_procedures(procs: &[ValeProcedure]) -> AuditReport {
    let mut findings = Vec::new();
    let mut total_instructions = 0;
    let mut proc_dit_map = std::collections::HashMap::new();

    for proc in procs {
        let mut proc_dit = false;
        for (idx, inst) in proc.instructions.iter().enumerate() {
            total_instructions += 1;
            let lower = inst.trim().to_lowercase();
            let mnemonic = lower.split_whitespace().next().unwrap_or("");

            // Track ARM DIT (Data Independent Timing) configuration per procedure
            if lower.contains("msr") && lower.contains("dit") {
                proc_dit = true;
            }

            // Variable-latency integer divide
            if mnemonic == "sdiv"
                || mnemonic == "udiv"
                || mnemonic == "div"
                || mnemonic == "idiv"
            {
                findings.push(AuditFinding {
                    procedure: proc.name.clone(),
                    instruction_index: idx,
                    opcode: mnemonic.to_string(),
                    severity: AuditSeverity::Error,
                    reason: "Variable-cycle division instruction. Vulnerable to timing side-channels.".to_string(),
                });
            }

            // Variable-latency FP divide
            if mnemonic == "fdiv" || mnemonic == "vfdiv" {
                findings.push(AuditFinding {
                    procedure: proc.name.clone(),
                    instruction_index: idx,
                    opcode: mnemonic.to_string(),
                    severity: AuditSeverity::Warning,
                    reason: "Floating-point division may incur data-dependent execution latency on certain microarchitectures.".to_string(),
                });
            }

            // Data-dependent branches inside cryptographic kernels
            if mnemonic.starts_with("b.")
                || mnemonic == "cbz"
                || mnemonic == "cbnz"
                || mnemonic == "tbz"
                || mnemonic == "tbnz"
            {
                findings.push(AuditFinding {
                    procedure: proc.name.clone(),
                    instruction_index: idx,
                    opcode: mnemonic.to_string(),
                    severity: AuditSeverity::Warning,
                    reason: "Conditional branch in cryptographic routine. Ensure branch condition is not secret-dependent.".to_string(),
                });
            }

            // Check register-indexed memory lookup: e.g. [x0, x1] or [r0, r1] (no immediate '#' offset)
            if lower.contains('[')
                && lower.contains(']')
                && lower.contains(',')
                && !lower.contains('#')
            {
                findings.push(AuditFinding {
                    procedure: proc.name.clone(),
                    instruction_index: idx,
                    opcode: mnemonic.to_string(),
                    severity: AuditSeverity::Error,
                    reason: format!(
                        "Register-indexed memory lookup (`{}`). Vulnerable to cache-timing and S-box collision attacks.",
                        inst.trim()
                    ),
                });
            }
        }
        proc_dit_map.insert(proc.name.clone(), proc_dit);
    }

    let all_procs_have_dit =
        !proc_dit_map.is_empty() && proc_dit_map.values().all(|&v| v);

    AuditReport {
        total_instructions,
        findings,
        dit_status: if all_procs_have_dit {
            DitStatus::Enabled
        } else {
            DitStatus::NotConfigured
        },
    }
}
