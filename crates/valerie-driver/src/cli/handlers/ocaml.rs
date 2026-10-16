//! Declarative native OCaml binary target handler.
//!
//! Encapsulates parser generation (menhir/ocamlyacc/ocamllex), incremental F*
//! extraction with selective cache eviction, OCaml module compilation, and native linking.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use valerie_driver::terminal::Status;
use valerie_manifest::{Manifest, OCamlBuildConfig};

/// Replaces `${VAR}` and `$VAR` substrings with their corresponding values from `vars`.
pub fn interpolate_variables(input: &str, vars: &HashMap<String, String>) -> String {
    let mut result = input.to_string();
    for (k, v) in vars {
        let pattern_braced = format!("${{{}}}", k);
        let pattern_simple = format!("${}", k);
        result = result.replace(&pattern_braced, v);
        result = result.replace(&pattern_simple, v);
    }
    result
}

pub struct OcamlBinaryHandler;

impl OcamlBinaryHandler {
    /// Executes the full native OCaml binary build pipeline for a task target.
    pub fn execute(
        cfg: &OCamlBuildConfig,
        env_map: &HashMap<String, String>,
        manifest: Option<&Manifest>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let ocamlfind = PathBuf::from("ocamlfind");

        // 1. Resolve packages
        let packages = Self::resolve_packages(cfg);

        // 2. Resolve include directories
        let include_args = Self::resolve_includes(cfg, env_map);

        let modules_dir = cfg
            .modules_dir
            .as_ref()
            .map(|d| {
                PathBuf::from(interpolate_variables(&d.to_string_lossy(), env_map))
            })
            .unwrap_or_else(|| PathBuf::from("."));

        let search_dirs: Vec<PathBuf> = manifest
            .and_then(|m| m.verification.as_ref())
            .map(|v| v.include.clone())
            .unwrap_or_default();

        let cache_dir = manifest
            .and_then(|m| m.verification.as_ref())
            .and_then(|v| v.cache_dir.clone())
            .unwrap_or_else(|| PathBuf::from(".cache"));

        let already_cached = manifest
            .and_then(|m| m.verification.as_ref())
            .map(|v| v.already_cached.join(","))
            .unwrap_or_default();

        let warn_err = manifest
            .and_then(|m| m.verification.as_ref())
            .and_then(|v| v.warn_error.clone());

        // 3. Build sub-target parser if configured
        if let Some(ref parser_cfg) = cfg.parser {
            Self::build_parser(parser_cfg, &ocamlfind, &packages, env_map)?;
        }

        // 4. Incrementally extract and compile F* and OCaml modules
        let mut compiled_cmx = Vec::new();
        let mut any_module_recompiled = false;

        for mod_name in &cfg.modules {
            let cmx = Self::compile_module(
                mod_name,
                &modules_dir,
                &search_dirs,
                &cache_dir,
                &already_cached,
                warn_err.as_deref(),
                &ocamlfind,
                &packages,
                &include_args,
                env_map,
                &mut any_module_recompiled,
            )?;
            compiled_cmx.push(cmx);
        }

        // 5. Incrementally compile bridge file if specified
        if let Some(ref bridge) = cfg.bridge {
            let bridge_cmx = Self::compile_single_source(
                bridge,
                "OCaml bridge",
                &ocamlfind,
                &packages,
                &include_args,
                env_map,
                any_module_recompiled,
            )?;
            compiled_cmx.push(bridge_cmx);
        }

        // 6. Incrementally compile main entry point
        let entry_cmx = Self::compile_single_source(
            &cfg.entry,
            "OCaml entrypoint",
            &ocamlfind,
            &packages,
            &include_args,
            env_map,
            any_module_recompiled,
        )?;
        compiled_cmx.push(entry_cmx);

        // 7. Link final native binary
        Self::link_native_binary(
            cfg,
            &ocamlfind,
            &packages,
            &include_args,
            &compiled_cmx,
            env_map,
        )?;

        Ok(())
    }

    fn resolve_packages(cfg: &OCamlBuildConfig) -> String {
        cfg.packages.join(",")
    }

