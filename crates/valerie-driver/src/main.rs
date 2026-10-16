use clap::{Arg, CommandFactory, FromArgMatches};
use std::path::PathBuf;
use std::time::Instant;

use valerie_driver::BackendMode as DriverBackendMode;
use valerie_driver::terminal::{Status, Style};
use valerie_manifest::{Manifest, TaskDef, resolver::ResolvedEnvironment};

mod cli;
use cli::{
    Cli, execute_command, execute_manifest_task, parse_target, print_verbose_header,
};

fn main() {
    if let Err(e) = run() {
        Status::error(format!("{}", e));
        if std::env::var("RUST_BACKTRACE")
            .map(|v| v != "0")
            .unwrap_or(false)
        {
            eprintln!(
                "
Backtrace:
{:?}",
                e
            );
        }
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Early color & quiet probe from raw args
    for arg in std::env::args() {
        if arg.starts_with("--color=") {
            if let Ok(choice) = arg
                .trim_start_matches("--color=")
                .parse::<valerie_driver::terminal::ColorChoice>()
            {
                valerie_driver::terminal::set_color_choice(choice);
            }
        } else if arg == "-q" || arg == "--quiet" {
            valerie_driver::terminal::set_quiet(true);
        }
    }

    // Phase 1: Pre-Parse Manifest Probe
    let manifest_ctx = Manifest::probe_current_dir();

    // Check if top-level help is requested
    let raw_args: Vec<String> = std::env::args().collect();
    let is_help = raw_args.iter().skip(1).all(|a| a.starts_with('-'))
        && raw_args.iter().any(|a| a == "--help" || a == "-h")
        || (raw_args.len() == 2 && raw_args[1] == "help");

    if is_help {
        let use_color = valerie_driver::terminal::colors_enabled();
        let color_choice = if use_color {
            clap::ColorChoice::Always
        } else {
            clap::ColorChoice::Never
        };
        let mut base_cmd = Cli::command().color(color_choice);
        let styled = base_cmd.render_help();
        let help_str = if use_color {
            styled.ansi().to_string()
        } else {
            styled.to_string()
        };

        if let Some((_, manifest)) = manifest_ctx.as_ref() {
            // Find where Options section begins
            let options_marker = if use_color {
                let formatted_needle = format!("Options:{}\n", Style::RESET);
                help_str
                    .find(&formatted_needle)
                    .or_else(|| help_str.find("Options:\n"))
            } else {
                help_str.find("Options:\n")
            };

            if let Some(pos) = options_marker {
                let (commands_part, options_part) = help_str.split_at(pos);
                print!("{}", commands_part);
                if use_color {
                    println!(
                        "{}Workspace Tasks (defined in valerie.toml):{}",
                        Style::GREEN_BOLD,
                        Style::RESET
                    );
                } else {
                    println!("Workspace Tasks (defined in valerie.toml):");
                }
                let mut sorted_tasks: Vec<(&String, &TaskDef)> =
                    manifest.tasks.iter().collect();
                sorted_tasks.sort_by_key(|(name, _)| name.as_str());
                for (name, def) in sorted_tasks {
                    let desc = def
                        .description
                        .as_deref()
                        .unwrap_or("Project task from valerie.toml");
                    if use_color {
                        println!(
                            "  {}{:14}{}{}",
                            Style::CYAN_BOLD,
                            name,
                            Style::RESET,
                            desc
                        );
                    } else {
                        println!("  {:14}{}", name, desc);
                    }
                }
                println!();
                print!("{}", options_part);
                return Ok(());
            }
        } else {
            print!("{}", help_str);
            return Ok(());
        }
    }

    // Phase 2: Programmatic Clap Command Tree Synthesis
    let clap_color = if valerie_driver::terminal::colors_enabled() {
        clap::ColorChoice::Always
    } else {
        clap::ColorChoice::Never
    };
    let mut cmd = Cli::command()
        .color(clap_color)
        .allow_external_subcommands(true);
    let mut dynamic_tasks: std::collections::HashMap<String, TaskDef> =
        std::collections::HashMap::new();

    if let Some((_, manifest)) = manifest_ctx.as_ref() {
        for (task_name, task_def) in &manifest.tasks {
            dynamic_tasks.insert(task_name.clone(), task_def.clone());
        }

        let mut sorted_tasks: Vec<(&String, &TaskDef)> =
            manifest.tasks.iter().collect();
        sorted_tasks.sort_by_key(|(name, _)| name.as_str());

        for (task_name, task_def) in sorted_tasks {
            if cmd.find_subcommand(task_name).is_some() {
                continue;
            }
            let static_name: &'static str =
                Box::leak(task_name.clone().into_boxed_str());
            let desc_str = task_def
                .description
                .clone()
                .unwrap_or_else(|| "Project task from valerie.toml".to_string());
            let static_desc: &'static str = Box::leak(desc_str.into_boxed_str());
            let sub = clap::Command::new(static_name).about(static_desc).arg(
                Arg::new("args")
                    .num_args(0..)
                    .trailing_var_arg(true)
                    .allow_hyphen_values(true)
                    .help("Arguments forwarded directly to the task runner"),
            );
            cmd = cmd.subcommand(sub);
        }
    }

    let matches = cmd.get_matches();

    // Check if a dynamic manifest task was matched (only if not a built-in core subcommand)
    if let Some((sub_name, sub_matches)) = matches.subcommand() {
        let is_core = matches!(
            sub_name,
            "emit"
                | "build"
                | "b"
                | "verify"
                | "v"
                | "extract"
                | "clean"
                | "init"
                | "header"
                | "audit"
                | "diff"
                | "bench"
                | "bundle"
                | "test"
                | "t"
                | "run"
                | "r"
                | "fuzz"
                | "vale"
                | "check"
                | "c"
                | "completions"
        );
        if !is_core && let Some(task) = dynamic_tasks.get(sub_name) {
            let forwarded_args: Vec<&str> = sub_matches
                .try_get_many::<String>("args")
                .ok()
                .flatten()
                .map(|v| v.map(|s| s.as_str()).collect())
                .unwrap_or_default();
            return execute_manifest_task(
                sub_name,
                task,
                &forwarded_args,
                &dynamic_tasks,
                manifest_ctx.as_ref().map(|(_, m)| m),
            );
        }
    }

    // Check if an external plugin was matched (valerie-<subcommand>)
    if let Some((sub_name, sub_matches)) = matches.subcommand() {
        let is_core = matches!(
            sub_name,
            "emit"
                | "build"
                | "b"
                | "verify"
                | "v"
                | "extract"
                | "clean"
                | "init"
                | "header"
                | "audit"
                | "diff"
                | "bench"
                | "bundle"
                | "test"
                | "t"
                | "run"
                | "r"
                | "fuzz"
                | "vale"
                | "check"
                | "c"
                | "completions"
        );
        if !is_core {
            let plugin_binary = format!("valerie-{}", sub_name);
            let forwarded_args: Vec<String> = sub_matches
                .get_many::<std::ffi::OsString>("")
                .map(|v| v.map(|s| s.to_string_lossy().to_string()).collect())
                .unwrap_or_default();

            match std::process::Command::new(&plugin_binary)
                .args(&forwarded_args)
                .status()
            {
                Ok(status) => {
                    if !status.success() {
                        return Err(format!(
                            "Plugin '{}' exited with status {}",
                            plugin_binary, status
                        )
                        .into());
                    }
                    return Ok(());
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(format!(
                        "no such subcommand: `{}`\n\n        Run `valerie --help` for available commands",
                        sub_name
                    )
                    .into());
                }
                Err(e) => {
                    return Err(format!(
                        "Failed to run external plugin '{}': {}",
                        plugin_binary, e
                    )
                    .into());
                }
            }
        }
    }

    let cli = Cli::from_arg_matches(&matches)?;

    // Initialize Cargo-style terminal preferences
    if let Ok(choice) = cli.color.parse::<valerie_driver::terminal::ColorChoice>() {
        valerie_driver::terminal::set_color_choice(choice);
    }
    if cli.quiet {
        valerie_driver::terminal::set_quiet(true);
    }

    let start_time = Instant::now();

    let has_explicit_target = cli.target.is_some();
    let host_default = valerie_target::TargetDescriptor::host();
    let mut target = if let Some(t) = cli.target.as_deref() {
        parse_target(t)
    } else {
        host_default
    };

    // Inherit default target hardware features from valerie.toml if in a workspace
    if let Some((_, manifest)) = manifest_ctx.as_ref() {
        let triple_key = target.triple();
        let arch_key = format!("{:?}", target.architecture).to_lowercase();
        if let Some(target_cfg) = manifest
            .target
            .get(&triple_key)
            .or_else(|| manifest.target.get(&arch_key))
        {
            if target_cfg.dit {
                target = target.with_dit();
            }
            if target_cfg.features.iter().any(|f| {
                f.contains("crypto") || f.contains("aes") || f.contains("zk")
            }) {
                target = target.with_crypto();
            }
        }
    }

    if cli.crypto {
        target = target.with_crypto();
    }
    if cli.dit {
        target = target.with_dit();
    }
    let backend_mode: DriverBackendMode = cli.backend.into();

    // Uniform toolchain resolution using manifest and CLI overrides
    let mut resolved_env =
        if let Some((ref manifest_path, ref manifest)) = manifest_ctx {
            ResolvedEnvironment::resolve_with_manifest_and_base(
                manifest,
                manifest_path.parent(),
            )
        } else {
            ResolvedEnvironment::resolve()
        };
    if let Some(ref custom_vale) = cli.vale_bin {
        resolved_env.vale_exe = Some(custom_vale.clone());
    }

    print_verbose_header(&target, backend_mode, &cli, &resolved_env);

    let vale_bin = resolved_env
        .vale_exe
        .clone()
        .unwrap_or_else(|| PathBuf::from("vale"));

    execute_command(
        &cli,
        target,
        has_explicit_target,
        backend_mode,
        vale_bin,
        start_time,
        manifest_ctx.as_ref(),
        Some(&resolved_env),
    )
}
