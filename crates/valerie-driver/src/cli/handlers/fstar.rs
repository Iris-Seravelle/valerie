//! F* and Pulse formal verification handler.
//!
//! Handles single-file verification for Vale specs (.vaf) and direct F*/Pulse
//! modules (.fst, .fsti), as well as workspace-level DAG dependency wave verification.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use valerie_depgraph::{DependencyGraph, ModuleNode};
use valerie_driver::terminal::{Status, format_tool_diagnostics};
use valerie_driver::vale_bridge::{
    compile_and_extract_vaf, is_fstar_available, verify_fst_with_fstar,
};
use valerie_manifest::Manifest;
use valerie_manifest::resolver::ResolvedEnvironment;

pub struct FStarHandler;

impl FStarHandler {
    /// Dispatches verification for a single input file or the workspace DAG.
    pub fn verify(
        file: Option<&Path>,
        pulse_only: bool,
        spec_only: bool,
        jobs: usize,
        force: bool,
        manifest_ctx: Option<&(PathBuf, Manifest)>,
        vale_bin: &Path,
        verbose: bool,
        start_time: Instant,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(input_file) = file {
            Self::verify_single_file(
                input_file,
                pulse_only,
                spec_only,
                force,
                manifest_ctx,
                vale_bin,
                verbose,
                start_time,
            )
        } else if let Some((_, manifest)) = manifest_ctx {
            Self::verify_workspace(
                manifest, pulse_only, spec_only, jobs, force, start_time,
            )
        } else {
            Err("`verify` requires an input file or an active workspace manifest (valerie.toml).".into())
        }
    }

