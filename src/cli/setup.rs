//! CLI setup and runtime maintenance commands.
//!
//! This file orchestrates interactive/non-interactive setup choices and calls
//! runner adapters for runner-specific behavior.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use agent_ruler::config::{
    load_runtime, runtime_projects_dir, save_config, RuntimeState, CONFIG_FILE_NAME,
};
use agent_ruler::helpers::ui::runtime_api::sync_selected_runner_telegram_bridges;
use agent_ruler::runners::claudecode::ClaudeCodeAdapter;
use agent_ruler::runners::openclaw::OpenClawAdapter;
use agent_ruler::runners::opencode::OpenCodeAdapter;
use agent_ruler::runners::{
    remove_configured_runner, IntegrationOption, IntegrationSelection, RunnerAdapter, RunnerKind,
};

const OPENCLAW_TOOLS_ADAPTER_INTEGRATION_ID: &str = "openclaw_tools_adapter";

/// Run interactive project setup and persist runner association.
pub fn run_setup(ruler_root: &Path, runtime_dir: Option<&Path>) -> Result<()> {
    let mut runtime = match load_runtime(ruler_root, runtime_dir) {
        Ok(runtime) => runtime,
        Err(_) => {
            println!("runtime is not initialized yet.");
            println!("run `agent-ruler init` first, then run `agent-ruler setup`.");
            return Ok(());
        }
    };

    println!();
    println!("setup: choose a runner");
    println!();
    let selected_runner = prompt_runner_selection()?;
    match selected_runner {
        RunnerKind::Openclaw => run_setup_with_adapter(&mut runtime, OpenClawAdapter::new()),
        RunnerKind::Claudecode => run_setup_with_adapter(&mut runtime, ClaudeCodeAdapter::new()),
        RunnerKind::Opencode => run_setup_with_adapter(&mut runtime, OpenCodeAdapter::new()),
    }
}

/// Remove project runner association and delete runtime-local managed paths.
pub fn run_runner_remove(
    ruler_root: &Path,
    runtime_dir: Option<&Path>,
    project_key: Option<&str>,
    runner_kind: RunnerKind,
) -> Result<()> {
    let mut runtime = load_runtime_target(ruler_root, runtime_dir, project_key)?;
    let configured = runtime.config.runner.clone();
    let removed = remove_configured_runner(&mut runtime, runner_kind)?;
    if !removed {
        println!("runner remove: no matching runner association found for this project");
        return Ok(());
    }

    if let Some(existing) = configured {
        println!(
            "runner removed: {} data deleted from {} and {}",
            existing.kind.display_name(),
            existing.managed_home.display(),
            existing.managed_workspace.display()
        );
    } else {
        println!("runner removed");
    }
    Ok(())
}

/// Delete a runtime directory (current project or explicit target).
pub fn run_purge(
    ruler_root: &Path,
    runtime_dir: Option<&Path>,
    project_key: Option<&str>,
    yes: bool,
) -> Result<()> {
    if !yes {
        return Err(anyhow!(
            "purge requires --yes to confirm runtime directory deletion"
        ));
    }

    let target = if let Some(key) = project_key {
        runtime_projects_dir().join(key)
    } else if let Some(runtime_dir) = runtime_dir {
        if runtime_dir.is_absolute() {
            runtime_dir.to_path_buf()
        } else {
            ruler_root.join(runtime_dir)
        }
    } else {
        let runtime = load_runtime(ruler_root, runtime_dir)
            .context("load runtime (run `agent-ruler init` first)")?;
        runtime.config.runtime_root
    };

    if !target.exists() {
        println!(
            "purge: runtime directory is already absent: {}",
            target.display()
        );
        return Ok(());
    }

    fs::remove_dir_all(&target).with_context(|| format!("remove {}", target.display()))?;
    println!("purge complete: {}", target.display());
    Ok(())
}

