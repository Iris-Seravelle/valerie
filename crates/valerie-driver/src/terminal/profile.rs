//! Cargo-style build and finish profile reporting.

use super::config::colors_enabled;
use std::fmt;
use std::time::Duration;

/// Canonical profile representation (`dev`, `release`, `test`, `bench`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Dev,
    Release,
    Test,
    Bench,
}

impl Profile {
    pub fn name(&self) -> &'static str {
        match self {
            Profile::Dev => "dev",
            Profile::Release => "release",
            Profile::Test => "test",
            Profile::Bench => "bench",
        }
    }

    pub fn default_description(&self) -> &'static str {
        match self {
            Profile::Dev => "unoptimized + debuginfo",
            Profile::Release => "optimized",
            Profile::Test => "unoptimized + debuginfo",
            Profile::Bench => "optimized",
        }
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` profile", self.name())
    }
}

/// Formatter for Cargo-exact `Finished` line:
/// `Finished dev [unoptimized + debuginfo] target(s) in 0.08s`
pub struct FinishedSummary {
    profile: Profile,
    extra_features: Vec<String>,
    duration: Duration,
    target_count: Option<usize>,
}

impl FinishedSummary {
    pub fn new(profile: Profile, duration: Duration) -> Self {
        Self {
            profile,
            extra_features: Vec::new(),
            duration,
            target_count: None,
        }
    }

    pub fn with_feature(mut self, feat: impl Into<String>) -> Self {
        self.extra_features.push(feat.into());
        self
    }

    pub fn target_count(mut self, count: usize) -> Self {
        self.target_count = Some(count);
        self
    }

    pub fn to_status_message(&self) -> String {
        let mut desc = self.profile.default_description().to_string();
        for feat in &self.extra_features {
            desc.push_str(", ");
            desc.push_str(feat);
        }

        let target_str = match self.target_count {
            Some(1) => "target in",
            _ => "target(s) in",
        };

        let secs = self.duration.as_secs_f64();
        let formatted_time = if secs < 60.0 {
            format!("{:.2}s", secs)
        } else {
            let mins = (secs / 60.0).floor() as u64;
            let rem = secs - (mins as f64 * 60.0);
            format!("{}m {:.2}s", mins, rem)
        };

        if colors_enabled() {
            format!(
                "{} [{}] {} {}",
                self.profile, desc, target_str, formatted_time
            )
        } else {
            format!(
                "{} [{}] {} {}",
                self.profile, desc, target_str, formatted_time
            )
        }
    }
}