    /// Verifies a single file according to its file type.
    fn verify_single_file(
        input_file: &Path,
        pulse_only: bool,
        _spec_only: bool,
        _force: bool,
        manifest_ctx: Option<&(PathBuf, Manifest)>,
        vale_bin: &Path,
        verbose: bool,
        start_time: Instant,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let ext = input_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        if !is_fstar_available() {
            return Err("F* formal verification engine (`fstar.exe` or `fstar`) is not available.".into());
        }

        match ext {
            "vaf" => {
                if verbose {
                    Status::diagnostic(
                        "Verification",
                        format!(
                            "Running Everest Vale + F* proof pipeline for `{}`",
                            input_file.display()
                        ),
                    );
                    Status::detail(format!(
                        "Vale toolchain:       {}",
                        vale_bin.display()
                    ));
                    Status::detail(
                        "F* proof checker:     fstar.exe with Z3 SMT solver",
                    );
                }
                Status::compiling(format!("Vale spec {}", input_file.display()));
                let temp_dir = std::env::temp_dir().join("valerie_verify_tmp");
                let _ = fs::create_dir_all(&temp_dir);
                let stem = input_file
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy();
                let out_fst = temp_dir.join(format!("{}.fst", stem));
                let out_fsti = temp_dir.join(format!("{}.fsti", stem));

                if let Err(e) =
                    compile_and_extract_vaf(vale_bin, input_file, &out_fst, &out_fsti)
                {
                    return Err(format!("Vale compilation error: {}", e).into());
                }

                Status::verifying(format!(
                    "F* proof obligations for {}",
                    input_file.display()
                ));
                if let Err(e) = verify_fst_with_fstar(&out_fsti, &out_fst, &temp_dir) {
                    return Err(format!("F* verification failure: {}", e).into());
                }

                Status::finished(format!(
                    "Verification passed for `{}` in {:.2?}",
                    input_file.display(),
                    start_time.elapsed()
                ));
                Ok(())
            }
            "fst" | "fsti" => {
                // Direct F* / Pulse module verification
                let content = fs::read_to_string(input_file).unwrap_or_default();
                let is_pulse = pulse_only || content.contains("#lang-pulse");

                if verbose {
                    Status::diagnostic(
                        "Verification",
                        format!(
                            "Running F* proof pipeline for `{}` (Pulse: {})",
                            input_file.display(),
                            is_pulse
                        ),
                    );
                }

                Status::verifying(format!("{}", input_file.display()));

                // Smartly piggyback on Manifest probe and Resolver architecture
                let probed_manifest = manifest_ctx
                    .map(|(p, m)| (p.clone(), m.clone()))
                    .or_else(|| {
                        let mut dir = input_file.parent()?.to_path_buf();
                        loop {
                            let cand = dir.join("valerie.toml");
                            if cand.is_file() {
                                if let Ok(m) = Manifest::from_file(&cand) {
                                    return Some((cand, m));
                                }
                            }
                            if !dir.pop() {
                                break;
                            }
                        }
                        // Also check standard subdirectories like valerie-core
                        let sub_cand = PathBuf::from("valerie-core/valerie.toml");
                        if sub_cand.is_file() {
                            if let Ok(m) = Manifest::from_file(&sub_cand) {
                                return Some((sub_cand, m));
                            }
                        }
                        None
                    });

                let base_dir = probed_manifest.as_ref().and_then(|(p, _)| p.parent().map(|d| d.to_path_buf()));
                let resolved_env = if let Some((_, ref m)) = probed_manifest {
                    ResolvedEnvironment::resolve_with_manifest_and_base(m, base_dir.as_deref())
                } else {
                    ResolvedEnvironment::resolve()
                };

                let fstar_bin = resolved_env
                    .fstar_exe
                    .as_deref()
                    .unwrap_or(Path::new("fstar.exe"));

                let ver_cfg = probed_manifest.as_ref().and_then(|(_, m)| m.verification.clone());

                // F* enforces that top-level module name matches the source filename.
                // If a user emits to arbitrary names (e.g. `test.fst` with `module ChaCha20_Quarter_Pulse`),
                // copy to a temporary directory with the authentic module filename so F* verifies cleanly.
                let mut target_file_to_verify = input_file.to_path_buf();
                let mut temp_module_holder: Option<PathBuf> = None;

                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("module ") {
                        let mod_name = trimmed["module ".len()..]
                            .trim()
                            .trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '.');
                        let file_stem = input_file
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy();

                        if !mod_name.is_empty() && !file_stem.eq_ignore_ascii_case(mod_name) {
                            let temp_dir = std::env::temp_dir().join("valerie_verify_fstar_mod");
                            let _ = fs::create_dir_all(&temp_dir);
                            let ext_str = if ext == "fsti" { "fsti" } else { "fst" };
                            let aliased_path = temp_dir.join(format!("{}.{}", mod_name, ext_str));
                            let _ = fs::copy(input_file, &aliased_path);
                            target_file_to_verify = aliased_path.clone();
                            temp_module_holder = Some(aliased_path);
                        }
                        break;
                    }
                }

                let mut cmd = Command::new(fstar_bin);

                // Add includes from manifest if present
                if let Some(ref cfg) = ver_cfg {
                    for inc in &cfg.include {
                        if let Some(ref b) = base_dir {
                            cmd.arg("--include").arg(b.join(inc));
                        } else {
                            cmd.arg("--include").arg(inc);
                        }
                    }
                    if let Some(ref cache) = cfg.cache_dir {
                        let cache_p = if let Some(ref b) = base_dir { b.join(cache) } else { PathBuf::from(cache) };
                        let _ = fs::create_dir_all(&cache_p);
                        cmd.arg("--cache_dir").arg(&cache_p);
                        cmd.arg("--include").arg(&cache_p);
                    } else {
                        let cache_p = if let Some(ref b) = base_dir { b.join(".cache") } else { PathBuf::from(".cache") };
                        let _ = fs::create_dir_all(&cache_p);
                        cmd.arg("--cache_dir").arg(&cache_p);
                        cmd.arg("--include").arg(&cache_p);
                    }
                    if let Some(ref warn_err) = cfg.warn_error {
                        cmd.arg("--warn_error").arg(warn_err);
                    } else {
                        cmd.arg("--warn_error").arg("-271-272-241");
                    }
                    if !cfg.already_cached.is_empty() {
                        cmd.arg("--already_cached").arg(cfg.already_cached.join(","));
                    }
                } else {
                    let cache_p = PathBuf::from(".cache");
                    let _ = fs::create_dir_all(&cache_p);
                    cmd.arg("--cache_dir").arg(&cache_p);
                    cmd.arg("--include").arg(&cache_p);
                    cmd.arg("--warn_error").arg("-271-272-241");
                }

                // Include file's parent directory
                if let Some(parent) = input_file.parent() {
                    if !parent.as_os_str().is_empty() {
                        cmd.arg("--include").arg(parent);
                    }
                }

                // Include ulib and pulse_lib from resolved_env
                if let Some(ref ulib) = resolved_env.fstar_ulib {
                    cmd.arg("--include").arg(ulib);
                }
                if is_pulse {
                    if let Some(ref pulse_lib) = resolved_env.pulse_lib {
                        cmd.arg("--include").arg(pulse_lib);
                    }
                }

                cmd.arg("--cache_checked_modules");
                cmd.arg(&target_file_to_verify);

                let output = cmd.output()?;
                if let Some(temp_path) = temp_module_holder {
                    let _ = fs::remove_file(temp_path);
                }

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let combined = format!("{}\n{}", stdout, stderr);
                    let formatted = format_tool_diagnostics(&combined);
                    return Err(format!("F* verification failure:\n\n{}", formatted).into());
                }