    fn resolve_includes(
        cfg: &OCamlBuildConfig,
        env_map: &HashMap<String, String>,
    ) -> Vec<String> {
        let mut include_args = Vec::new();
        for inc in &cfg.includes {
            let expanded = interpolate_variables(&inc.to_string_lossy(), env_map);
            include_args.push("-I".to_string());
            include_args.push(expanded);
        }
        include_args
    }

    fn build_parser(
        parser_cfg: &valerie_manifest::OCamlParserConfig,
        ocamlfind: &Path,
        packages: &str,
        env_map: &HashMap<String, String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let p_dir = PathBuf::from(interpolate_variables(
            &parser_cfg.dir.to_string_lossy(),
            env_map,
        ));
        let out_dir = parser_cfg
            .output_dir
            .as_ref()
            .map(|d| {
                PathBuf::from(interpolate_variables(&d.to_string_lossy(), env_map))
            })
            .unwrap_or_else(|| p_dir.clone());
        fs::create_dir_all(&out_dir)?;

        let archive_dst = out_dir.join(&parser_cfg.archive);

        if let Some(ref mly) = parser_cfg.mly {
            let mly_path = p_dir.join(mly);
            let gen_ml = out_dir.join(mly).with_extension("ml");
            let gen_mli = out_dir.join(mly).with_extension("mli");
            let need_yacc = !gen_ml.is_file() || !gen_mli.is_file() || {
                match (fs::metadata(&mly_path), fs::metadata(&gen_ml)) {
                    (Ok(m_src), Ok(m_gen)) => {
                        m_src
                            .modified()
                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                            > m_gen
                                .modified()
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                    }
                    _ => true,
                }
            };

            if need_yacc {
                let generator =
                    parser_cfg.generator.as_deref().unwrap_or_else(|| {
                        if which::which("menhir").is_ok() {
                            "menhir"
                        } else {
                            "ocamlyacc"
                        }
                    });
                Status::compiling(format!(
                    "Generating parser from {} with {}",
                    mly_path.display(),
                    generator
                ));
                if out_dir != p_dir {
                    let target_mly = out_dir.join(mly);
                    fs::copy(&mly_path, &target_mly)?;
                }
                let mut gen_cmd = std::process::Command::new(generator);
                gen_cmd.arg(mly).current_dir(&out_dir);
                let mut success =
                    gen_cmd.status().map(|s| s.success()).unwrap_or(false);
                if !success
                    && generator == "menhir"
                    && parser_cfg.generator.is_none()
                {
                    if which::which("ocamlyacc").is_ok() {
                        let mut fallback_cmd =
                            std::process::Command::new("ocamlyacc");
                        fallback_cmd.arg(mly).current_dir(&out_dir);
                        success = fallback_cmd
                            .status()
                            .map(|s| s.success())
                            .unwrap_or(false);
                    }
                }
                if !success {
                    return Err(format!(
                        "Parser generation failed for `{}`",
                        mly_path.display()
                    )
                    .into());
                }
            }
        }

        if let Some(ref mll) = parser_cfg.mll {
            let mll_path = p_dir.join(mll);
            let gen_lex_ml = out_dir.join(mll).with_extension("ml");
            let need_lex = !gen_lex_ml.is_file() || {
                match (fs::metadata(&mll_path), fs::metadata(&gen_lex_ml)) {
                    (Ok(m_src), Ok(m_gen)) => {
                        m_src
                            .modified()
                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                            > m_gen
                                .modified()
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                    }
                    _ => true,
                }
            };

            if need_lex {
                Status::compiling(format!(
                    "Generating lexer from {}",
                    mll_path.display()
                ));
                if out_dir != p_dir {
                    let target_mll = out_dir.join(mll);
                    fs::copy(&mll_path, &target_mll)?;
                }
                let status = std::process::Command::new("ocamllex")
                    .arg(mll)
                    .current_dir(&out_dir)
                    .status()?;
                if !status.success() {
                    return Err(format!(
                        "ocamllex failed for `{}`",
                        mll_path.display()
                    )
                    .into());
                }
            }
        }

        // Build parser archive (.cmxa)
        let archive_a = archive_dst.with_extension("a");
        let archive_needs_rebuild =
            !archive_dst.is_file() || !archive_a.is_file() || {
                let m_ar = fs::metadata(&archive_dst)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                parser_cfg.sources.iter().any(|src| {
                    let p = if out_dir.join(src).is_file() {
                        out_dir.join(src)
                    } else {
                        p_dir.join(src)
                    };
                    fs::metadata(&p)
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                        > m_ar
                })
            };

        if archive_needs_rebuild {
            let abs_p_dir =
                fs::canonicalize(&p_dir).unwrap_or_else(|_| p_dir.clone());
            let abs_out_dir =
                fs::canonicalize(&out_dir).unwrap_or_else(|_| out_dir.clone());
            for src in &parser_cfg.sources {
                let src_path = if out_dir.join(src).is_file() {
                    out_dir.join(src)
                } else {
                    p_dir.join(src)
                };
                let abs_src = fs::canonicalize(&src_path).unwrap_or(src_path);

                let mut p_compile_cmd = std::process::Command::new(ocamlfind);
                p_compile_cmd.arg("opt").current_dir(&out_dir);
                if !packages.is_empty() {
                    p_compile_cmd.arg("-package").arg(packages);
                }
                p_compile_cmd.arg("-I").arg(&abs_p_dir);
                p_compile_cmd.arg("-I").arg(&abs_out_dir);
                p_compile_cmd.arg("-c").arg(&abs_src);
                let status = p_compile_cmd.status()?;
                if !status.success() {
                    return Err(format!(
                        "Failed to compile parser source `{}`",
                        src
                    )
                    .into());
                }

                if out_dir != p_dir {
                    let stem = PathBuf::from(src)
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    for ext in &["cmx", "cmi", "o"] {
                        let src_art = p_dir.join(format!("{}.{}", stem, ext));
                        let dst_art = out_dir.join(format!("{}.{}", stem, ext));
                        if src_art.is_file() {
                            let _ = fs::rename(&src_art, &dst_art);
                        }
                    }
                }
            }

            let mut p_ar_cmd = std::process::Command::new(ocamlfind);
            p_ar_cmd
                .arg("opt")
                .arg("-a")
                .arg("-o")
                .arg(&parser_cfg.archive)
                .current_dir(&out_dir);
            let mut seen_cmx = std::collections::HashSet::new();
            for src in &parser_cfg.sources {
                let cmx_name = PathBuf::from(src)
                    .file_name()
                    .unwrap_or_default()
                    .to_os_string();
                let cmx = PathBuf::from(cmx_name).with_extension("cmx");
                if seen_cmx.insert(cmx.clone()) {
                    p_ar_cmd.arg(cmx);
                }
            }
            let status = p_ar_cmd.status()?;
            if !status.success() {
                return Err("Failed to create parser archive".into());
            }
            Status::finished(format!(
                "Built parser library {}",
                archive_dst.display()
            ));
        }

        Ok(())
    }

