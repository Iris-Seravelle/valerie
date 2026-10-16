pub mod audit;
pub mod bench;
pub mod bundle;
pub mod diff;
pub mod driver;
pub mod header;
pub mod terminal;
pub mod testing;
pub mod vale_bridge;

pub use audit::{AuditFinding, AuditReport, AuditSeverity, audit_procedures};
pub use bench::{BenchmarkResult, run_benchmark};
pub use bundle::{BundleArtifacts, bundle_distribution};
pub use diff::{DiffEntry, compare_emitted_vs_disassembly, disassemble_object};
pub use driver::{
    BackendMode, DriverError, assemble_string, assemble_string_with_backend,
    link_executable, run_executable_with_qemu, run_target_executable, strip_binary,
};
pub use header::{
    HeaderConfig, generate_c_header, generate_c_header_with_config,
    infer_procedure_params,
};
pub use terminal::{Status, Style, colors_enabled};
pub use testing::{ExecutionEngine, ExecutionResult, QemuConfig, TargetTestRunner};
pub use vale_bridge::{
    ValeProcedure, compile_and_extract_vaf, extract_procedures_from_fst,
    verify_fst_with_fstar,
};
