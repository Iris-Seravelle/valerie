//! Cargo-style terminal formatting and UI subsystem.

pub mod config;
pub mod diagnostics;
pub mod profile;
pub mod status;
pub mod style;

pub use config::{
    ColorChoice, colors_enabled, is_quiet, set_color_choice, set_quiet,
};
pub use diagnostics::{
    Diagnostic, DiagnosticEngine, DiagnosticLevel, DiagnosticParser, SourceSpan,
    format_tool_diagnostics,
};
pub use profile::{FinishedSummary, Profile};
pub use status::Status;
pub use style::Style;