fn run_setup_with_adapter<A: RunnerAdapter>(runtime: &mut RuntimeState, adapter: A) -> Result<()> {
    println!("setup: selected {}", adapter.display_name());
    println!();

    let mut host_install = adapter.detect_host_install(None)?;
    let mut import_from_host = false;

    if adapter.kind() == RunnerKind::Openclaw {
        if let Some(found) = host_install.as_ref() {
            println!(
                "setup: detected host OpenClaw home via {} at {}",
                found.detected_by,
                found.home.display()
            );
        } else {
            println!(
                "setup: host OpenClaw home was not auto-detected (optional import can be skipped)"
            );
        }
        println!();

        import_from_host = prompt_yes_no(
            "Import channel settings from host OpenClaw into this project?",
            false,
        )?;
        if import_from_host && host_install.is_none() {
            let manual_path =
                prompt_optional_path("Host OpenClaw home path (leave blank to skip import): ")?;
            if let Some(path) = manual_path {
                host_install = adapter.detect_host_install(Some(path.as_path()))?;
                if host_install.is_none() {
                    println!("setup: path did not look like an OpenClaw home; skipping import");
                    import_from_host = false;
                }
            } else {
                import_from_host = false;
            }
        }
    } else if let Some(found) = host_install.as_ref() {
        println!(
            "setup: detected runner binary via {} at {}",
            found.detected_by,
            found.home.display()
        );
    } else {
        println!(
            "setup: runner binary `{}` was not found in PATH during setup",
            adapter.kind().executable_name()
        );
        println!(
            "setup: runner association is still saved; install runner and re-run setup if needed"
        );
    }

    if adapter.kind() != RunnerKind::Openclaw {
        println!();
        println!("setup note: channel wiring can be configured separately in Control Settings.");
        println!();
    } else {
        println!();
    }

    let integration_choices = if adapter.kind() == RunnerKind::Openclaw {
        let auto_selected = adapter
            .integration_options()
            .iter()
            .filter(|option| option.id == OPENCLAW_TOOLS_ADAPTER_INTEGRATION_ID)
            .map(|option| IntegrationSelection::new(option.id))
            .collect::<Vec<_>>();
        if !auto_selected.is_empty() {
            println!("setup: OpenClaw tools adapter integration enabled automatically.");
        }
        auto_selected
    } else {
        prompt_integrations(adapter.integration_options())?
    };
    let paths = adapter.provision_project_paths(runtime)?;
    // Import is read-only from host side; all writes happen under managed paths.
    let import_report =
        adapter.optional_import_from_host(host_install.as_ref(), &paths, import_from_host)?;

    let mut config = runtime.config.clone();
    adapter.write_runner_config(
        runtime,
        &mut config,
        &paths,
        &import_report,
        &integration_choices,
    )?;
    let config_path = config.state_dir.join(CONFIG_FILE_NAME);
    save_config(&config_path, &config)
        .with_context(|| format!("persist setup config at {}", config_path.display()))?;
    runtime.config = config;
    adapter.validate(&runtime.config)?;
    if let Err(err) = sync_selected_runner_telegram_bridges(runtime, true, true) {
        eprintln!(
            "setup: runner bridge sync warning: unable to align managed Telegram bridges: {err}"
        );
    }

    if import_report.imported {
        println!(
            "setup: imported {} item(s) into Ruler-managed {} home",
            import_report.copied_items.len(),
            adapter.display_name()
        );
        if let Some(snapshot) = import_report.snapshot_path {
            println!("setup: import snapshot: {}", snapshot.display());
        }
    }
    if !import_report.imported {
        println!("setup: import skipped");
    }

    println!();
    adapter.print_next_steps(runtime, &runtime.config);
    Ok(())
}

fn load_runtime_target(
    ruler_root: &Path,
    runtime_dir: Option<&Path>,
    project_key: Option<&str>,
) -> Result<RuntimeState> {
    if let Some(project_key) = project_key {
        let runtime_root = runtime_projects_dir().join(project_key);
        return load_runtime(ruler_root, Some(runtime_root.as_path())).with_context(|| {
            format!(
                "load runtime for project key {} at {}",
                project_key,
                runtime_root.display()
            )
        });
    }

    load_runtime(ruler_root, runtime_dir).context("load runtime (run `agent-ruler init` first)")
}

fn prompt_runner_selection() -> Result<RunnerKind> {
    println!("1) OpenClaw");
    println!("2) Claude Code");
    println!("3) OpenCode");
    // Non-interactive setup must be deterministic; default to the only
    // baseline runner kind rather than failing on missing TTY.
    if !io::stdin().is_terminal() {
        println!("setup: non-interactive input detected; defaulting to OpenClaw");
        return Ok(RunnerKind::Openclaw);
    }

    loop {
        let input = read_line("Runner selection [1]: ")?;
        match input.trim() {
            "" | "1" => return Ok(RunnerKind::Openclaw),
            "2" => return Ok(RunnerKind::Claudecode),
            "3" => return Ok(RunnerKind::Opencode),
            _ => println!("invalid choice; enter 1, 2, or 3"),
        }
    }
}

fn prompt_yes_no(prompt: &str, default_yes: bool) -> Result<bool> {
    let suffix = if default_yes { "[Y/n]" } else { "[y/N]" };
    if !io::stdin().is_terminal() {
        return Ok(default_yes);
    }

    loop {
        let raw = read_line(&format!("{prompt} {suffix}: "))?;
        let value = raw.trim().to_ascii_lowercase();
        if value.is_empty() {
            return Ok(default_yes);
        }
        match value.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("please answer y or n"),
        }
    }
}

fn prompt_optional_path(prompt: &str) -> Result<Option<PathBuf>> {
    if !io::stdin().is_terminal() {
        return Ok(None);
    }
    let raw = read_line(prompt)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(trimmed)))
}

fn prompt_integrations(options: &[IntegrationOption]) -> Result<Vec<IntegrationSelection>> {
    if options.is_empty() || !io::stdin().is_terminal() {
        return Ok(Vec::new());
    }

    println!("setup: optional integrations");
    for (index, option) in options.iter().enumerate() {
        println!("{}) {} - {}", index + 1, option.label, option.detail);
    }
    println!("Enter comma-separated numbers (for example: 1,2), or press Enter to skip.");

    loop {
        let raw = read_line("Integration selections: ")?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        let mut picks = BTreeSet::new();
        let mut valid = true;
        for token in trimmed.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            let parsed = match token.parse::<usize>() {
                Ok(value) => value,
                Err(_) => {
                    valid = false;
                    break;
                }
            };
            if parsed == 0 || parsed > options.len() {
                valid = false;
                break;
            }
            picks.insert(parsed - 1);
        }

        if !valid {
            println!("invalid selection; use comma-separated numbers from the list");
            continue;
        }

        let selected = picks
            .into_iter()
            .map(|idx| IntegrationSelection::new(options[idx].id))
            .collect();
        return Ok(selected);
    }
}

fn read_line(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush().context("flush stdout")?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("read stdin line")?;
    Ok(input)
}
