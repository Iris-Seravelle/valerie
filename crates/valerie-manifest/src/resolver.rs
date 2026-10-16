//! Toolchain resolver: discovers F*, Vale, KaRaMeL, Clang, and Z3 environment paths.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct ResolvedEnvironment {
    pub fstar_exe: Option<PathBuf>,
    pub fstar_ulib: Option<PathBuf>,
    pub pulse_lib: Option<PathBuf>,
    pub vale_exe: Option<PathBuf>,
    pub karamel_exe: Option<PathBuf>,
    pub karamel_include: Option<PathBuf>,
    pub karamel_lib: Option<PathBuf>,
    pub fstar_lib: Option<PathBuf>,
    pub clang_exe: Option<PathBuf>,
}

impl ResolvedEnvironment {
    /// Discovers system and environment prefixes dynamically ($PREFIX, $OPAM_SWITCH_PREFIX, etc.)
    pub fn discovered_prefixes() -> Vec<PathBuf> {
        let mut prefixes = Vec::new();

        // 1. Explicit toolchain / package manager environment variables
        if let Ok(opam_prefix) = std::env::var("OPAM_SWITCH_PREFIX") {
            prefixes.push(PathBuf::from(opam_prefix));
        }
        if let Ok(prefix) = std::env::var("PREFIX") {
            prefixes.push(PathBuf::from(prefix));
        }

        // 2. Query opam directly if available
        if let Ok(out) = Command::new("opam").args(["var", "prefix"]).output()
            && out.status.success()
            && let Ok(s) = String::from_utf8(out.stdout)
        {
            let p = PathBuf::from(s.trim());
            if !prefixes.contains(&p) {
                prefixes.push(p);
            }
        }

        // 3. User home directory prefixes
        if let Ok(home) = std::env::var("HOME").map(PathBuf::from) {
            let user_candidates = [
                home.join(".opam/fstar"),
                home.join(".opam/default"),
                home.join(".local"),
            ];
            for cand in user_candidates {
                if cand.is_dir() && !prefixes.contains(&cand) {
                    prefixes.push(cand);
                }
            }
        }

        // 4. Standard POSIX / Linux / Unix prefixes
        for std_prefix in ["/usr/local", "/usr"] {
            let p = PathBuf::from(std_prefix);
            if p.is_dir() && !prefixes.contains(&p) {
                prefixes.push(p);
            }
        }

        prefixes
    }

    /// Attempts to discover all available verification and compiler tools.
    pub fn resolve() -> Self {
        let fstar_exe = which::which("fstar.exe")
            .or_else(|_| which::which("fstar"))
            .ok();

        let fstar_ulib = fstar_exe
            .as_ref()
            .and_then(|exe| Self::exec_locate(exe, &["--locate_file", "Prims.fst"]))
            .map(PathBuf::from)
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .filter(|p| p.join("Prims.fst").exists())
            .or_else(|| {
                fstar_exe
                    .as_ref()
                    .and_then(|exe| Self::exec_locate(exe, &["--locate_lib"]))
                    .map(PathBuf::from)
                    .map(|p| p.join("ulib"))
                    .filter(|p| p.join("Prims.fst").exists())
            })
            .or_else(Self::probe_known_ulib);

        let fstar_lib = Self::discover_fstar_lib(&fstar_ulib);

        let pulse_lib = fstar_ulib.as_ref().and_then(Self::probe_pulse_lib);

        let vale_exe = std::env::var("VALE_BIN")
            .ok()
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .or_else(|| {
                std::env::var("VALE_DIR")
                    .or_else(|_| std::env::var("VALE_HOME"))
                    .ok()
                    .and_then(|d| {
                        Self::resolve_tool_path(
                            &d,
                            &[
                                "valerie-compiler",
                                "vale-compiler",
                                "vale",
                                "vale.exe",
                            ],
                        )
                    })
            })
            .or_else(|| which::which("valerie-compiler").ok())
            .or_else(|| which::which("valy").ok())
            .or_else(|| which::which("vale-compiler").ok())
            .or_else(|| which::which("vale").ok())
            .or_else(|| which::which("vale.exe").ok());

        let karamel_exe = which::which("krml")
            .or_else(|_| which::which("karamel"))
            .ok();

        let karamel_include = karamel_exe
            .as_ref()
            .and_then(|exe| Self::exec_locate(exe, &["-locate-include"]))
            .map(PathBuf::from)
            .filter(|p| {
                p.join("krmllib.h").exists() || p.join("krml/krmllib.h").exists()
            })
            .or_else(Self::probe_known_krml_include);

        let karamel_lib = karamel_exe
            .as_ref()
            .and_then(|exe| Self::exec_locate(exe, &["-locate-krmllib"]))
            .map(PathBuf::from)
            .filter(|p| p.join("dist/minimal").exists())
            .or_else(Self::probe_known_krml_lib);

        let clang_exe = which::which("clang").ok();

        Self {
            fstar_exe,
            fstar_ulib,
            fstar_lib,
            pulse_lib,
            vale_exe,
            karamel_exe,
            karamel_include,
            karamel_lib,
            clang_exe,
        }
    }

