use clap::builder::styling::{AnsiColor, Effects, Styles};
pub use clap::{CommandFactory, FromArgMatches, Parser};
use clap::{Subcommand, ValueEnum};
use std::path::PathBuf;
use valerie_driver::BackendMode as DriverBackendMode;

fn cli_styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Green.on_default() | Effects::BOLD)
        .usage(AnsiColor::Green.on_default() | Effects::BOLD)
        .literal(AnsiColor::Cyan.on_default() | Effects::BOLD)
        .placeholder(AnsiColor::Cyan.on_default())
        .error(AnsiColor::Red.on_default() | Effects::BOLD)
        .valid(AnsiColor::Green.on_default())
        .invalid(AnsiColor::Yellow.on_default())
}

#[derive(Parser, Debug)]
#[command(
    name = "valerie",
    author = "Seuriin <seuriin@gmail.com>, Iris-Seravelle <iris.seravelle@gmail.com>",
    version = concat!(env!("CARGO_PKG_VERSION"), " - Seuriin & Iris-Seravelle (", env!("VALERIE_COMMIT_HASH"), ")"),
    about = "Compiler Driver, Assembly Emitter & Formal Verification Pipeline for Vale/F*",
    before_help = concat!("valerie ", env!("CARGO_PKG_VERSION"), " - Seuriin & Iris-Seravelle (", env!("VALERIE_COMMIT_HASH"), ")"),
    styles = cli_styles()
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Target architecture (e.g. aarch64-linux, arm64-macos, armv7-linux, x86_64-linux, x86_64-windows, riscv64-linux) [default: host platform]
    #[arg(long, global = true, help_heading = "Toolchain & Target Options")]
    pub target: Option<String>,

    /// Assembler backend mode: auto, native, llvm
    #[arg(
        long,
        global = true,
        default_value = "auto",
        help_heading = "Toolchain & Target Options"
    )]
    pub backend: CliBackendMode,

    /// Custom Vale compiler binary or directory
    #[arg(long, global = true, help_heading = "Toolchain & Target Options")]
    pub vale_bin: Option<PathBuf>,

    /// Enable cryptographic hardware extensions (ARM +crypto, x86 AES-NI/SHA-NI, RISC-V Zk)
    #[arg(long, global = true, help_heading = "Toolchain & Target Options")]
    pub crypto: bool,

    /// Enable constant-time hardware enforcement (ARMv8.4-A DIT / Intel DOITM)
    #[arg(long, global = true, help_heading = "Toolchain & Target Options")]
    pub dit: bool,

    /// Skip F* proof checking during build or test
    #[arg(long, global = true, help_heading = "Toolchain & Target Options")]
    pub skip_verify: bool,

    /// Number of parallel jobs across compilation, verification, and testing
    #[arg(
        short = 'j',
        long = "jobs",
        global = true,
        default_value_t = default_parallel_jobs(),
        help_heading = "Toolchain & Target Options"
    )]
    pub jobs: usize,

    /// Coloring: auto, always, never
    #[arg(
        long,
        global = true,
        value_name = "WHEN",
        default_value = "auto",
        help_heading = "Output Options"
    )]
    pub color: String,

    /// Do not print cargo-style log messages
    #[arg(short, long, global = true, help_heading = "Output Options")]
    pub quiet: bool,

    /// Enable verbose diagnostics
    #[arg(short, long, global = true, help_heading = "Output Options")]
    pub verbose: bool,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliBackendMode {
    Auto,
    Native,
    Llvm,
}

impl From<CliBackendMode> for DriverBackendMode {
    fn from(mode: CliBackendMode) -> Self {
        match mode {
            CliBackendMode::Auto => DriverBackendMode::Auto,
            CliBackendMode::Native => DriverBackendMode::Native,
            CliBackendMode::Llvm => DriverBackendMode::Llvm,
        }
    }
}