                Status::finished(format!(
                    "Verification passed for `{}` in {:.2?}",
                    input_file.display(),
                    start_time.elapsed()
                ));
                Ok(())
            }
            _ => Err(format!(
                "Unsupported file format for verification: `{}` (expected .fst, .fsti, or .vaf)",
                input_file.display()
            )
            .into()),
        }
    }

    /// Verifies workspace modules according to dependency waves.
    fn verify_workspace(
        manifest: &Manifest,
        pulse_only: bool,
        spec_only: bool,
        jobs: usize,
        force: bool,
        start_time: Instant,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let ver_cfg = manifest
            .verification
            .as_ref()
            .ok_or("Manifest is missing [verification] table")?;

        let dep_graph = DependencyGraph::build(&ver_cfg.include).map_err(|e| {
            format!("Failed to compute workspace dependency DAG: {}", e)
        })?;

        let batches = dep_graph.execution_batches();

        let resolved_env = ResolvedEnvironment::resolve();
        let fstar_bin = resolved_env
            .fstar_exe
            .as_deref()
            .unwrap_or(Path::new("fstar.exe"));

        let cache_dir = ver_cfg.cache_dir.as_deref().unwrap_or(Path::new(".cache"));
        let hints_dir = ver_cfg.hints_dir.as_deref().unwrap_or(Path::new("hints"));
        let warn_err = ver_cfg.warn_error.as_deref().unwrap_or("-271-272-241");

        fs::create_dir_all(cache_dir)?;
        fs::create_dir_all(hints_dir)?;

        Status::testing(format!(
            "Workspace DAG verification: running across {} dependency wave(s) (concurrency: {} job(s))",
            batches.len(),
            jobs
        ));

        let mut total_verified = 0;
        let mut total_skipped = 0;
        let mut recompiled: HashSet<String> = HashSet::new();

        let is_node_up_to_date = |node: &ModuleNode,
                                  recompiled: &HashSet<String>|
         -> bool {
            if force {
                return false;
            }
            let file_name =
                node.path.file_name().unwrap_or_default().to_string_lossy();
            let checked_candidates = [
                cache_dir.join(format!("{}.checked", file_name)),
                cache_dir.join(format!("{}.fst.checked", node.name)),
                node.path.with_extension(format!(
                    "{}.checked",
                    node.path.extension().unwrap_or_default().to_string_lossy()
                )),
                node.path.with_extension("fst.checked"),
                node.path.with_file_name(format!("{}.checked", file_name)),
            ];

            let checked_path = checked_candidates.into_iter().find(|p| p.is_file());
            let checked_path = match checked_path {
                Some(p) => p,
                None => return false,
            };

            for dep in &node.opens {
                if recompiled.contains(dep) {
                    return false;
                }
            }

            if let (Ok(m_src), Ok(m_chk)) =
                (fs::metadata(&node.path), fs::metadata(&checked_path))
                && m_chk.len() > 0
            {
                if let (Ok(t_src), Ok(t_chk)) = (m_src.modified(), m_chk.modified())
                {
                    return t_chk >= t_src;
                }
                return true;
            }
            false
        };

        for (wave_idx, wave) in batches.iter().enumerate() {
            let filtered_nodes: Vec<_> = wave
                .iter()
                .filter(|m| {
                    let path_str = m.path.to_string_lossy();
                    if pulse_only {
                        path_str.contains("pulse")
                    } else if spec_only {
                        path_str.contains("spec")
                    } else {
                        true
                    }
                })
                .cloned()
                .collect();

            if filtered_nodes.is_empty() {
                continue;
            }

            let (to_verify, up_to_date): (Vec<_>, Vec<_>) = filtered_nodes
                .into_iter()
                .partition(|node| !is_node_up_to_date(node, &recompiled));

            total_skipped += up_to_date.len();
            if to_verify.is_empty() {
                continue;
            }

            if jobs > 1 && to_verify.len() > 1 {
                let failure = Arc::new(AtomicBool::new(false));
                let chunks = to_verify.chunks(jobs);
                for chunk in chunks {
                    let mut handles = Vec::new();
                    for node in chunk {
                        let fstar_bin_buf = fstar_bin.to_path_buf();
                        let includes = ver_cfg.include.clone();
                        let pulse_lib = resolved_env.pulse_lib.clone();
                        let cache_dir_buf = cache_dir.to_path_buf();
                        let warn_err_str = warn_err.to_string();
                        let cached_str = ver_cfg.already_cached.join(",");
                        let node_path = node.path.clone();
                        let fail_clone = failure.clone();

                        Status::compiling(format!(
                            "[VERIFY (wave {})] {}",
                            wave_idx + 1,
                            node_path.display()
                        ));
                        handles.push(std::thread::spawn(move || {
                            let mut cmd = Command::new(fstar_bin_buf);
                            for inc in &includes {
                                cmd.arg("--include").arg(inc);
                            }
                            if let Some(ref p) = pulse_lib {
                                cmd.arg("--include").arg(p);
                            }
                            cmd.arg("--include").arg(&cache_dir_buf);
                            cmd.arg("--cache_dir").arg(&cache_dir_buf);
                            cmd.arg("--cache_checked_modules");
                            cmd.arg("--warn_error").arg(warn_err_str);
                            if !cached_str.is_empty() {
                                cmd.arg("--already_cached").arg(cached_str);
                            }
                            cmd.arg(&node_path);

                            match cmd.status() {
                                Ok(s) if s.success() => {
                                    let checked_file_name = format!(
                                        "{}.checked",
                                        node_path
                                            .file_name()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                    );
                                    let checked_path =
                                        cache_dir_buf.join(&checked_file_name);
                                    if let Ok(f) = fs::OpenOptions::new()
                                        .write(true)
                                        .open(&checked_path)
                                    {
                                        let _ = f.set_modified(
                                            std::time::SystemTime::now(),
                                        );
                                    }
                                }
                                _ => {
                                    fail_clone.store(true, Ordering::SeqCst);
                                }
                            }
                        }));
                    }
                    for h in handles {
                        let _ = h.join();
                    }
                    if failure.load(Ordering::SeqCst) {
                        return Err(format!(
                            "Parallel verification failed in wave {}",
                            wave_idx + 1
                        )
                        .into());
                    }
                }
            } else {
                for node in &to_verify {
                    Status::compiling(format!(
                        "[VERIFY (wave {})] {}",
                        wave_idx + 1,
                        node.path.display()
                    ));
                    let mut cmd = Command::new(fstar_bin);
                    for inc in &ver_cfg.include {
                        cmd.arg("--include").arg(inc);
                    }
                    if let Some(ref p) = resolved_env.pulse_lib {
                        cmd.arg("--include").arg(p);
                    }
                    cmd.arg("--include").arg(cache_dir);
                    cmd.arg("--cache_dir").arg(cache_dir);
                    cmd.arg("--cache_checked_modules");
                    cmd.arg("--warn_error").arg(warn_err);
                    if !ver_cfg.already_cached.is_empty() {
                        cmd.arg("--already_cached")
                            .arg(ver_cfg.already_cached.join(","));
                    }
                    cmd.arg(&node.path);

                    let status = cmd.status()?;
                    if !status.success() {
                        return Err(format!(
                            "Verification failed for `{}`",
                            node.path.display()
                        )
                        .into());
                    } else {
                        let checked_file_name = format!(
                            "{}.checked",
                            node.path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                        );
                        let checked_path = cache_dir.join(&checked_file_name);
                        if let Ok(f) =
                            fs::OpenOptions::new().write(true).open(&checked_path)
                        {
                            let _ = f.set_modified(std::time::SystemTime::now());
                        }
                    }
                }
            }
            for node in &to_verify {
                recompiled.insert(node.name.clone());
            }
            total_verified += to_verify.len();
        }

        if total_verified == 0 {
            Status::finished(format!(
                "Workspace formal verification: all {} module(s) up to date (cached) in {:.2?}",
                total_skipped,
                start_time.elapsed()
            ));
        } else {
            Status::finished(format!(
                "Workspace formal verification passed: {} module(s) verified ({} up to date skipped) in {:.2?}",
                total_verified,
                total_skipped,
                start_time.elapsed()
            ));
        }

        Ok(())
    }
}
