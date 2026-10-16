//! Workspace KaRaMeL extraction orchestration pipeline.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use valerie_driver::terminal::Status;
use valerie_manifest::Manifest;
use valerie_manifest::resolver::ResolvedEnvironment;

/// Orchestrates F* KaRaMeL AST emission and KaRaMeL C bundle extraction for a workspace.
pub fn extract_workspace(
    manifest: &Manifest,
    start_time: Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    let ext_cfg = manifest
        .extraction
        .as_ref()
        .and_then(|e| e.c.as_ref())
        .ok_or("Manifest is missing [extraction.c] configuration table")?;

    let resolved_env = ResolvedEnvironment::resolve();
    let krml_bin = resolved_env
        .karamel_exe
        .as_deref()
        .unwrap_or(Path::new("krml"));

    let out_dir = ext_cfg.output_dir.as_deref().unwrap_or(Path::new("dist"));
    fs::create_dir_all(out_dir)?;

    Status::compiling(format!(
        "Extracting C bundles via KaRaMeL into `{}`",
        out_dir.display()
    ));

    let mut cmd = std::process::Command::new(krml_bin);
    cmd.arg("-skip-compilation");
    if ext_cfg.minimal_headers {
        cmd.arg("-minimal");
    }
    if ext_cfg.builtin_uint128 {
        cmd.arg("-fbuiltin-uint128");
    }
    cmd.arg("-fparentheses");
    if let Some(ref we) = ext_cfg.warn_error {
        cmd.arg("-warn-error").arg(we);
    }

    // Add drop list
    if !ext_cfg.drop.is_empty() {
        cmd.arg("-drop").arg(ext_cfg.drop.join(","));
    }

    // Collect global and per-bundle libraries
    let mut all_libraries = ext_cfg.libraries.clone();
    for bundle in &ext_cfg.bundle {
        for lib in &bundle.libraries {
            if !all_libraries.contains(lib) {
                all_libraries.push(lib.clone());
            }
        }
    }
    if !all_libraries.is_empty() {
        cmd.arg("-library").arg(all_libraries.join(","));
    }

    // Add bundles
    for bundle in &ext_cfg.bundle {
        let mut b_arg = format!("{}={}", bundle.name, bundle.modules.join(","));
        if let Some(ref r) = bundle.rename {
            b_arg.push_str(&format!("[rename={}]", r));
        }
        cmd.arg("-bundle").arg(b_arg);
        for h in &bundle.headers {
            cmd.arg("-add-include").arg(format!("\"{}\"", h));
        }
    }

    cmd.arg("-tmpdir").arg(out_dir);

    // Add all *.krml files found in output directory
    let mut krml_files = Vec::new();
    for entry in fs::read_dir(out_dir).into_iter().flatten().flatten() {
        let p = entry.path();
        if p.extension().is_some_and(|e| e == "krml") {
            krml_files.push(p);
        }
    }

    // Collect all modules to extract
    let mut modules_to_extract = Vec::new();
    for bundle in &ext_cfg.bundle {
        for m in &bundle.modules {
            if !modules_to_extract.contains(m) {
                modules_to_extract.push(m.clone());
            }
        }
    }

    // Check if .krml files need generation or are older than .fst sources
    let mut need_codegen = krml_files.is_empty();
    if !need_codegen {
        for m in &modules_to_extract {
            let m_krml = out_dir.join(format!("{}.krml", m.replace('.', "_")));
            if !m_krml.exists() {
                need_codegen = true;
                break;
            }
        }
    }

    if need_codegen
        && let Some(ref ver_cfg) = manifest.verification
        && let Some(fstar_bin) = resolved_env.fstar_exe.as_ref()
    {
        Status::compiling(format!(
            "Emitting KaRaMeL AST files (.krml) from verified F* modules into `{}`",
            out_dir.display()
        ));

        let cache_dir = ver_cfg.cache_dir.as_deref().unwrap_or(Path::new(".cache"));
        let warn_err = ver_cfg.warn_error.as_deref().unwrap_or("-271-272-241");
        let cached_str = ver_cfg.already_cached.join(",");

        for mod_name in &modules_to_extract {
            let mut fstar_cmd = std::process::Command::new(fstar_bin);
            for inc in &ver_cfg.include {
                fstar_cmd.arg("--include").arg(inc);
            }
            if let Some(ref ulib) = resolved_env.fstar_ulib {
                fstar_cmd.arg("--include").arg(ulib);
            }
            if let Some(ref p) = resolved_env.pulse_lib {
                fstar_cmd.arg("--include").arg(p);
            }
            fstar_cmd.arg("--cache_dir").arg(cache_dir);
            fstar_cmd.arg("--odir").arg(out_dir);
            fstar_cmd.arg("--cache_checked_modules");
            fstar_cmd.arg("--warn_error").arg(warn_err);
            if !cached_str.is_empty() {
                fstar_cmd.arg("--already_cached").arg(&cached_str);
            }
            fstar_cmd.arg("--codegen").arg("krml");
            fstar_cmd.arg("--extract_module").arg(mod_name);

            // Find source .fst file for module across all include dirs recursively or by filename
            let fst_leaf = format!("{}.fst", mod_name);
            let mut search_dirs: Vec<PathBuf> = ver_cfg.include.clone();
            if let Some(ref ulib) = resolved_env.fstar_ulib {
                if !search_dirs.contains(ulib) {
                    search_dirs.push(ulib.clone());
                }
            }
            if let Some(ref pulse) = resolved_env.pulse_lib {
                if !search_dirs.contains(pulse) {
                    search_dirs.push(pulse.clone());
                }
            }

            let mut found_src = None;
            for inc in &search_dirs {
                let candidate = inc.join(&fst_leaf);
                if candidate.exists() {
                    found_src = Some(candidate);
                    break;
                }
                if let Ok(entries) = fs::read_dir(inc) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_file()
                            && p.file_name().and_then(|n| n.to_str())
                                == Some(&fst_leaf)
                        {
                            found_src = Some(p);
                            break;
                        }
                    }
                }
                if found_src.is_some() {
                    break;
                }
            }

            if let Some(src_path) = found_src {
                fstar_cmd.arg(src_path);
                let status = fstar_cmd.status()?;
                if !status.success() {
                    return Err(format!(
                        "F* KaRaMeL AST extraction failed for module `{}` with status {}",
                        mod_name, status
                    ).into());
                }
            } else {
                return Err(format!(
                    "Could not find source .fst file for module `{}` in include paths",
                    mod_name
                ).into());
            }
        }

        // Re-scan for newly produced .krml files
        for entry in fs::read_dir(out_dir).into_iter().flatten().flatten() {
            let p = entry.path();
            if p.extension().is_some_and(|e| e == "krml") {
                krml_files.push(p);
            }
        }
    }

    if krml_files.is_empty() {
        eprintln!(
            "Notice: No *.krml AST files found in `{}` to extract.",
            out_dir.display()
        );
    } else {
        for m in &modules_to_extract {
            let target_krml = out_dir.join(format!("{}.krml", m.replace('.', "_")));
            if target_krml.exists() {
                cmd.arg(&target_krml);
            }
        }
        let status = cmd.status()?;
        if !status.success() {
            return Err(format!(
                "KaRaMeL C extraction failed with status {}",
                status
            )
            .into());
        }
    }

    Status::finished(format!(
        "Extracted C code to `{}` in {:.2?}",
        out_dir.display(),
        start_time.elapsed()
    ));

    Ok(())
}
