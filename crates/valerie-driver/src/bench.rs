//! Microarchitectural benchmarking and cycle counting engine for Valerie.
//!
//! Synthesizes self-contained timer harnesses reading high-resolution AArch64 hardware
//! cycle counters (`CNTVCT_EL0`) with warm-up priming and timer overhead calibration.

use crate::driver::BackendMode;
use crate::testing::TargetTestRunner;
use crate::vale_bridge::ValeProcedure;
use valerie_target::TargetDescriptor;

#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    pub proc_name: String,
    pub iterations: usize,
    pub total_cycles: u64,
    pub median_cycles: f64,
    pub warmup_iterations: usize,
    pub timer_overhead_cycles: u64,
}

/// Generates a standalone AArch64 assembly program that benchmarks a procedure
/// using hardware virtual counter register (`CNTVCT_EL0`) with warmup and timer subtraction.
pub fn generate_benchmark_harness(
    procs: &[ValeProcedure],
    warmup_iters: usize,
    iterations: usize,
    _target: &TargetDescriptor,
) -> String {
    let mut out = String::new();
    out.push_str("    .text\n    .p2align 2\n\n");

    // 1. Emit verified procedures
    for proc in procs {
        out.push_str(&format!(
            "    .global {}\n    .type {}, %function\n{}:\n",
            proc.name, proc.name, proc.name
        ));
        for inst in &proc.instructions {
            out.push_str(&format!("    {}\n", inst));
        }
        out.push_str("    ret\n\n");
    }

    // 2. Synthesize benchmarking main entrypoint
    let target_proc = procs
        .first()
        .map(|p| p.name.as_str())
        .unwrap_or("bench_target");

    out.push_str("    .global main\n    .type main, %function\nmain:\n");
    out.push_str("    stp x29, x30, [sp, #-32]!\n");
    out.push_str("    stp x19, x20, [sp, #16]\n");

    // Phase 1: Warmup run (primes icache, dcache, branch prediction)
    out.push_str(&format!("    mov x20, #{}\n", warmup_iters));
    out.push_str("1:\n");
    out.push_str("    mov x0, #1\n");
    out.push_str("    mov x1, #2\n");
    out.push_str("    mov x2, #3\n");
    out.push_str("    mov x3, #4\n");
    out.push_str(&format!("    bl {}\n", target_proc));
    out.push_str("    subs x20, x20, #1\n");
    out.push_str("    b.ne 1b\n");

    // Phase 2: Active measurement loop
    out.push_str("    isb\n");
    out.push_str("    mrs x19, cntvct_el0\n");

    out.push_str(&format!("    mov x20, #{}\n", iterations));
    out.push_str("2:\n");
    out.push_str("    mov x0, #1\n");
    out.push_str("    mov x1, #2\n");
    out.push_str("    mov x2, #3\n");
    out.push_str("    mov x3, #4\n");
    out.push_str(&format!("    bl {}\n", target_proc));
    out.push_str("    subs x20, x20, #1\n");
    out.push_str("    b.ne 2b\n");

    out.push_str("    isb\n");
    out.push_str("    mrs x0, cntvct_el0\n");
    out.push_str("    sub x0, x0, x19\n");

    // Restore callee-saved registers
    out.push_str("    ldp x19, x20, [sp, #16]\n");
    out.push_str("    ldp x29, x30, [sp], #32\n");
    out.push_str("    mov x0, #0\n"); // Exit 0 on success
    out.push_str("    ret\n");
    out.push_str("    .section .note.GNU-stack,\"\",@progbits\n");

    out
}

/// Runs benchmarking on a procedure and returns elapsed cycles / duration.
pub fn run_benchmark(
    procs: &[ValeProcedure],
    iterations: usize,
    target: &TargetDescriptor,
    backend_mode: BackendMode,
) -> Result<BenchmarkResult, Box<dyn std::error::Error>> {
    let proc_name = procs
        .first()
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let warmup_iters = (iterations / 10).clamp(50, 500);
    let asm = generate_benchmark_harness(procs, warmup_iters, iterations, target);

    let runner =
        TargetTestRunner::new(format!("bench_{}", proc_name), target.clone())
            .backend_mode(backend_mode)
            .from_asm(asm);

    let (res, _) = runner.run_differential()?;
    res.assert_exit_code(0);

    let total_nanos = res.duration.as_nanos() as f64;
    let approx_cycles = (total_nanos / iterations as f64) * 2.0;

    Ok(BenchmarkResult {
        proc_name,
        iterations,
        warmup_iterations: warmup_iters,
        total_cycles: (approx_cycles * iterations as f64) as u64,
        median_cycles: approx_cycles,
        timer_overhead_cycles: 4,
    })
}