    /// Dynamically locates the OCaml F* library directory containing prims.cmi / fstar.cmxa
    fn discover_fstar_lib(fstar_ulib: &Option<PathBuf>) -> Option<PathBuf> {
        // 1. Try querying ocamlfind directly
        if let Ok(out) = Command::new("ocamlfind").args(["query", "fstar"]).output()
            && out.status.success()
            && let Ok(s) = String::from_utf8(out.stdout)
        {
            let p = PathBuf::from(s.trim());
            if p.join("prims.cmi").exists() || p.join("fstar.cmxa").exists() {
                return Some(p);
            }
            let sub_lib = p.join("lib");
            if sub_lib.join("prims.cmi").exists()
                || sub_lib.join("fstar.cmxa").exists()
            {
                return Some(sub_lib);
            }
        }

        // 2. Derive relative to fstar_ulib
        if let Some(ulib) = fstar_ulib
            && let Some(parent) = ulib.parent()
        {
            let sub_lib = parent.join("lib");
            if sub_lib.join("prims.cmi").exists()
                || sub_lib.join("fstar.cmxa").exists()
            {
                return Some(sub_lib);
            }
            if parent.join("prims.cmi").exists()
                || parent.join("fstar.cmxa").exists()
            {
                return Some(parent.to_path_buf());
            }
        }

        // 3. Search across discovered prefixes
        for prefix in Self::discovered_prefixes() {
            let candidates = [
                prefix.join("lib/fstar/lib"),
                prefix.join("lib/fstar"),
                prefix.join("lib/ocaml/fstar"),
            ];
            for cand in candidates {
                if cand.join("prims.cmi").exists()
                    || cand.join("fstar.cmxa").exists()
                {
                    return Some(cand);
                }
            }
        }

        None
    }

    /// Resolves a single tool path given a configured value or command name, optionally relative to a base directory.
    pub fn resolve_tool_path(
        configured: &str,
        binary_names: &[&str],
    ) -> Option<PathBuf> {
        Self::resolve_tool_path_relative(configured, binary_names, None)
    }

    /// Resolves tool path relative to a specific base directory (e.g. manifest parent directory).
    pub fn resolve_tool_path_relative(
        configured: &str,
        binary_names: &[&str],
        base_dir: Option<&Path>,
    ) -> Option<PathBuf> {
        if configured.is_empty() || configured == "auto" {
            return None;
        }
        let raw_p = PathBuf::from(configured);
        let candidates = if let Some(base) = base_dir {
            if raw_p.is_absolute() {
                vec![raw_p.clone()]
            } else {
                vec![base.join(&raw_p), raw_p.clone()]
            }
        } else {
            vec![raw_p.clone()]
        };

        for p in candidates {
            if p.is_file() {
                return Some(p);
            }
            if p.is_dir() {
                for name in binary_names {
                    let bin = p.join(name);
                    if bin.is_file() {
                        return Some(bin);
                    }
                }
            }
        }

        if configured.contains('/') || configured.contains('\\') {
            if let Some(base) = base_dir {
                if !raw_p.is_absolute() {
                    let combined = base.join(&raw_p);
                    if combined.exists() {
                        return Some(combined);
                    }
                }
            }
            return Some(raw_p);
        }
        which::which(configured).ok()
    }

    /// Resolves toolchain taking valerie.toml manifest configuration into account generically.
    pub fn resolve_with_manifest(manifest: &crate::Manifest) -> Self {
        Self::resolve_with_manifest_and_base(manifest, None)
    }