    fn compile_module(
        mod_name: &str,
        modules_dir: &Path,
        search_dirs: &[PathBuf],
        cache_dir: &Path,
        already_cached: &str,
        warn_err: Option<&str>,
        ocamlfind: &Path,
        packages: &str,
        include_args: &[String],
        env_map: &HashMap<String, String>,
        any_module_recompiled: &mut bool,
    ) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let ml_src = modules_dir.join(format!("{}.ml", mod_name));
        let cmx_dst = modules_dir.join(format!("{}.cmx", mod_name));

        let fst_dot_name = mod_name.replace('_', ".");
        let mut fst_source = None;
        for search_dir in search_dirs {
            let candidate = search_dir.join(format!("{}.fst", fst_dot_name));
            if candidate.is_file() {
                fst_source = Some(candidate);
                break;
            }
        }

        let needs_extract = !ml_src.is_file() || {
            if let Some(ref fst) = fst_source {
                match (fs::metadata(fst), fs::metadata(&ml_src)) {
                    (Ok(m_fst), Ok(m_ml)) => {
                        m_fst
                            .modified()
                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                            > m_ml
                                .modified()
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                    }
                    _ => false,
                }
            } else {
                false
            }
        };

        if needs_extract && let Some(ref fst) = fst_source {
            Self::extract_fstar_module(
                mod_name,
                &fst_dot_name,
                fst,
                modules_dir,
                search_dirs,
                cache_dir,
                already_cached,
                warn_err,
                env_map,
            )?;
        }

