//! Manifest parser, configuration schema, and directory probe for `valerie.toml`.

pub mod resolver;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ManifestError {
    #[error("Manifest file not found: {0}")]
    NotFound(PathBuf),
    #[error("Failed to parse TOML manifest: {0}")]
    ParseError(#[from] toml::de::Error),
    #[error("I/O error reading manifest: {0}")]
    Io(#[from] std::io::Error),
    #[error("Toolchain binary resolution failed for '{tool}': {details}")]
    ToolchainNotFound { tool: String, details: String },
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Manifest {
    pub package: PackageConfig,
    #[serde(default)]
    pub toolchain: ToolchainConfig,
    #[serde(default)]
    pub target: HashMap<String, TargetConfig>,
    #[serde(default)]
    pub verification: Option<VerificationConfig>,
    #[serde(default)]
    pub assembly: Option<AssemblyConfig>,
    #[serde(default)]
    pub extraction: Option<ExtractionConfig>,
    #[serde(default)]
    pub distribution: Option<DistributionConfig>,
    #[serde(default)]
    pub bindings: BindingsConfig,
    #[serde(default)]
    pub tasks: HashMap<String, TaskDef>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct PackageConfig {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub edition: Option<String>,
    pub description: Option<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct ToolchainConfig {
    #[serde(default = "default_auto")]
    pub fstar: String,
    #[serde(default = "default_auto")]
    pub vale: String,
    #[serde(default = "default_auto")]
    pub karamel: String,
    #[serde(default = "default_auto")]
    pub clang: String,
    pub z3: Option<String>,
}

fn default_auto() -> String {
    "auto".to_string()
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct TargetConfig {
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub dit: bool,
    #[serde(default = "default_auto")]
    pub assembler: String,
    #[serde(default)]
    pub assembly: Option<bool>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct VerificationConfig {
    #[serde(rename = "cache-dir", default)]
    pub cache_dir: Option<PathBuf>,
    #[serde(rename = "hints-dir", default)]
    pub hints_dir: Option<PathBuf>,
    #[serde(rename = "warn-error", default)]
    pub warn_error: Option<String>,
    #[serde(rename = "already-cached", default)]
    pub already_cached: Vec<String>,
    #[serde(default)]
    pub include: Vec<PathBuf>,
    #[serde(default)]
    pub pulse: PulseConfig,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct PulseConfig {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct AssemblyConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub sources: Vec<PathBuf>,
    #[serde(rename = "emit-ast-dir", default)]
    pub emit_ast_dir: Option<PathBuf>,
    #[serde(rename = "output-dir", default)]
    pub output_dir: Option<PathBuf>,
    #[serde(default)]
    pub abi: Option<String>,
    #[serde(default)]
    pub aliases: std::collections::HashMap<String, Vec<String>>,
}

fn default_true() -> bool {
    true
}

impl Default for AssemblyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sources: Vec::new(),
            emit_ast_dir: None,
            output_dir: None,
            abi: None,
            aliases: std::collections::HashMap::new(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct ExtractionConfig {
    #[serde(default)]
    pub c: Option<CExtractionConfig>,
    #[serde(default)]
    pub ocaml: Option<OCamlExtractionConfig>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct OCamlExtractionConfig {
    #[serde(rename = "output-dir", default)]
    pub output_dir: Option<PathBuf>,
    #[serde(default)]
    pub modules: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct CExtractionConfig {
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(rename = "output-dir", default)]
    pub output_dir: Option<PathBuf>,
    #[serde(rename = "builtin-uint128", default)]
    pub builtin_uint128: bool,
    #[serde(rename = "minimal-headers", default)]
    pub minimal_headers: bool,
    #[serde(rename = "warn-error", default)]
    pub warn_error: Option<String>,
    #[serde(default)]
    pub cflags: Vec<String>,
    #[serde(default)]
    pub drop: Vec<String>,
    #[serde(default)]
    pub libraries: Vec<String>,
    #[serde(default)]
    pub runtime: Vec<String>,
    #[serde(default)]
    pub bundle: Vec<BundleDefinition>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct BundleDefinition {
    pub name: String,
    #[serde(default)]
    pub modules: Vec<String>,
    pub rename: Option<String>,
    #[serde(default)]
    pub headers: Vec<String>,
    #[serde(default)]
    pub libraries: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct DistributionConfig {
    #[serde(rename = "static-lib", default)]
    pub static_lib: Option<PathBuf>,
    #[serde(default)]
    pub headers: Vec<PathBuf>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct BindingsConfig {
    pub rust: Option<RustBindingsConfig>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct RustBindingsConfig {
    pub workspace: PathBuf,
    #[serde(default)]
    pub crates: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct TaskDef {
    pub description: Option<String>,
    #[serde(default, alias = "depends_on", alias = "depends-on")]
    pub deps: Vec<String>,
    pub build: Option<String>,
    #[serde(default)]
    pub run: String,
    /// Ordered list of OCaml module names for the compiler task. Exposed as
    /// `$OCAML_MODULES` (space-separated) in the task environment.
    #[serde(default, rename = "ocaml-modules")]
    pub ocaml_modules: Vec<String>,
    /// OCaml findlib package dependencies. Exposed as `$OCAML_PACKAGES`
    /// (comma-separated, ready for `-package`).
    #[serde(default, rename = "ocaml-packages")]
    pub ocaml_packages: Vec<String>,
    /// Declarative OCaml binary target definition. When present, valerie
    /// drives incremental compilation and linking natively without shell scripts.
    #[serde(rename = "ocaml-bin")]
    pub ocaml_bin: Option<OCamlBuildConfig>,
    /// Strip generated binary or output target
    #[serde(default)]
    pub strip: bool,
    /// Arbitrary extra environment variables injected into the task shell.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct OCamlBuildConfig {
    pub entry: PathBuf,
    pub output: PathBuf,
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub includes: Vec<PathBuf>,
    #[serde(default)]
    pub modules: Vec<String>,
    #[serde(default, rename = "modules-dir")]
    pub modules_dir: Option<PathBuf>,
    #[serde(default)]
    pub bridge: Option<PathBuf>,
    #[serde(default)]
    pub libraries: Vec<PathBuf>,
    #[serde(default)]
    pub cc: Option<String>,
    #[serde(default)]
    pub strip: bool,
    /// Optional parser sub-target to generate (.mly/.mll) and compile into an archive (.cmxa)
    #[serde(default)]
    pub parser: Option<OCamlParserConfig>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct OCamlParserConfig {
    pub dir: PathBuf,
    #[serde(default, rename = "output-dir")]
    pub output_dir: Option<PathBuf>,
    pub mly: Option<PathBuf>,
    pub mll: Option<PathBuf>,
    #[serde(default)]
    pub generator: Option<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    pub archive: PathBuf,
}

impl Manifest {
    /// Loads a manifest from a specific file path.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ManifestError> {
        let content = std::fs::read_to_string(path.as_ref())?;
        let manifest: Manifest = toml::from_str(&content)?;
        Ok(manifest)
    }

    /// Probes current working directory and parent directories for `valerie.toml`.
    pub fn probe_current_dir() -> Option<(PathBuf, Self)> {
        let mut curr = std::env::current_dir().ok()?;
        loop {
            let candidate = curr.join("valerie.toml");
            if candidate.is_file()
                && let Ok(manifest) = Self::from_file(&candidate)
            {
                return Some((candidate, manifest));
            }
            if !curr.pop() {
                break;
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_manifest() {
        let toml_str = r#"
            [package]
            name = "my_crypto"
            version = "0.1.0"
            authors = ["Alice <alice@example.com>"]

            [tasks.test]
            description = "Run test suite"
            run = "cargo test"
        "#;
        let m: Manifest =
            toml::from_str(toml_str).expect("Failed to parse minimal manifest");
        assert_eq!(m.package.name, "my_crypto");
        assert_eq!(m.tasks.len(), 1);
        assert_eq!(m.tasks["test"].run, "cargo test");
    }

    #[test]
    fn test_parse_full_manifest_from_rfc() {
        let toml_str = r#"
            [package]
            name = "shuangning"
            version = "0.1.0"
            authors = ["Seuriin", "Iris-Seravelle"]
            edition = "2026"
            description = "Verified Adelic384 AEAD & Shuangning Stream Cipher Suite"

            [toolchain]
            fstar = "auto"
            vale = "auto"
            karamel = "auto"
            clang = "auto"
            z3 = "4.13.3"

            [target.aarch64-linux]
            features = ["+crypto"]
            dit = true
            assembler = "native"

            [tasks.sat]
            description = "SMT/SAT Preimage Inversion Resistance Benchmark"
            run = "python3 cryptanalysis/sat_inversion.py"
        "#;
        let m: Manifest =
            toml::from_str(toml_str).expect("Failed to parse RFC manifest");
        assert_eq!(m.package.name, "shuangning");
        assert_eq!(m.target["aarch64-linux"].dit, true);
        assert_eq!(m.tasks["sat"].run, "python3 cryptanalysis/sat_inversion.py");
    }
}
