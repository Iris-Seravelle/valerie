//! Terminal configuration and color decision logic.

use std::io::{self, IsTerminal};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Color choice configuration, matching Cargo's `--color` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

impl std::str::FromStr for ColorChoice {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(ColorChoice::Auto),
            "always" => Ok(ColorChoice::Always),
            "never" => Ok(ColorChoice::Never),
            other => Err(format!(
                "invalid color choice `{}`; expected one of: auto, always, never",
                other
            )),
        }
    }
}

static COLOR_MODE: AtomicU8 = AtomicU8::new(0); // 0 = Auto, 1 = Always, 2 = Never
static QUIET_MODE: AtomicBool = AtomicBool::new(false);

/// Sets the global color choice for the driver.
pub fn set_color_choice(choice: ColorChoice) {
    let val = match choice {
        ColorChoice::Auto => 0,
        ColorChoice::Always => 1,
        ColorChoice::Never => 2,
    };
    COLOR_MODE.store(val, Ordering::SeqCst);
}

/// Sets whether quiet mode is enabled.
pub fn set_quiet(quiet: bool) {
    QUIET_MODE.store(quiet, Ordering::SeqCst);
}

/// Returns whether quiet mode is enabled.
pub fn is_quiet() -> bool {
    QUIET_MODE.load(Ordering::Relaxed)
}

/// Checks if colors should be enabled (TTY connected and NO_COLOR not set, or forced by Always).
pub fn colors_enabled() -> bool {
    match COLOR_MODE.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => {
            if std::env::var_os("NO_COLOR").is_some() {
                return false;
            }
            if let Ok(cargo_color) = std::env::var("CARGO_TERM_COLOR") {
                if cargo_color == "never" {
                    return false;
                } else if cargo_color == "always" {
                    return true;
                }
            }
            if std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false) {
                return false;
            }
            io::stderr().is_terminal() || io::stdout().is_terminal()
        }
    }
}