pub fn default_parallel_jobs() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    // --- Workspace Lifecycle Commands ---
    /// Compile the current workspace or source file
    ///
    /// Lowers Vale procedures to hardware assembly (.s), compiles extracted C
    /// bundle units in dist/, and packages the final target into an object file
    /// or workspace static library archive (lib<pkg>.a).
    #[command(alias = "b", next_help_heading = "Workspace Lifecycle Commands")]
    Build {
        /// Input file (.vaf or .s) or omit in workspace mode
        file: Option<PathBuf>,
        /// Destination output path (.o, .a, or binary)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Build workspace static library archive
        #[arg(long)]
        lib: bool,
        /// Strip debug symbols from the resulting binary
        #[arg(long)]
        strip: bool,
    },

    /// Analyze source and report errors without code emission
    ///
    /// Validates syntax, procedure interfaces, and AST definitions
    /// across workspace Vale files without writing code artifacts or linking.
    #[command(alias = "c", next_help_heading = "Workspace Lifecycle Commands")]
    Check {
        /// Input file (.vaf, .fst) or omit in workspace mode
        file: Option<PathBuf>,
    },

    /// Run execution test harnesses or workspace test suite
    ///
    /// Executes unit tests, NIST statistical randomness benchmarks,
    /// and verified cryptographic test vectors across C and Rust harnesses.
    #[command(
        alias = "t",
        alias = "test-vale",
        next_help_heading = "Workspace Lifecycle Commands"
    )]
    Test {
        /// Optional test filter
        filter: Option<String>,
        /// Do not capture test output
        #[arg(long)]
        nocapture: bool,
        /// Force execution under QEMU user-mode emulator (qemu-aarch64, qemu-arm, qemu-x86_64, qemu-riscv64)
        #[arg(long)]
        qemu: bool,
        /// Target CPU model for QEMU (e.g. cortex-a57, max, rv64)
        #[arg(long)]
        cpu: Option<String>,
        /// End-to-end mode: check, emit, and formally verify with F*/Pulse simultaneously
        #[arg(long)]
        e2e: bool,
    },

    /// Remove build artifacts and verification caches
    ///
    /// Purges intermediate object files (.o), compilation caches (.cache/),
    /// and generated temporary directory artifacts.
    #[command(next_help_heading = "Workspace Lifecycle Commands")]
    Clean,

    /// Create a new valerie workspace in an existing directory
    ///
    /// Initializes a valerie.toml manifest template configured for
    /// Vale hardware assembly lowering and KaRaMeL C bundle extraction.
    #[command(next_help_heading = "Workspace Lifecycle Commands")]
    Init {
        /// Workspace directory (default: current directory)
        path: Option<PathBuf>,
        /// Package name
        #[arg(long)]
        name: Option<String>,
    },

    // --- Code Generation & Extraction ---
    /// Lower Vale source to assembly, F*, Pulse, or IR
    ///
    /// Invokes the Vale compiler to lower verified cryptographic procedures
    /// into target machine assembly (.s), F* proof modules (.fst/.fsti),
    /// Pulse separation logic specifications, or raw Vale IR AST dumps.
    #[command(
        alias = "compile-vale",
        next_help_heading = "Code Generation & Extraction"
    )]
    Emit {
        /// Input file (.vaf, .fst, or .s) or omit in workspace mode
        file: Option<PathBuf>,
        /// Output assembly path (.s)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Output F* implementation (.fst)
        #[arg(long)]
        fst: Option<PathBuf>,
        /// Output F* interface (.fsti)
        #[arg(long)]
        fsti: Option<PathBuf>,
        /// Output Pulse module (.fst)
        #[arg(long)]
        pulse: Option<PathBuf>,
        /// Output Vale IR AST dump
        #[arg(long)]
        ir: Option<PathBuf>,
    },

    /// Extract verified modules to C via KaRaMeL
    ///
    /// Extracts verified F* and Pulse modules into optimized C source and header
    /// bundles in dist/, using configurations defined in valerie.toml.
    #[command(next_help_heading = "Code Generation & Extraction")]
    Extract,

    /// Generate C/C++ ABI header prototypes
    ///
    /// Inferred from Vale procedure signatures and target ABI conventions,
    /// emitting C prototypes with platform include guards and type mappings.
    #[command(next_help_heading = "Code Generation & Extraction")]
    Header {
        /// Input file (.vaf or .fst) or omit in workspace mode
        file: Option<PathBuf>,
        /// Output header path or directory
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Custom description for generated C headers
        #[arg(long)]
        desc: Option<String>,
    },

    /// Package compiled distribution bundle (.s, .o, .h, and .a)
    ///
    /// Compiles all source units and collects assembly, object, header,
    /// and static archive artifacts into a single distributable bundle folder.
    #[command(next_help_heading = "Code Generation & Extraction")]
    Bundle {
        /// Input file (.vaf or .fst)
        file: PathBuf,
        /// Output directory
        #[arg(short, long, default_value = "dist")]
        output: PathBuf,
    },

    // --- Verification & Cryptographic Auditing ---
    /// Formally verify proof obligations with F* and Z3
    ///
    /// Type-checks and discharges formal verification conditions across
    /// the workspace dependency DAG in parallel dependency waves.
    #[command(
        alias = "v",
        next_help_heading = "Verification & Cryptographic Auditing"
    )]
    Verify {
        /// Input specification (.fst, .vaf) or omit in workspace mode
        file: Option<PathBuf>,
        /// Verify Pulse separation logic modules only
        #[arg(long)]
        pulse: bool,
        /// Verify specification modules only
        #[arg(long)]
        spec: bool,
        /// Force re-verification of all modules bypassing the checked file cache
        #[arg(short, long)]
        force: bool,
    },

    /// Audit assembly for constant-time cryptographic safety
    ///
    /// Scans instructions for secret-dependent branch conditions, memory lookups,
    /// and checks Data Independent Timing (DIT) compliance on ARMv8.4+.
    #[command(next_help_heading = "Verification & Cryptographic Auditing")]
    Audit {
        /// Input file (.vaf or .fst)
        file: PathBuf,
        /// Require Data Independent Timing (DIT)
        #[arg(long)]
        require_dit: bool,
    },

    /// Verify binary disassembly parity against emitted AST
    ///
    /// Disassembles assembled machine code via llvm-objdump to prove zero
    /// compiler-induced mutations or instruction inflation occurred.
    #[command(next_help_heading = "Verification & Cryptographic Auditing")]
    Diff {
        /// Input file (.vaf or .fst)
        file: PathBuf,
        /// Show machine code byte encodings
        #[arg(long)]
        show_bytes: bool,
        /// Compare emitted assembly against a compiler (e.g. clang, gcc)
        #[arg(long)]
        against: Option<String>,
        /// Optimization level for comparison
        #[arg(long, default_value = "3")]
        opt_level: String,
        /// Format output as JSON
        #[arg(long)]
        json: bool,
    },

    // --- Execution, Benchmarking & Testing Tools ---
    /// Compile and run program (Native or QEMU)
    ///
    /// Builds the target binary and executes it directly on native hardware
    /// or under QEMU user-mode emulation for cross-architecture validation.
    #[command(alias = "r", next_help_heading = "Execution & Analysis Tools")]
    Run {
        /// Input file (.vaf or .s)
        file: PathBuf,
        /// Force execution under QEMU user-mode emulator (qemu-aarch64, qemu-arm, qemu-x86_64, qemu-riscv64)
        #[arg(long)]
        qemu: bool,
        /// Target CPU model for QEMU (e.g. cortex-a57, max, rv64)
        #[arg(long)]
        cpu: Option<String>,
    },

    /// Benchmark execution cycles via hardware counters
    ///
    /// Reads CNTVCT_EL0 / RDTSC hardware performance counters to measure
    /// cycle counts and execution throughput across warm and cold trials.
    #[command(next_help_heading = "Execution & Analysis Tools")]
    Bench {
        /// Input file (.vaf or .fst)
        file: PathBuf,
        /// Iteration count
        #[arg(long, default_value_t = 1000)]
        iters: usize,
    },

    /// Run differential property-based fuzzer
    ///
    /// Executes randomized execution runs across distinct target backends
    /// to detect discrepancies between Vale verified output and reference logic.
    #[command(next_help_heading = "Execution & Analysis Tools")]
    Fuzz {
        /// Fuzzing iterations
        #[arg(short = 'n', long, default_value_t = 1000)]
        iterations: usize,
    },

    /// Run Vale compiler directly with native flags
    ///
    /// Direct command-line passthrough invocation of vale.exe with custom flags.
    #[command(next_help_heading = "Low-Level Toolchain Access")]
    Vale {
        /// Input .vaf source file
        #[arg(short = 'i', long = "in")]
        input: Option<PathBuf>,
        /// Output F* (.fst) implementation file
        #[arg(short = 'o', long = "out")]
        out: Option<PathBuf>,
        /// Output F* (.fsti) interface file
        #[arg(long = "outi")]
        outi: Option<PathBuf>,
        /// Output Pulse separation logic (.fst) module
        #[arg(long)]
        pulse: Option<PathBuf>,
        /// Output Vale IR AST dump
        #[arg(long)]
        ir: Option<PathBuf>,
        /// Target architecture (aarch64, arm32, x64, riscv64)
        #[arg(long)]
        arch: Option<String>,
        /// Positional input .vaf file fallback
        file: Option<PathBuf>,
        /// Build and install vale-compiler into ~/.local/bin/
        #[arg(long)]
        install: bool,
    },

    /// Generate shell auto-completion scripts
    ///
    /// Emits shell completion definitions for bash, zsh, fish, powershell, or elvish.
    #[command(next_help_heading = "Low-Level Toolchain Access")]
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
        /// Automatically install completion script into user profile / completions directory
        #[arg(long)]
        install: bool,
    },
}