        let o_dst = cmx_dst.with_extension("o");
        let up_to_date =
            if !*any_module_recompiled && cmx_dst.is_file() && o_dst.is_file() {
                if let (Ok(m_src), Ok(m_dst), Ok(m_o)) = (
                    fs::metadata(&ml_src),
                    fs::metadata(&cmx_dst),
                    fs::metadata(&o_dst),
                ) {
                    if let (Ok(t_src), Ok(t_dst), Ok(t_o)) =
                        (m_src.modified(), m_dst.modified(), m_o.modified())
                    {
                        t_dst >= t_src && t_o >= t_src
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };

        if !up_to_date {
            *any_module_recompiled = true;
            Status::compiling(format!("Compiling OCaml module {}", mod_name));
            let mut cmd = std::process::Command::new(ocamlfind);
            cmd.arg("opt");
            if !packages.is_empty() {
                cmd.arg("-package").arg(packages);
            }
            for inc in include_args {
                cmd.arg(inc);
            }
            cmd.arg("-c").arg(&ml_src);
            for (k, v) in env_map {
                cmd.env(k, v);
            }
            let status = cmd.status()?;
            if !status.success() {
                return Err(format!(
                    "Failed to compile OCaml module `{}`",
                    ml_src.display()
                )
                .into());
            }
        }

        Ok(cmx_dst)
    }

    fn extract_fstar_module(
        mod_name: &str,
        fst_dot_name: &str,
        fst: &Path,
        modules_dir: &Path,
        search_dirs: &[PathBuf],
        cache_dir: &Path,
        already_cached: &str,
        warn_err: Option<&str>,
        env_map: &HashMap<String, String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let fstar_bin = env_map
            .get("FSTAR_EXE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("fstar.exe"));

        Status::compiling(format!("Extracting {} from {}", mod_name, fst.display()));

        let run_fstar = |args: &[&str]| -> (bool, String) {
            let mut cmd = std::process::Command::new(&fstar_bin);
            for search_dir in search_dirs {
                cmd.arg("--include").arg(search_dir);
            }
            if let Some(pulse_lib) = env_map.get("PULSE_LIB") {
                cmd.arg("--include").arg(pulse_lib);
            }
            cmd.arg("--include").arg(cache_dir);
            cmd.arg("--cache_dir").arg(cache_dir);
            if !already_cached.is_empty() {
                cmd.arg("--already_cached").arg(already_cached);
            }
            if let Some(we) = warn_err {
                cmd.arg("--warn_error").arg(we);
            }
            for a in args {
                cmd.arg(a);
            }
            cmd.arg(fst);
            match cmd.output() {
                Ok(out) => {
                    let stdout_str = String::from_utf8_lossy(&out.stdout);
                    let stderr_str = String::from_utf8_lossy(&out.stderr);
                    let full_output = format!("{}\n{}", stdout_str, stderr_str);
                    (out.status.success(), full_output)
                }
                Err(e) => (false, e.to_string()),
            }
        };

        // Strategy 1: Fast direct extraction
        let (mut success, mut err_output) = run_fstar(&[
            "--lax",
            "--codegen",
            "OCaml",
            "--extract_module",
            fst_dot_name,
            "--odir",
            &modules_dir.to_string_lossy(),
        ]);

        // Strategy 2: If extraction failed, parse and evict ONLY failing/stale caches and verify incrementally
        if !success {
            let fst_base = fst.file_name().unwrap_or_default().to_string_lossy();
            let _ = fs::remove_file(cache_dir.join(format!("{}.checked", fst_base)));
            let _ = fs::remove_file(
                cache_dir.join(format!("{}.fst.checked", fst_dot_name)),
            );

            for line in err_output.lines() {
                if line.contains(".checked")
                    && (line.contains("stale") || line.contains("mismatch"))
                {
                    for word in line.split_whitespace() {
                        let clean_word = word.trim_matches(|c| {
                            c == '\'' || c == '`' || c == '(' || c == ')' || c == ':'
                        });
                        if clean_word.ends_with(".checked") {
                            let path = PathBuf::from(clean_word);
                            let _ = fs::remove_file(&path);
                            if let Some(fname) = path.file_name() {
                                let _ = fs::remove_file(cache_dir.join(fname));
                            }
                        }
                    }
                }
            }

            Status::verifying(format!(
                "Refreshing stale cache for `{}` incrementally...",
                mod_name
            ));
            if let Ok(curr_exe) = std::env::current_exe() {
                let _ = std::process::Command::new(curr_exe).arg("verify").status();
            }
            let (retry_ok, retry_err) = run_fstar(&[
                "--lax",
                "--codegen",
                "OCaml",
                "--extract_module",
                fst_dot_name,
                "--odir",
                &modules_dir.to_string_lossy(),
            ]);
            success = retry_ok;
            err_output = retry_err;
        }

        if !success {
            return Err(format!(
                "F* extraction failed for `{}` across all defensive fallback strategies:\n{}",
                fst.display(),
                err_output
            )
            .into());
        }

        Ok(())
    }

    fn compile_single_source(
        source_path: &Path,
        kind: &str,
        ocamlfind: &Path,
        packages: &str,
        include_args: &[String],
        env_map: &HashMap<String, String>,
        force_recompile: bool,
    ) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let expanded = PathBuf::from(interpolate_variables(
            &source_path.to_string_lossy(),
            env_map,
        ));
        let cmx = expanded.with_extension("cmx");
        let o = expanded.with_extension("o");

        let up_to_date = !force_recompile
            && if cmx.is_file() && o.is_file() {
                if let (Ok(m_src), Ok(m_dst), Ok(m_o)) = (
                    fs::metadata(&expanded),
                    fs::metadata(&cmx),
                    fs::metadata(&o),
                ) {
                    if let (Ok(t_src), Ok(t_dst), Ok(t_o)) =
                        (m_src.modified(), m_dst.modified(), m_o.modified())
                    {
                        t_dst >= t_src && t_o >= t_src
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };

        if !up_to_date {
            Status::compiling(format!("Compiling {} {}", kind, expanded.display()));
            let mut cmd = std::process::Command::new(ocamlfind);
            cmd.arg("opt");
            if !packages.is_empty() {
                cmd.arg("-package").arg(packages);
            }
            for inc in include_args {
                cmd.arg(inc);
            }
            cmd.arg("-c").arg(&expanded);
            for (k, v) in env_map {
                cmd.env(k, v);
            }
            let status = cmd.status()?;
            if !status.success() {
                return Err(format!(
                    "Failed to compile {} `{}`",
                    kind,
                    expanded.display()
                )
                .into());
            }
        }

        Ok(cmx)
    }

    fn link_native_binary(
        cfg: &OCamlBuildConfig,
        ocamlfind: &Path,
        packages: &str,
        include_args: &[String],
        compiled_cmx: &[PathBuf],
        env_map: &HashMap<String, String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let output_expanded = PathBuf::from(interpolate_variables(
            &cfg.output.to_string_lossy(),
            env_map,
        ));
        if let Some(parent) = output_expanded.parent() {
            fs::create_dir_all(parent)?;
        }

        Status::compiling(format!(
            "Linking native OCaml binary {}",
            output_expanded.display()
        ));
        let mut link_cmd = std::process::Command::new(ocamlfind);
        link_cmd.arg("opt");
        let cc_driver = cfg
            .cc
            .as_deref()
            .or_else(|| env_map.get("CC").map(|s| s.as_str()))
            .unwrap_or("clang");
        link_cmd.arg("-cc").arg(cc_driver);
        link_cmd.arg("-linkpkg");
        if !packages.is_empty() {
            link_cmd.arg("-package").arg(packages);
        }
        for inc in include_args {
            link_cmd.arg(inc);
        }
        for lib in &cfg.libraries {
            let lib_path = interpolate_variables(&lib.to_string_lossy(), env_map);
            link_cmd.arg(lib_path);
        }
        for cmx in compiled_cmx {
            link_cmd.arg(cmx);
        }
        link_cmd.arg("-o").arg(&output_expanded);
        for (k, v) in env_map {
            link_cmd.env(k, v);
        }

        let status = link_cmd.status()?;
        if !status.success() {
            return Err(format!(
                "Linking failed for `{}` with status: {}",
                output_expanded.display(),
                status
            )
            .into());
        }

        if cfg.strip {
            Status::compiling(format!(
                "Stripping debug symbols from {}",
                output_expanded.display()
            ));
            valerie_driver::strip_binary(&output_expanded)?;
        }

        Status::finished(format!(
            "Built native binary {}",
            output_expanded.display()
        ));
        Ok(())
    }
}
