//! Cargo-style status badge printer and standard diagnostics.

use super::config::{colors_enabled, is_quiet};
use super::profile::{FinishedSummary, Profile};
use super::style::Style;
use std::fmt;
use std::time::Duration;

/// Cargo-style status badge printer.
/// Emits right-aligned 12-character status keywords:
/// `   Compiling `, `    Finished `, `     Running `, `       Error `
pub struct Status;

impl Status {
    /// Prints a bold green status: `   Compiling foo`
    pub fn compiling(message: impl fmt::Display) {
        Self::print_normal(Style::GREEN_BOLD, "Compiling", message);
    }

    /// Prints a bold cyan status: `    Checking foo`
    pub fn checking(message: impl fmt::Display) {
        Self::print_normal(Style::CYAN_BOLD, "Checking", message);
    }

    /// Prints a bold cyan status: `   Verifying foo` (for formal verification)
    pub fn verifying(message: impl fmt::Display) {
        Self::print_normal(Style::CYAN_BOLD, "Verifying", message);
    }

    /// Prints a bold cyan status: `  Assembling foo.s`
    pub fn assembling(message: impl fmt::Display) {
        Self::print_normal(Style::CYAN_BOLD, "Assembling", message);
    }

    /// Prints a bold cyan status: `     Linking foo`
    pub fn linking(message: impl fmt::Display) {
        Self::print_normal(Style::CYAN_BOLD, "Linking", message);
    }

    /// Prints a bold green status: `     Running foo`
    pub fn running(message: impl fmt::Display) {
        Self::print_normal(Style::GREEN_BOLD, "Running", message);
    }

    /// Prints a bold blue status: `     Testing foo`
    pub fn testing(message: impl fmt::Display) {
        Self::print_normal(Style::BLUE_BOLD, "Testing", message);
    }

    /// Prints a bold magenta status: `    Emulated foo via qemu`
    pub fn emulated(message: impl fmt::Display) {
        Self::print_normal(Style::MAGENTA_BOLD, "Emulated", message);
    }

    /// Prints a bold cyan status: `  Diagnostic key: value`
    pub fn diagnostic(category: &str, message: impl fmt::Display) {
        Self::print_normal(Style::CYAN_BOLD, category, message);
    }

    /// Prints a bold green status: `    Finished dev [unoptimized + debuginfo] target(s) in 0.08s`
    pub fn finished(message: impl fmt::Display) {
        Self::print_normal(Style::GREEN_BOLD, "Finished", message);
    }

    /// Prints a canonical finished summary matching Cargo standard output.
    pub fn finished_summary(
        profile: Profile,
        duration: Duration,
        extra_features: &[&str],
    ) {
        let mut summary = FinishedSummary::new(profile, duration);
        for f in extra_features {
            summary = summary.with_feature(*f);
        }
        Self::finished(summary.to_status_message());
    }

    /// Prints a bold yellow status: `     warning: foo`
    pub fn warning(message: impl fmt::Display) {
        if colors_enabled() {
            eprintln!(
                "{}{:>12}{} {}",
                Style::YELLOW_BOLD,
                "warning:",
                Style::RESET,
                message
            );
        } else {
            eprintln!("{:>12} {}", "warning:", message);
        }
    }

    /// Prints a bold red status: `       error: foo`
    pub fn error(message: impl fmt::Display) {
        if colors_enabled() {
            eprintln!(
                "{}{:>12}{} {}",
                Style::RED_BOLD,
                "error:",
                Style::RESET,
                message
            );
        } else {
            eprintln!("{:>12} {}", "error:", message);
        }
    }

    /// Prints an indented dim detail line for verbose mode.
    pub fn detail(message: impl fmt::Display) {
        if is_quiet() {
            return;
        }
        if colors_enabled() {
            eprintln!("               {}{}{}", Style::DIM, message, Style::RESET);
        } else {
            eprintln!("               {}", message);
        }
    }

    /// Prints a right-aligned 12-char colored status header, suppressed in quiet mode.
    pub fn print_normal(color: &str, status: &str, message: impl fmt::Display) {
        if is_quiet() {
            return;
        }
        Self::print(color, status, message);
    }

    /// Formats and prints a right-aligned 12-char colored status header unconditionally.
    pub fn print(color: &str, status: &str, message: impl fmt::Display) {
        if colors_enabled() {
            let padded = format!("{:>12}", status);
            eprintln!("{}{}{} {}", color, padded, Style::RESET, message);
        } else {
            eprintln!("{:>12} {}", status, message);
        }
    }

    /// Emits `running N tests` header matching Cargo
    pub fn test_suite_header(count: usize) {
        if is_quiet() {
            return;
        }
        let noun = if count == 1 { "test" } else { "tests" };
        eprintln!("\nrunning {} {}", count, noun);
    }

    /// Emits `test <name> ... ok` or `test <name> ... FAILED`
    pub fn test_case(name: &str, passed: bool, duration: Option<Duration>) {
        if is_quiet() && passed {
            return;
        }
        let status_str = if passed {
            if colors_enabled() {
                format!("{}{}{}", Style::GREEN_BOLD, "ok", Style::RESET)
            } else {
                "ok".to_string()
            }
        } else {
            if colors_enabled() {
                format!("{}{}{}", Style::RED_BOLD, "FAILED", Style::RESET)
            } else {
                "FAILED".to_string()
            }
        };

        let timing = duration
            .map(|d| format!(" ({:.3}s)", d.as_secs_f64()))
            .unwrap_or_default();

        eprintln!("test {:<40} ... {}{}", name, status_str, timing);
    }

    /// Emits final `test result: ok. X passed; Y failed; finished in Zs`
    pub fn test_suite_result(passed: usize, failed: usize, duration: Duration) {
        let result_str = if failed == 0 {
            if colors_enabled() {
                format!("{}{}{}", Style::GREEN_BOLD, "ok", Style::RESET)
            } else {
                "ok".to_string()
            }
        } else {
            if colors_enabled() {
                format!("{}{}{}", Style::RED_BOLD, "FAILED", Style::RESET)
            } else {
                "FAILED".to_string()
            }
        };

        eprintln!(
            "\ntest result: {}. {} passed; {} failed; 0 ignored; finished in {:.2}s\n",
            result_str,
            passed,
            failed,
            duration.as_secs_f64()
        );
    }
}