    /// Resolves toolchain taking manifest and manifest path into account for relative tool discovery.
    pub fn resolve_with_manifest_and_base(
        manifest: &crate::Manifest,
        base_dir: Option<&Path>,
    ) -> Self {
        let mut env = Self::resolve();

        // Generic tool resolution for all toolchain tools
        if let Some(p) = Self::resolve_tool_path_relative(
            &manifest.toolchain.vale,
            &["valerie-compiler", "vale-compiler", "vale", "vale.exe"],
            base_dir,
        ) {
            env.vale_exe = Some(p);
        }
        if let Some(p) = Self::resolve_tool_path_relative(
            &manifest.toolchain.fstar,
            &["fstar.exe", "fstar"],
            base_dir,
        ) {
            env.fstar_exe = Some(p);
        }
        if let Some(p) = Self::resolve_tool_path_relative(
            &manifest.toolchain.karamel,
            &["krml", "karamel"],
            base_dir,
        ) {
            env.karamel_exe = Some(p);
        }
        if let Some(p) = Self::resolve_tool_path_relative(
            &manifest.toolchain.clang,
            &["clang"],
            base_dir,
        ) {
            env.clang_exe = Some(p);
        }

        env
    }

    /// Converts resolved toolchain paths to an environment variable map suitable for task runners.
    pub fn to_env_map(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        if let Some(ref p) = self.fstar_exe {
            m.insert("FSTAR_EXE".into(), p.to_string_lossy().into());
        }
        if let Some(ref p) = self.fstar_ulib {
            m.insert("FSTAR_ULIB".into(), p.to_string_lossy().into());
        }
        if let Some(ref p) = self.fstar_lib {
            m.insert("FSTAR_LIB".into(), p.to_string_lossy().into());
        }
        if let Some(ref p) = self.pulse_lib {
            m.insert("PULSE_LIB".into(), p.to_string_lossy().into());
        }
        if let Some(ref p) = self.vale_exe {
            m.insert("VALE_EXE".into(), p.to_string_lossy().into());
        }
        if let Some(ref p) = self.karamel_exe {
            m.insert("KRML_EXE".into(), p.to_string_lossy().into());
        }
        if let Some(ref p) = self.karamel_include {
            m.insert("KRML_INCLUDE".into(), p.to_string_lossy().into());
        }
        if let Some(ref p) = self.karamel_lib {
            m.insert("KRML_LIB".into(), p.to_string_lossy().into());
        }
        if let Some(ref p) = self.clang_exe {
            m.insert("CC".into(), p.to_string_lossy().into());
        }
        m
    }

    fn exec_locate(binary: &PathBuf, args: &[&str]) -> Option<String> {
        Command::new(binary)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
    }

    fn probe_known_ulib() -> Option<PathBuf> {
        for prefix in Self::discovered_prefixes() {
            let candidates = [
                prefix.join("lib/fstar/ulib"),
                prefix.join("ulib"),
                prefix.join("share/fstar/ulib"),
            ];
            for cand in candidates {
                if cand.join("Prims.fst").exists() {
                    return Some(cand);
                }
            }
        }
        None
    }

    fn probe_pulse_lib(fstar_ulib: &PathBuf) -> Option<PathBuf> {
        // 1. Check relative to fstar_ulib
        let rel_candidates = [
            fstar_ulib.join("../pulse/pulse/lib"),
            fstar_ulib.join("../pulse/lib"),
            fstar_ulib.join("pulse/lib"),
        ];
        for cand in rel_candidates {
            if cand.join("Pulse.fst").exists() {
                return Some(cand);
            }
        }

        // 2. Search across discovered prefixes
        for prefix in Self::discovered_prefixes() {
            let candidates = [
                prefix.join("pulse/pulse/lib"),
                prefix.join("lib/pulse/lib"),
                prefix.join("share/pulse/lib"),
            ];
            for cand in candidates {
                if cand.join("Pulse.fst").exists() {
                    return Some(cand);
                }
            }
        }

        None
    }

    fn probe_known_krml_include() -> Option<PathBuf> {
        for prefix in Self::discovered_prefixes() {
            let candidates = [
                prefix.join("opt/karamel/include"),
                prefix.join("include/krml"),
                prefix.join("include"),
                prefix.join("lib/karamel/include"),
            ];
            for cand in candidates {
                if cand.join("krmllib.h").exists()
                    || cand.join("krml/krmllib.h").exists()
                {
                    return Some(cand);
                }
            }
        }
        None
    }

    fn probe_known_krml_lib() -> Option<PathBuf> {
        for prefix in Self::discovered_prefixes() {
            let candidates = [
                prefix.join("opt/karamel/krmllib"),
                prefix.join("lib/karamel/krmllib"),
                prefix.join("lib/krmllib"),
            ];
            for cand in candidates {
                if cand.join("dist/minimal").exists() {
                    return Some(cand);
                }
            }
        }
        None
    }
}
