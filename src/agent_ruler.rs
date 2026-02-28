//! Agent Ruler CLI entrypoint and command orchestration.
//!
//! This binary delegates policy decisions to the library crate and keeps CLI
//! control-flow semantics (interactive behavior, detached gateway lifecycle).

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

use ::agent_ruler::approvals::ApprovalStore;
use ::agent_ruler::config::{
    detect_ruler_root, init_layout, load_runtime, reset_layout, save_config, RuntimeState,
    CONFIG_FILE_NAME,
};
use ::agent_ruler::embedded_bridge::ensure_embedded_bridge_assets;
use ::agent_ruler::helpers::approvals::append_approval_resolution_receipt;
use ::agent_ruler::helpers::commands::ui::{stop_ui_processes_in_projects_root, UiPidGuard};
use ::agent_ruler::helpers::maybe_apply_approval_effect;
use ::agent_ruler::openclaw_bridge::{ensure_generated_config, generated_config_path};
use ::agent_ruler::policy::PolicyEngine;
use ::agent_ruler::receipts::ReceiptStore;
use ::agent_ruler::runner::run_confined;
use ::agent_ruler::runners::openclaw::{
    enforce_session_memory_hook_guard, enforce_tools_adapter_config_guard,
    find_managed_gateway_listener_pid, inspect_managed_telegram_config,
    maybe_collect_gateway_port_diagnostics,
};
use ::agent_ruler::runners::{
    apply_runner_env_overrides, configured_runner_targets_command,
    reconcile_runner_executable_with_options, RunnerAvailabilityState, RunnerCheckOptions,
    RunnerKind,
};
use ::agent_ruler::staged_exports::{StagedExportState, StagedExportStore};
use ::agent_ruler::ui;

use crate::cli::{
    resolve_approval_targets, run_delivery, run_export, run_import, run_manual_smoke, run_purge,
    run_runner_remove, run_setup, run_update, run_wait_for_approval,
};

#[derive(Parser, Debug)]
#[command(
    name = "agent-ruler",
    version,
    about = "Deterministic reference monitor + confinement runner for AI agents"
)]
struct Cli {
    #[arg(long, global = true)]
    runtime_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Init {
        #[arg(long)]
        workspace: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    Setup,
    Run {
        #[arg(long)]
        background: bool,
        #[arg(long, hide = true)]
        foreground: bool,
        #[arg(required = true, trailing_var_arg = true)]
        cmd: Vec<String>,
    },
    Status {
        #[arg(long)]
        json: bool,
    },
    Tail {
        #[arg(default_value_t = 50)]
        lines: usize,
    },
    Approve {
        #[arg(long)]
        id: Option<String>,
        #[arg(long, value_enum, default_value_t = ApprovalDecisionArg::List)]
        decision: ApprovalDecisionArg,
        #[arg(long)]
        all: bool,
    },
    ResetExec {
        #[arg(long)]
        yes: bool,
    },
    Reset {
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        keep_config: bool,
    },
    Ui {
        #[arg(long)]
        bind: Option<String>,
        #[command(subcommand)]
        action: Option<UiAction>,
    },
    Export {
        src: PathBuf,
        dst: PathBuf,
        #[arg(long)]
        preview_only: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        bypass: bool,
        #[arg(long)]
        i_understand_bypass_risk: bool,
    },
    Deliver {
        staged: PathBuf,
        destination: Option<PathBuf>,
        #[arg(long)]
        preview_only: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        move_artifact: bool,
        #[arg(long)]
        bypass: bool,
        #[arg(long)]
        i_understand_bypass_risk: bool,
    },
    Import {
        src: PathBuf,
        dst: Option<PathBuf>,
        #[arg(long)]
        preview_only: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        bypass: bool,
        #[arg(long)]
        i_understand_bypass_risk: bool,
    },
    Smoke {
        #[arg(long)]
        non_interactive: bool,
    },
    /// Wait for an approval decision (useful for agents to poll without failing)
    Wait {
        /// Approval ID to wait for
        #[arg(long)]
        id: String,
        /// Timeout in seconds (default 60)
        #[arg(long, default_value_t = 60)]
        timeout: u64,
        /// Output format
        #[arg(long)]
        json: bool,
    },
    Runner {
        #[command(subcommand)]
        command: RunnerCommands,
    },
    Purge {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    Update {
        #[arg(long)]
        check: bool,
        #[arg(long)]
        version: Option<String>,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, hide = true)]
        from_ui: bool,
    },
}

#[derive(Subcommand, Debug)]
enum UiAction {
    Stop,
}

#[derive(Subcommand, Debug)]
enum RunnerCommands {
    Remove {
        runner: RunnerArg,
        #[arg(long)]
        project: Option<String>,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum ApprovalDecisionArg {
    List,
    Approve,
    Deny,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum RunnerArg {
    Openclaw,
}

impl From<RunnerArg> for RunnerKind {
    fn from(value: RunnerArg) -> Self {
        match value {
            RunnerArg::Openclaw => RunnerKind::Openclaw,
        }
    }
}

/// Parse CLI args and dispatch command handlers.
pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    // Use detect_ruler_root() which derives from binary location, not cwd
    let ruler_root = detect_ruler_root();
    ensure_embedded_bridge_assets(&ruler_root).context("prepare embedded bridge assets")?;
    let runtime_dir = cli.runtime_dir.clone();

    match cli.command {
        Commands::Init { workspace, force } => {
            let config = init_layout(&ruler_root, runtime_dir.as_deref(), workspace, force)?;
            println!("initialized Agent Ruler");
            println!("ruler_root: {}", config.ruler_root.display());
            println!("runtime: {}", config.runtime_root.display());
            println!("workspace: {}", config.workspace.display());
            println!("shared-zone: {}", config.shared_zone_dir.display());
            println!(
                "default-user-destination: {}",
                config.default_delivery_dir.display()
            );
            println!("state: {}", config.state_dir.display());
            Ok(())
        }
        Commands::Setup => run_setup(&ruler_root, runtime_dir.as_deref()),
        Commands::Run {
            background,
            foreground,
            cmd,
        } => {
            let mut runtime = load_runtime(&ruler_root, runtime_dir.as_deref())
                .context("load runtime (run `agent-ruler init` first)")?;

            if is_openclaw_gateway_stop(&cmd) {
                let gateway_stopped = stop_managed_background_gateway(&runtime)?;
                let bridge_stopped = stop_managed_openclaw_bridge(&runtime)?;
                if !(gateway_stopped && bridge_stopped) {
                    return Err(anyhow!(
                        "gateway stop failed: one or more managed processes are still running (see log for details)"
                    ));
                }
                return Ok(());
            }

            let targets_runner = configured_runner_targets_command(&runtime, &cmd);
            let runner_status = reconcile_runner_executable_with_options(
                &mut runtime,
                "run",
                RunnerCheckOptions::default(),
            )?;
            if targets_runner
                && matches!(
                    runner_status,
                    RunnerAvailabilityState::MissingUnresolved
                        | RunnerAvailabilityState::MissingKept
                        | RunnerAvailabilityState::MissingReconfigure
                )
            {
                return Err(anyhow!(
                    "runner `{}` is not available; install it or run `agent-ruler setup` / `agent-ruler runner remove openclaw`",
                    RunnerKind::Openclaw.executable_name()
                ));
            }

            if is_openclaw_gateway_launch(&cmd) {
                maybe_auto_configure_ui_bind_for_tailscale(&mut runtime);
            }

            if is_openclaw_command(&cmd) {
                match enforce_tools_adapter_config_guard(&runtime) {
                    Ok(true) => {
                        eprintln!(
                            "runner config guard: restored Agent Ruler tools adapter wiring in managed OpenClaw config."
                        );
                    }
                    Ok(false) => {}
                    Err(err) => {
                        eprintln!(
                            "runner config guard: unable to enforce tools adapter wiring: {err}"
                        );
                    }
                }

                ensure_openclaw_preflight_api_ready(&runtime, &cmd)?;
            }

            if is_openclaw_gateway_launch(&cmd) {
                let managed_home = managed_openclaw_home(&runtime);
                match enforce_session_memory_hook_guard(&managed_home) {
                    Ok(true) => {
                        eprintln!("gateway config guard: disabled session-memory hook for non-anthropic model defaults.");
                    }
                    Ok(false) => {}
                    Err(err) => {
                        eprintln!(
                            "gateway config guard: unable to apply managed config guard: {err}"
                        );
                    }
                }
                print_gateway_telegram_hints(&runtime);
                match maybe_start_managed_openclaw_bridge(&runtime)? {
                    BridgeStartupState::NotRequired => {
                        eprintln!(
                            "bridge diagnostics: no bridge routes discovered; skipping approvals hook sync."
                        );
                    }
                    BridgeStartupState::Active => {
                        configure_managed_openclaw_approvals_hook(&runtime)
                            .context("configure managed approvals hook after bridge startup")?;
                    }
                }
            }

            let normalized_cmd = normalize_openclaw_gateway_launch_command(&cmd);

            // `openclaw gateway` defaults to detached mode unless caller
            // explicitly forces foreground. This preserves prior UX where
            // gateway command returns while service keeps running.
            if is_openclaw_gateway_launch(&cmd) && !foreground {
                return spawn_background_run(&runtime, &normalized_cmd);
            }

            let engine =
                PolicyEngine::new(runtime.policy.clone(), runtime.config.workspace.clone());
            let approvals = ApprovalStore::new(&runtime.config.approvals_file);
            let receipts = ReceiptStore::new(&runtime.config.receipts_file);

            if background {
                return spawn_background_run(&runtime, &normalized_cmd);
            }

            let effective_cmd = apply_runner_env_overrides(&runtime, &normalized_cmd);
            let run = run_confined(&effective_cmd, &runtime, &engine, &approvals, &receipts)?;
            if run.exit_code != 0 {
                print_openclaw_gateway_port_diagnostics(&runtime, &cmd, &run.stdout, &run.stderr);
                print_openclaw_gateway_telegram_diagnostics(
                    &runtime,
                    &cmd,
                    &run.stdout,
                    &run.stderr,
                );
                std::process::exit(run.exit_code);
            }
            Ok(())
        }
        Commands::Status { json } => {
            let runtime = load_runtime_with_runner_preflight(
                &ruler_root,
                runtime_dir.as_deref(),
                "status",
                RunnerCheckOptions {
                    allow_prompt: !json,
                    emit_to_stderr: json,
                },
            )?;
            let approvals = ApprovalStore::new(&runtime.config.approvals_file);
            let receipts = ReceiptStore::new(&runtime.config.receipts_file);
            let staged = StagedExportStore::new(&runtime.config.staged_exports_file);

            let pending = approvals.list_pending().unwrap_or_default();
            let all_staged = staged.list().unwrap_or_default();
            let staged_ready = all_staged
                .iter()
                .filter(|r| r.state == StagedExportState::Staged)
                .count();
            let delivered = all_staged
                .iter()
                .filter(|r| r.state == StagedExportState::Delivered)
                .count();
            let tail = receipts.tail(1).unwrap_or_default();
            let runner_info = runtime.config.runner.as_ref().map(|runner| {
                serde_json::json!({
                    "kind": format!("{:?}", runner.kind).to_lowercase(),
                    "managed_home": runner.managed_home,
                    "managed_workspace": runner.managed_workspace,
                    "integrations": runner.integrations,
                    "missing_executable": runner.missing.executable_missing,
                    "missing_decision": runner.missing.decision,
                })
            });

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "profile": runtime.policy.profile,
                        "policy_version": runtime.policy.version,
                        "policy_hash": runtime.policy_hash,
                        "ruler_root": runtime.config.ruler_root,
                        "runtime_root": runtime.config.runtime_root,
                        "workspace": runtime.config.workspace,
                        "shared_zone": runtime.config.shared_zone_dir,
                        "default_delivery_dir": runtime.config.default_delivery_dir,
                        "state_dir": runtime.config.state_dir,
                        "pending_approvals": pending.len(),
                        "staged_exports": staged_ready,
                        "delivered_exports": delivered,
                        "last_receipt": tail.first(),
                        "ui": format!("http://{}", runtime.config.ui_bind),
                        "runner": runner_info,
                    }))?
                );
            } else {
                println!("profile: {}", runtime.policy.profile);
                println!("policy version: {}", runtime.policy.version);
                println!("policy hash: {}", runtime.policy_hash);
                println!("ruler root: {}", runtime.config.ruler_root.display());
                println!("runtime root: {}", runtime.config.runtime_root.display());
                println!("workspace: {}", runtime.config.workspace.display());
                println!("shared zone: {}", runtime.config.shared_zone_dir.display());
                println!(
                    "default user destination: {}",
                    runtime.config.default_delivery_dir.display()
                );
                println!("state: {}", runtime.config.state_dir.display());
                println!("pending approvals: {}", pending.len());
                println!("staged exports: {}", staged_ready);
                println!("delivered exports: {}", delivered);
                println!("ui: http://{}", runtime.config.ui_bind);
                if let Some(runner) = runtime.config.runner.as_ref() {
                    println!("runner: {}", runner.kind.display_name());
                    println!("runner managed home: {}", runner.managed_home.display());
                    println!(
                        "runner managed workspace: {}",
                        runner.managed_workspace.display()
                    );
                    if runner.missing.executable_missing {
                        println!(
                            "runner executable missing: true ({:?})",
                            runner.missing.decision
                        );
                    }
                } else {
                    println!("runner: none");
                }
                if let Some(last) = tail.first() {
                    println!(
                        "last receipt: {} {:?} {:?}",
                        last.timestamp, last.action.kind, last.decision.verdict
                    );
                }
            }
            Ok(())
        }
        Commands::Tail { lines } => {
            let runtime = load_runtime_with_runner_preflight(
                &ruler_root,
                runtime_dir.as_deref(),
                "tail",
                RunnerCheckOptions::default(),
            )?;
            let receipts = ReceiptStore::new(&runtime.config.receipts_file);
            for item in receipts.tail(lines)? {
                println!("{}", serde_json::to_string_pretty(&item)?);
            }
            Ok(())
        }
        Commands::Approve { id, decision, all } => {
            let runtime = load_runtime_with_runner_preflight(
                &ruler_root,
                runtime_dir.as_deref(),
                "approve",
                RunnerCheckOptions::default(),
            )?;
            let approvals = ApprovalStore::new(&runtime.config.approvals_file);
            let receipts = ReceiptStore::new(&runtime.config.receipts_file);

            match decision {
                ApprovalDecisionArg::List => {
                    for approval in approvals.list_pending()? {
                        println!(
                            "{} | {:?} | {} | {}",
                            approval.id, approval.reason, approval.action.operation, approval.note
                        );
                    }
                    Ok(())
                }
                ApprovalDecisionArg::Approve => {
                    let targets = resolve_approval_targets(&approvals, id, all)?;
                    if targets.is_empty() {
                        println!("no approvals matched");
                        return Ok(());
                    }
                    for target in targets {
                        let update = approvals.approve_idempotent(&target)?;
                        if update.changed {
                            append_approval_resolution_receipt(
                                &receipts,
                                &runtime,
                                &update.approval,
                                "approval-resolution-cli-approve",
                            )?;
                            println!("approved {}", update.approval.id);
                            maybe_apply_approval_effect(&runtime, &update.approval, &receipts)?;
                        } else {
                            println!("already approved {}", update.approval.id);
                        }
                    }
                    Ok(())
                }
                ApprovalDecisionArg::Deny => {
                    let targets = resolve_approval_targets(&approvals, id, all)?;
                    if targets.is_empty() {
                        println!("no approvals matched");
                        return Ok(());
                    }
                    for target in targets {
                        let update = approvals.deny_idempotent(&target)?;
                        if update.changed {
                            append_approval_resolution_receipt(
                                &receipts,
                                &runtime,
                                &update.approval,
                                "approval-resolution-cli-deny",
                            )?;
                            println!("denied {}", update.approval.id);
                        } else {
                            println!("already denied {}", update.approval.id);
                        }
                    }
                    Ok(())
                }
            }
        }
        Commands::ResetExec { yes } => {
            let runtime = load_runtime(&ruler_root, runtime_dir.as_deref())?;
            if !yes {
                return Err(anyhow!(
                    "reset-exec requires --yes to confirm ephemeral execution artifacts reset"
                ));
            }
            if runtime.config.exec_layer_dir.exists() {
                fs::remove_dir_all(&runtime.config.exec_layer_dir).with_context(|| {
                    format!("remove {}", runtime.config.exec_layer_dir.display())
                })?;
            }
            fs::create_dir_all(&runtime.config.exec_layer_dir)
                .with_context(|| format!("recreate {}", runtime.config.exec_layer_dir.display()))?;
            println!(
                "ephemeral execution artifacts reset: {} (workspace/policy untouched)",
                runtime.config.exec_layer_dir.display()
            );
            Ok(())
        }
        Commands::Reset { yes, keep_config } => {
            if !yes {
                return Err(anyhow!("reset requires --yes to confirm runtime reset"));
            }

            let config = reset_layout(&ruler_root, runtime_dir.as_deref(), keep_config)?;
            println!("runtime reset complete");
            println!("runtime: {}", config.runtime_root.display());
            println!("workspace: {}", config.workspace.display());
            println!("shared-zone: {}", config.shared_zone_dir.display());
            println!("state: {}", config.state_dir.display());
            if keep_config {
                println!("config impact: preserved existing config + policy");
            } else {
                println!("config impact: restored default config + policy");
            }
            Ok(())
        }
        Commands::Ui { bind, action } => {
            let mut runtime = load_runtime_with_runner_preflight(
                &ruler_root,
                runtime_dir.as_deref(),
                "ui",
                RunnerCheckOptions::default(),
            )?;
            match action {
                Some(UiAction::Stop) => stop_ui_action(&runtime),
                None => {
                    let bind = match bind {
                        Some(value) => value,
                        None => {
                            maybe_auto_configure_ui_bind_for_tailscale(&mut runtime);
                            runtime.config.ui_bind.clone()
                        }
                    };
                    run_ui_server(runtime, bind).await
                }
            }
        }
        Commands::Export {
            src,
            dst,
            preview_only,
            force,
            bypass,
            i_understand_bypass_risk,
        } => {
            let runtime = load_runtime_with_runner_preflight(
                &ruler_root,
                runtime_dir.as_deref(),
                "export",
                RunnerCheckOptions::default(),
            )?;
            run_export(
                &runtime,
                &src,
                &dst,
                preview_only,
                force,
                bypass,
                i_understand_bypass_risk,
                "export-cli",
            )
        }
        Commands::Deliver {
            staged,
            destination,
            preview_only,
            force,
            move_artifact,
            bypass,
            i_understand_bypass_risk,
        } => {
            let runtime = load_runtime_with_runner_preflight(
                &ruler_root,
                runtime_dir.as_deref(),
                "deliver",
                RunnerCheckOptions::default(),
            )?;
            run_delivery(
                &runtime,
                &staged,
                destination.as_deref(),
                preview_only,
                force,
                move_artifact,
                bypass,
                i_understand_bypass_risk,
                "deliver-cli",
            )
        }
        Commands::Import {
            src,
            dst,
            preview_only,
            force,
            bypass,
            i_understand_bypass_risk,
        } => {
            let runtime = load_runtime_with_runner_preflight(
                &ruler_root,
                runtime_dir.as_deref(),
                "import",
                RunnerCheckOptions::default(),
            )?;
            run_import(
                &runtime,
                &src,
                dst.as_deref(),
                preview_only,
                force,
                bypass,
                i_understand_bypass_risk,
                "import-cli",
            )
        }
        Commands::Smoke { non_interactive } => {
            run_manual_smoke(&ruler_root, runtime_dir.as_deref(), non_interactive)
        }
        Commands::Wait { id, timeout, json } => {
            run_wait_for_approval(&ruler_root, runtime_dir.as_deref(), &id, timeout, json)
        }
        Commands::Runner { command } => match command {
            RunnerCommands::Remove { runner, project } => run_runner_remove(
                &ruler_root,
                runtime_dir.as_deref(),
                project.as_deref(),
                runner.into(),
            ),
        },
        Commands::Purge { project, yes } => {
            run_purge(&ruler_root, runtime_dir.as_deref(), project.as_deref(), yes)
        }
        Commands::Update {
            check,
            version,
            yes,
            json,
            from_ui,
        } => run_update(
            &ruler_root,
            runtime_dir.as_deref(),
            check,
            version.as_deref(),
            yes,
            json,
            from_ui,
        ),
    }
}

fn load_runtime_with_runner_preflight(
    ruler_root: &Path,
    runtime_dir: Option<&Path>,
    command_name: &str,
    options: RunnerCheckOptions,
) -> Result<::agent_ruler::config::RuntimeState> {
    let mut runtime = load_runtime(ruler_root, runtime_dir)
        .context("load runtime (run `agent-ruler init` first)")?;
    let _ = reconcile_runner_executable_with_options(&mut runtime, command_name, options)?;
    Ok(runtime)
}

const MANAGED_CHILD_PID_FILE_ENV: &str = "AGENT_RULER_MANAGED_CHILD_PID_FILE";
const GATEWAY_PID_RECORD_FILE_NAME: &str = "openclaw-gateway.pid.json";
const GATEWAY_CHILD_PID_FILE_NAME: &str = "openclaw-gateway.child.pid";
const GATEWAY_LOG_FILE_NAME: &str = "openclaw-gateway.log";
const OPENCLAW_CHANNEL_BRIDGE_PID_FILE_NAME: &str = "openclaw-channel-bridge.pid";
const OPENCLAW_CHANNEL_BRIDGE_LOG_FILE_NAME: &str = "openclaw-channel-bridge.log";
const OPENCLAW_BRIDGE_RUNNER_DIR_NAME: &str = "openclaw";
const OPENCLAW_CHANNEL_BRIDGE_CONFIG_FILE_NAME: &str = "channel-bridge.json";
const OPENCLAW_CHANNEL_BRIDGE_LEGACY_CONFIG_FILE_NAME: &str = "openclaw-channel-bridge.json";
const OPENCLAW_CHANNEL_BRIDGE_LOCAL_CONFIG_FILE_NAME: &str = "channel-bridge.local.json";
const OPENCLAW_CHANNEL_BRIDGE_LEGACY_LOCAL_CONFIG_FILE_NAME: &str =
    "openclaw-channel-bridge.local.json";
const OPENCLAW_CHANNEL_BRIDGE_SCRIPT_FILE_NAME: &str = "channel_bridge.py";
const OPENCLAW_CHANNEL_BRIDGE_LEGACY_SCRIPT_FILE_NAME: &str = "openclaw_channel_bridge.py";
const OPENCLAW_BRIDGE_ROUTES_POINTER: &str =
    "plugins.entries.openclaw-agent-ruler-tools.config.approvalBridgeRoutes";
const OPENCLAW_APPROVALS_HOOK_ID: &str = "agent-ruler-approvals";
const OPENCLAW_APPROVALS_HOOK_DIR_NAME: &str = "approvals-hook";
const OPENCLAW_APPROVALS_HOOK_LEGACY_DIR_NAME: &str = "openclaw-approvals-hook";
const OPENCLAW_APPROVALS_HOOK_BRIDGE_URL_POINTER: &str =
    "hooks.internal.entries.agent-ruler-approvals.env.AR_OPENCLAW_BRIDGE_URL";
const OPENCLAW_PREFLIGHT_UI_LOG_FILE_NAME: &str = "agent-ruler-ui.log";
const OPENCLAW_TOOL_PREFLIGHT_PATH: &str = "/api/openclaw/tool/preflight";

fn stop_host_gateway() -> Result<()> {
    let _ = Command::new("openclaw").args(["gateway", "stop"]).status();

    let _ = Command::new("systemctl")
        .args(["--user", "stop", "openclaw-gateway.service"])
        .status();

    Ok(())
}

fn bridge_root_dir(runtime: &::agent_ruler::config::RuntimeState) -> PathBuf {
    runtime.config.ruler_root.join("bridge")
}

fn openclaw_bridge_dir(runtime: &::agent_ruler::config::RuntimeState) -> PathBuf {
    bridge_root_dir(runtime).join(OPENCLAW_BRIDGE_RUNNER_DIR_NAME)
}

fn openclaw_bridge_script_path(runtime: &::agent_ruler::config::RuntimeState) -> PathBuf {
    let preferred = openclaw_bridge_dir(runtime).join(OPENCLAW_CHANNEL_BRIDGE_SCRIPT_FILE_NAME);
    if preferred.exists() {
        return preferred;
    }
    let legacy = bridge_root_dir(runtime).join(OPENCLAW_CHANNEL_BRIDGE_LEGACY_SCRIPT_FILE_NAME);
    if legacy.exists() {
        return legacy;
    }
    preferred
}

fn openclaw_approvals_hook_source_dir(runtime: &::agent_ruler::config::RuntimeState) -> PathBuf {
    let preferred = openclaw_bridge_dir(runtime).join(OPENCLAW_APPROVALS_HOOK_DIR_NAME);
    if preferred.exists() {
        return preferred;
    }
    let legacy = bridge_root_dir(runtime).join(OPENCLAW_APPROVALS_HOOK_LEGACY_DIR_NAME);
    if legacy.exists() {
        return legacy;
    }
    preferred
}

fn managed_openclaw_approvals_hook_dir(runtime: &::agent_ruler::config::RuntimeState) -> PathBuf {
    managed_openclaw_home(runtime)
        .join(".openclaw")
        .join("hooks")
        .join(OPENCLAW_APPROVALS_HOOK_ID)
}

fn write_generated_openclaw_bridge_config(
    runtime: &::agent_ruler::config::RuntimeState,
) -> Result<(PathBuf, String)> {
    let config = ensure_generated_config(runtime).context("generate managed bridge config")?;
    let path = generated_config_path(runtime);
    Ok((path, config.inbound_bind))
}

fn copy_directory_recursive(src: &Path, dst: &Path) -> Result<()> {
    if !src.is_dir() {
        return Err(anyhow!("source is not a directory: {}", src.display()));
    }
    fs::create_dir_all(dst).with_context(|| format!("create {}", dst.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("read {}", src.display()))? {
        let entry = entry.with_context(|| format!("read {}", src.display()))?;
        let src_path = entry.path();
        let dest_path = dst.join(entry.file_name());
        let file_type = entry
            .file_type()
            .with_context(|| format!("stat {}", src_path.display()))?;
        if file_type.is_dir() {
            copy_directory_recursive(&src_path, &dest_path)?;
            continue;
        }
        if file_type.is_file() {
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            fs::copy(&src_path, &dest_path).with_context(|| {
                format!("copy {} -> {}", src_path.display(), dest_path.display())
            })?;
        }
    }
    Ok(())
}

fn command_failure_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }
    format!("exit status {}", output.status)
}

fn configure_managed_openclaw_approvals_hook(
    runtime: &::agent_ruler::config::RuntimeState,
) -> Result<()> {
    let source_hook_dir = openclaw_approvals_hook_source_dir(runtime);
    if !source_hook_dir.exists() {
        return Err(anyhow!(
            "missing approvals hook source at {}",
            source_hook_dir.display()
        ));
    }

    let managed_hook_dir = managed_openclaw_approvals_hook_dir(runtime);
    if managed_hook_dir.exists() {
        fs::remove_dir_all(&managed_hook_dir)
            .with_context(|| format!("remove {}", managed_hook_dir.display()))?;
    }
    copy_directory_recursive(&source_hook_dir, &managed_hook_dir)?;

    let bridge_config =
        ensure_generated_config(runtime).context("load generated bridge config for hook wiring")?;
    let bridge_url = format!("http://{}/inbound", bridge_config.inbound_bind.trim());
    let bridge_url_json =
        serde_json::to_string(&bridge_url).context("serialize inbound bridge URL for OpenClaw")?;
    let managed_home = managed_openclaw_home(runtime);

    let enable_output = Command::new("openclaw")
        .args(["hooks", "enable", OPENCLAW_APPROVALS_HOOK_ID])
        .env("OPENCLAW_HOME", &managed_home)
        .output()
        .with_context(|| {
            format!(
                "run `openclaw hooks enable {}` with OPENCLAW_HOME={}",
                OPENCLAW_APPROVALS_HOOK_ID,
                managed_home.display()
            )
        })?;
    if !enable_output.status.success() {
        return Err(anyhow!(command_failure_detail(&enable_output)));
    }

    let set_env_output = Command::new("openclaw")
        .args([
            "config",
            "set",
            OPENCLAW_APPROVALS_HOOK_BRIDGE_URL_POINTER,
            &bridge_url_json,
            "--json",
        ])
        .env("OPENCLAW_HOME", &managed_home)
        .output()
        .with_context(|| {
            format!(
                "run `openclaw config set {}` with OPENCLAW_HOME={}",
                OPENCLAW_APPROVALS_HOOK_BRIDGE_URL_POINTER,
                managed_home.display()
            )
        })?;
    if !set_env_output.status.success() {
        return Err(anyhow!(command_failure_detail(&set_env_output)));
    }

    eprintln!(
        "bridge diagnostics: managed approvals hook ready at {} (inbound: {}).",
        managed_hook_dir.display(),
        bridge_url
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BridgeStartupState {
    NotRequired,
    Active,
}

fn openclaw_bridge_pid_file(runtime: &::agent_ruler::config::RuntimeState) -> PathBuf {
    runtime
        .config
        .runtime_root
        .join("user_data")
        .join("logs")
        .join(OPENCLAW_CHANNEL_BRIDGE_PID_FILE_NAME)
}

fn openclaw_bridge_log_file(runtime: &::agent_ruler::config::RuntimeState) -> PathBuf {
    runtime
        .config
        .runtime_root
        .join("user_data")
        .join("logs")
        .join(OPENCLAW_CHANNEL_BRIDGE_LOG_FILE_NAME)
}

fn bridge_config_routes(config_path: &Path) -> Result<Option<Vec<serde_json::Value>>> {
    let raw = fs::read_to_string(config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", config_path.display()))?;
    let routes = parsed
        .get("routes")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    if routes.is_empty() {
        return Ok(None);
    }
    Ok(Some(routes))
}

fn write_bridge_config_routes(config_path: &Path, routes: &[serde_json::Value]) -> Result<()> {
    if routes.is_empty() {
        return Ok(());
    }
    let raw = fs::read_to_string(config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let mut parsed: serde_json::Value =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", config_path.display()))?;
    let Some(root) = parsed.as_object_mut() else {
        return Err(anyhow!(
            "bridge config root must be a JSON object: {}",
            config_path.display()
        ));
    };
    root.insert(
        "routes".to_string(),
        serde_json::Value::Array(routes.to_vec()),
    );
    fs::write(
        config_path,
        serde_json::to_string_pretty(&parsed).context("serialize bridge config JSON")?,
    )
    .with_context(|| format!("write {}", config_path.display()))
}

fn find_legacy_bridge_routes(
    runtime: &::agent_ruler::config::RuntimeState,
) -> Result<Option<(PathBuf, Vec<serde_json::Value>)>> {
    for candidate in [
        openclaw_bridge_dir(runtime).join(OPENCLAW_CHANNEL_BRIDGE_CONFIG_FILE_NAME),
        openclaw_bridge_dir(runtime).join(OPENCLAW_CHANNEL_BRIDGE_LOCAL_CONFIG_FILE_NAME),
        bridge_root_dir(runtime).join(OPENCLAW_CHANNEL_BRIDGE_LEGACY_CONFIG_FILE_NAME),
        bridge_root_dir(runtime).join(OPENCLAW_CHANNEL_BRIDGE_LEGACY_LOCAL_CONFIG_FILE_NAME),
    ] {
        if !candidate.exists() {
            continue;
        }
        if let Some(routes) = bridge_config_routes(&candidate)? {
            return Ok(Some((candidate, routes)));
        }
    }

    Ok(None)
}

fn write_managed_openclaw_bridge_routes(
    runtime: &::agent_ruler::config::RuntimeState,
    routes: &[serde_json::Value],
) -> Result<()> {
    if routes.is_empty() {
        return Ok(());
    }

    let managed_home = managed_openclaw_home(runtime);
    let serialized =
        serde_json::to_string(routes).context("serialize bridge routes for OpenClaw config set")?;
    let output = Command::new("openclaw")
        .args([
            "config",
            "set",
            OPENCLAW_BRIDGE_ROUTES_POINTER,
            &serialized,
            "--json",
        ])
        .env("OPENCLAW_HOME", &managed_home)
        .output()
        .with_context(|| {
            format!(
                "run `openclaw config set {}` with OPENCLAW_HOME={}",
                OPENCLAW_BRIDGE_ROUTES_POINTER,
                managed_home.display()
            )
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let detail = if stderr.is_empty() {
        format!("exit status {}", output.status)
    } else {
        stderr
    };
    Err(anyhow!(detail))
}

fn managed_openclaw_bridge_routes_count(
    runtime: &::agent_ruler::config::RuntimeState,
) -> Result<Option<usize>> {
    let managed_home = managed_openclaw_home(runtime);
    let output = Command::new("openclaw")
        .args(["config", "get", OPENCLAW_BRIDGE_ROUTES_POINTER, "--json"])
        .env("OPENCLAW_HOME", &managed_home)
        .output()
        .with_context(|| {
            format!(
                "run `openclaw config get {}` with OPENCLAW_HOME={}",
                OPENCLAW_BRIDGE_ROUTES_POINTER,
                managed_home.display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr
            .to_ascii_lowercase()
            .contains("config path not found")
        {
            return Ok(None);
        }
        let detail = if stderr.is_empty() {
            format!("exit status {}", output.status)
        } else {
            stderr
        };
        return Err(anyhow!(detail));
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() || raw.eq_ignore_ascii_case("null") {
        return Ok(None);
    }

    let parsed: serde_json::Value =
        serde_json::from_str(&raw).context("parse OpenClaw bridge routes JSON")?;
    let Some(routes) = parsed.as_array() else {
        return Ok(None);
    };
    if routes.is_empty() {
        return Ok(None);
    }
    Ok(Some(routes.len()))
}

fn maybe_start_managed_openclaw_bridge(
    runtime: &::agent_ruler::config::RuntimeState,
) -> Result<BridgeStartupState> {
    let (config_path, inbound_bind) = write_generated_openclaw_bridge_config(runtime)
        .context("generate bridge runtime config before startup")?;
    let inbound_addrs =
        resolve_socket_addrs(&inbound_bind).context("resolve bridge inbound bind address")?;
    let inline_routes = match find_legacy_bridge_routes(runtime) {
        Ok(Some((source_path, routes))) => {
            eprintln!(
                "bridge diagnostics: found {} route(s) in legacy bridge config {}; will auto-seed managed OpenClaw routes when missing.",
                routes.len(),
                source_path.display()
            );
            Some(routes)
        }
        Ok(None) => None,
        Err(err) => {
            eprintln!("bridge diagnostics: unable to inspect legacy bridge route sources: {err}");
            None
        }
    };

    let mut managed_routes_present = false;
    let mut inline_fallback_ready = false;
    let mut proceed_with_channel_autodiscovery = false;
    match managed_openclaw_bridge_routes_count(runtime) {
        Ok(Some(count)) => {
            eprintln!(
                "bridge diagnostics: managed OpenClaw bridge routes present ({} route(s)).",
                count
            );
            managed_routes_present = true;
        }
        Ok(None) => {
            if let Some(routes) = inline_routes.as_ref() {
                match write_managed_openclaw_bridge_routes(runtime, routes) {
                    Ok(()) => {
                        managed_routes_present = true;
                        eprintln!(
                            "bridge diagnostics: auto-seeded managed OpenClaw bridge routes at `{}` from existing bridge config ({} route(s)).",
                            OPENCLAW_BRIDGE_ROUTES_POINTER,
                            routes.len()
                        );
                    }
                    Err(err) => {
                        eprintln!(
                            "bridge diagnostics: unable to auto-seed managed OpenClaw bridge routes: {err}"
                        );
                        match write_bridge_config_routes(&config_path, routes) {
                            Ok(()) => {
                                inline_fallback_ready = true;
                                eprintln!(
                                    "bridge diagnostics: using runtime-generated inline bridge routes as fallback."
                                );
                            }
                            Err(write_err) => {
                                eprintln!(
                                    "bridge diagnostics: unable to write runtime-generated inline bridge routes fallback: {write_err}"
                                );
                            }
                        }
                    }
                }
            }
            if managed_routes_present {
                // no-op: managed routes are now populated
            } else if inline_fallback_ready {
                eprintln!(
                    "bridge diagnostics: managed OpenClaw bridge routes missing at `{}`; using inline bridge config routes as fallback.",
                    OPENCLAW_BRIDGE_ROUTES_POINTER
                );
            } else {
                eprintln!(
                    "bridge diagnostics: managed OpenClaw bridge routes missing at `{}`; bridge will attempt channel-default route autodiscovery.",
                    OPENCLAW_BRIDGE_ROUTES_POINTER
                );
                proceed_with_channel_autodiscovery = true;
            }
        }
        Err(err) => {
            eprintln!("bridge diagnostics: unable to verify managed OpenClaw bridge routes: {err}");
            proceed_with_channel_autodiscovery = true;
        }
    }
    if !managed_routes_present && !inline_fallback_ready && !proceed_with_channel_autodiscovery {
        return Ok(BridgeStartupState::NotRequired);
    }

    let script_path = openclaw_bridge_script_path(runtime);
    if !script_path.exists() {
        return Err(anyhow!(
            "managed bridge startup blocked: missing bridge script {}",
            script_path.display()
        ));
    }

    let pid_file = openclaw_bridge_pid_file(runtime);
    if let Ok(raw) = fs::read_to_string(&pid_file) {
        if let Ok(pid) = raw.trim().parse::<u32>() {
            if process_exists(pid) {
                if is_any_tcp_addr_reachable(&inbound_addrs) {
                    eprintln!(
                        "bridge diagnostics: managed OpenClaw channel bridge already running (pid: {}, inbound: {}).",
                        pid, inbound_bind
                    );
                    return Ok(BridgeStartupState::Active);
                }
                eprintln!(
                    "bridge diagnostics: clearing stale bridge pid {} (inbound {} is not reachable).",
                    pid, inbound_bind
                );
            }
        }
    }
    let _ = remove_if_exists(&pid_file);

    let logs_dir = runtime.config.runtime_root.join("user_data").join("logs");
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("create bridge logs directory {}", logs_dir.display()))?;
    let log_path = openclaw_bridge_log_file(runtime);
    let stdout = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open bridge log {}", log_path.display()))?;
    let stderr = stdout
        .try_clone()
        .with_context(|| format!("clone bridge log handle {}", log_path.display()))?;

    let managed_home = managed_openclaw_home(runtime);
    let current_exe =
        std::env::current_exe().context("resolve current agent-ruler binary for bridge launch")?;

    let mut child = Command::new("python3")
        .arg(&script_path)
        .arg("--config")
        .arg(&config_path)
        .arg("--openclaw-home")
        .arg(&managed_home)
        .arg("--agent-ruler-bin")
        .arg(&current_exe)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .context("spawn managed OpenClaw channel bridge process")?;

    if let Err(err) = fs::write(&pid_file, format!("{}\n", child.id())) {
        eprintln!(
            "bridge diagnostics: unable to persist bridge pid file {}: {err}",
            pid_file.display()
        );
    }

    let startup_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let _ = remove_if_exists(&pid_file);
                return Err(anyhow!(
                    "managed bridge exited before inbound listener {} became ready (status {}); check {}",
                    inbound_bind,
                    status,
                    log_path.display()
                ));
            }
            Ok(None) => {
                if is_any_tcp_addr_reachable(&inbound_addrs) {
                    eprintln!(
                        "bridge diagnostics: managed OpenClaw channel bridge started (pid: {}, inbound: {}, log: {}).",
                        child.id(),
                        inbound_bind,
                        log_path.display()
                    );
                    return Ok(BridgeStartupState::Active);
                }
                if Instant::now() >= startup_deadline {
                    let _ = kill_openclaw_bridge_process(child.id());
                    let _ = remove_if_exists(&pid_file);
                    return Err(anyhow!(
                        "managed bridge did not open inbound listener {} within startup timeout; check {}",
                        inbound_bind,
                        log_path.display()
                    ));
                }
                std::thread::sleep(Duration::from_millis(120));
            }
            Err(err) => {
                return Err(anyhow!(
                    "unable to confirm managed bridge status: {err} (log: {})",
                    log_path.display()
                ));
            }
        }
    }
}

/// Start the UI while writing the pid file so the new stop command can locate it.
async fn run_ui_server(runtime: RuntimeState, bind: String) -> Result<()> {
    let _pid_guard = UiPidGuard::create(&runtime)?;
    ui::serve(
        runtime.config.ruler_root.clone(),
        Some(runtime.config.runtime_root.clone()),
        bind,
    )
    .await
}

/// Terminate the UI process that was already recorded in the pid file.
fn stop_ui_action(runtime: &RuntimeState) -> Result<()> {
    if stop_ui_processes_in_projects_root(runtime)? {
        Ok(())
    } else {
        Err(anyhow!(
            "ui stop: Agent Ruler UI did not exit within the expected time"
        ))
    }
}

/// Launch a command in a detached child so the CLI can return immediately (used for `openclaw gateway`).
/// Gateway launches are serialized: we remember the managed PID, stop any host gateway, and record logs+pid.
fn spawn_background_run(
    runtime: &::agent_ruler::config::RuntimeState,
    cmd: &[String],
) -> Result<()> {
    if cmd.is_empty() {
        return Err(anyhow!("empty command"));
    }

    let gateway_launch = is_openclaw_gateway_launch(cmd);
    let logs_dir = runtime.config.runtime_root.join("user_data").join("logs");
    if gateway_launch {
        // Enforce a single managed detached gateway per runtime by honoring
        // the persisted PID record before spawning another launcher.
        let record_path = gateway_pid_record_file(runtime);
        let gateway_log_path = logs_dir.join(GATEWAY_LOG_FILE_NAME);
        if record_path.exists() {
            if let Ok(raw) = fs::read_to_string(&record_path) {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if let Some(pid) = parsed.get("pid").and_then(serde_json::Value::as_u64) {
                        if process_exists(pid as u32) {
                            return Err(anyhow!(
                                "managed OpenClaw gateway is already running (pid: {}). Stop it first with `agent-ruler run -- openclaw gateway stop`.",
                                pid
                            ));
                        }
                    }
                }
            }
            let _ = remove_if_exists(&record_path);
        }
        if let Some(pid) = detect_managed_gateway_listener_pid(runtime) {
            write_gateway_pid_record(runtime, pid, pid, &gateway_log_path, cmd)?;
            println!(
                "OpenClaw gateway is already running in managed mode (pid: {}).",
                pid
            );
            println!("logs: {}", gateway_log_path.display());
            println!("stop command: agent-ruler run -- openclaw gateway stop");
            return Ok(());
        }
    }
    if gateway_launch {
        // Best-effort cleanup of unmanaged host gateway instances to reduce
        // port conflicts before managed detached launch.
        let _ = stop_host_gateway();
    }

    fs::create_dir_all(&logs_dir).with_context(|| format!("create {}", logs_dir.display()))?;
    let log_path = if gateway_launch {
        logs_dir.join(GATEWAY_LOG_FILE_NAME)
    } else {
        logs_dir.join("agent-ruler-run.log")
    };
    let stdout = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open {}", log_path.display()))?;
    let log_offset = stdout
        .metadata()
        .with_context(|| format!("metadata {}", log_path.display()))?
        .len();
    let stderr = stdout
        .try_clone()
        .with_context(|| format!("clone {}", log_path.display()))?;

    let current_exe = std::env::current_exe().context("resolve current agent-ruler executable")?;
    let mut child_cmd = Command::new(current_exe);
    child_cmd
        .arg("--runtime-dir")
        .arg(&runtime.config.runtime_root)
        .arg("run");
    if gateway_launch {
        child_cmd.arg("--foreground");
    }
    child_cmd.arg("--");
    for token in cmd {
        child_cmd.arg(token);
    }
    if gateway_launch {
        let pid_capture_file = gateway_child_pid_file(runtime);
        remove_if_exists(&pid_capture_file)?;
        child_cmd.env(MANAGED_CHILD_PID_FILE_ENV, &pid_capture_file);
    }

    child_cmd
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));

    let child = child_cmd.spawn().context("spawn background run command")?;

    if gateway_launch {
        // Gateway writes its own process PID to logs after daemonization.
        // Capture that managed PID (not launcher PID) for future stop semantics.
        let managed_pid =
            match wait_for_gateway_child_pid(runtime, child.id(), &log_path, log_offset) {
                Ok(pid) => pid,
                Err(err) => {
                    let _ = stop_managed_openclaw_bridge(runtime);
                    return Err(err);
                }
            };
        write_gateway_pid_record(runtime, managed_pid, child.id(), &log_path, cmd)?;
        println!("OpenClaw gateway started detached.");
        println!("PID: {}", managed_pid);
        println!("logs: {}", log_path.display());
        println!("follow logs: tail -f {}", log_path.display());
        println!("stop command: agent-ruler run -- openclaw gateway stop");
        return Ok(());
    }

    println!("background run started (pid: {})", child.id());
    println!("logs: {}", log_path.display());
    println!("follow logs: tail -f {}", log_path.display());
    Ok(())
}

fn is_openclaw_gateway_launch(cmd: &[String]) -> bool {
    let tokens = command_tokens_without_env_prefix(cmd);
    if tokens.len() < 2 {
        return false;
    }
    if tokens[0] != "openclaw" || tokens[1] != "gateway" {
        return false;
    }
    !tokens
        .iter()
        .skip(2)
        .any(|token| *token == "stop" || *token == "status")
}

fn is_openclaw_command(cmd: &[String]) -> bool {
    let tokens = command_tokens_without_env_prefix(cmd);
    tokens.first().copied() == Some("openclaw")
}

fn openclaw_command_needs_preflight_api(cmd: &[String]) -> bool {
    let tokens = command_tokens_without_env_prefix(cmd);
    if tokens.len() < 2 || tokens[0] != "openclaw" {
        return false;
    }

    match tokens[1] {
        "agent" | "tui" | "acp" => true,
        "gateway" => !tokens
            .iter()
            .skip(2)
            .any(|token| *token == "stop" || *token == "status"),
        _ => false,
    }
}

fn ensure_openclaw_preflight_api_ready(
    runtime: &::agent_ruler::config::RuntimeState,
    cmd: &[String],
) -> Result<()> {
    if !openclaw_command_needs_preflight_api(cmd) {
        return Ok(());
    }

    let bind = runtime.config.ui_bind.trim();
    let addrs = resolve_socket_addrs(bind)?;
    if is_any_tcp_addr_reachable(&addrs) {
        return ensure_existing_ui_supports_openclaw_preflight(&addrs, bind);
    }

    let logs_dir = runtime.config.runtime_root.join("user_data").join("logs");
    fs::create_dir_all(&logs_dir).with_context(|| format!("create {}", logs_dir.display()))?;
    let log_path = logs_dir.join(OPENCLAW_PREFLIGHT_UI_LOG_FILE_NAME);
    let stdout = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open {}", log_path.display()))?;
    let stderr = stdout
        .try_clone()
        .with_context(|| format!("clone {}", log_path.display()))?;

    let current_exe = std::env::current_exe().context("resolve current agent-ruler executable")?;
    let mut child = Command::new(current_exe);
    child
        .arg("--runtime-dir")
        .arg(&runtime.config.runtime_root)
        .arg("ui")
        .arg("--bind")
        .arg(bind)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));

    let mut child = child
        .spawn()
        .context("spawn background Agent Ruler UI for OpenClaw preflight")?;
    let started = Instant::now();
    loop {
        if is_any_tcp_addr_reachable(&addrs) {
            match probe_openclaw_preflight_endpoint_status(&addrs, bind) {
                Ok(Some(404)) => {
                    let _ = child.kill();
                    return Err(anyhow!(
                        "OpenClaw preflight API unavailable at http://{}: endpoint {} returned HTTP 404. This usually means a stale Agent Ruler UI binary is running. Stop the existing UI and reinstall/update Agent Ruler (for local builds: `bash install/install.sh --local`).",
                        bind,
                        OPENCLAW_TOOL_PREFLIGHT_PATH
                    ));
                }
                Ok(Some(_status)) => {
                    eprintln!(
                        "preflight api: started Agent Ruler UI at http://{} for OpenClaw tool mediation.",
                        bind
                    );
                    return Ok(());
                }
                Ok(None) | Err(_) => {}
            }
        }

        if let Some(status) = child
            .try_wait()
            .context("poll background Agent Ruler UI process")?
        {
            return Err(anyhow!(
                "OpenClaw preflight API unavailable at http://{}; background UI exited with status {}. Check {}",
                bind,
                status,
                log_path.display()
            ));
        }

        if started.elapsed() > Duration::from_secs(8) {
            return Err(anyhow!(
                "OpenClaw preflight API unavailable at http://{}; timed out while starting background UI. Check {}",
                bind,
                log_path.display()
            ));
        }

        std::thread::sleep(Duration::from_millis(200));
    }
}

fn resolve_socket_addrs(bind: &str) -> Result<Vec<SocketAddr>> {
    let addrs = bind
        .to_socket_addrs()
        .with_context(|| format!("resolve ui bind `{bind}` for OpenClaw preflight"))?
        .collect::<Vec<_>>();
    if addrs.is_empty() {
        return Err(anyhow!(
            "ui bind `{}` did not resolve to any socket address",
            bind
        ));
    }
    Ok(addrs)
}

fn is_any_tcp_addr_reachable(addrs: &[SocketAddr]) -> bool {
    addrs.iter().copied().any(is_tcp_addr_reachable)
}

fn is_tcp_addr_reachable(addr: SocketAddr) -> bool {
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

fn ensure_existing_ui_supports_openclaw_preflight(addrs: &[SocketAddr], bind: &str) -> Result<()> {
    match probe_openclaw_preflight_endpoint_status(addrs, bind) {
        Ok(Some(404)) => Err(anyhow!(
            "OpenClaw preflight API unavailable at http://{}: endpoint {} returned HTTP 404. A stale Agent Ruler UI process is likely running. Stop it and rerun with the current Agent Ruler binary (for local builds: `bash install/install.sh --local`).",
            bind,
            OPENCLAW_TOOL_PREFLIGHT_PATH
        )),
        Ok(Some(_status)) => Ok(()),
        Ok(None) => Err(anyhow!(
            "OpenClaw preflight API probe failed at http://{}: UI port is reachable but no HTTP status was returned for {}. Check for port conflicts and restart Agent Ruler UI.",
            bind,
            OPENCLAW_TOOL_PREFLIGHT_PATH
        )),
        Err(err) => Err(anyhow!(
            "OpenClaw preflight API probe failed at http://{} for {}: {}",
            bind,
            OPENCLAW_TOOL_PREFLIGHT_PATH,
            err
        )),
    }
}

fn probe_openclaw_preflight_endpoint_status(
    addrs: &[SocketAddr],
    host_header: &str,
) -> Result<Option<u16>> {
    let mut last_err: Option<anyhow::Error> = None;
    for addr in addrs {
        match probe_openclaw_preflight_endpoint(*addr, host_header) {
            Ok(status) => return Ok(Some(status)),
            Err(err) => last_err = Some(err),
        }
    }

    match last_err {
        Some(err) => Err(err),
        None => Ok(None),
    }
}

fn probe_openclaw_preflight_endpoint(addr: SocketAddr, host_header: &str) -> Result<u16> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(300))
        .with_context(|| format!("connect to preflight api probe target {}", addr))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(700)))
        .with_context(|| format!("set read timeout for preflight probe {}", addr))?;
    stream
        .set_write_timeout(Some(Duration::from_millis(700)))
        .with_context(|| format!("set write timeout for preflight probe {}", addr))?;

    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        OPENCLAW_TOOL_PREFLIGHT_PATH, host_header
    );

    stream
        .write_all(request.as_bytes())
        .with_context(|| format!("write preflight probe request to {}", addr))?;
    let mut response = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                response.extend_from_slice(&chunk[..read]);
                if response.contains(&b'\n') {
                    break;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::TimedOut => break,
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("read preflight probe response from {}", addr));
            }
        }
    }

    parse_http_status_code(&response).ok_or_else(|| {
        let preview_len = response.len().min(120);
        let preview = String::from_utf8_lossy(&response[..preview_len])
            .replace('\r', "\\r")
            .replace('\n', "\\n");
        anyhow!(
            "unable to parse HTTP status from preflight probe response at {} (preview: `{}`)",
            addr,
            preview
        )
    })
}

fn parse_http_status_code(response: &[u8]) -> Option<u16> {
    let text = String::from_utf8_lossy(response);
    let start = text.find("HTTP/")?;
    let mut parts = text[start..].split_whitespace();
    let version = parts.next()?;
    if !version.starts_with("HTTP/") {
        return None;
    }
    parts.next()?.parse::<u16>().ok()
}

fn parse_bind_port(bind: &str) -> u16 {
    bind.rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
        .unwrap_or(4622)
}

fn detect_tailscale_ipv4() -> Result<Option<String>> {
    let output = Command::new("tailscale")
        .args(["ip", "-4"])
        .output()
        .map_err(|err| anyhow!("tailscale CLI unavailable: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("tailscale ip -4 exited with status {}", output.status)
        } else {
            stderr
        };
        return Err(anyhow!(detail));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let ip = stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string);
    Ok(ip)
}

fn maybe_auto_configure_ui_bind_for_tailscale(runtime: &mut RuntimeState) {
    let port = parse_bind_port(&runtime.config.ui_bind);
    let local_bind = format!("127.0.0.1:{port}");

    let desired_bind = match detect_tailscale_ipv4() {
        Ok(Some(ip)) => {
            let bind = format!("{ip}:{port}");
            if bind != runtime.config.ui_bind {
                eprintln!(
                    "tailscale auto-bind: detected {ip}; configuring Control Panel bind to {bind}."
                );
            }
            bind
        }
        Ok(None) => {
            eprintln!(
                "tailscale auto-bind: no tailscale IPv4 found; continuing locally at {local_bind}. After Tailscale is configured, restart Agent Ruler to auto-bind."
            );
            local_bind
        }
        Err(err) => {
            eprintln!(
                "tailscale auto-bind: {err}; continuing locally at {local_bind}. Install/configure Tailscale, then restart Agent Ruler to auto-bind."
            );
            local_bind
        }
    };

    if desired_bind == runtime.config.ui_bind {
        return;
    }

    runtime.config.ui_bind = desired_bind.clone();
    let config_path = runtime.config.state_dir.join(CONFIG_FILE_NAME);
    if let Err(err) = save_config(&config_path, &runtime.config) {
        eprintln!(
            "tailscale auto-bind: unable to persist ui_bind={} ({err}); using current-session value only.",
            desired_bind
        );
    }
}

fn normalize_openclaw_gateway_launch_command(cmd: &[String]) -> Vec<String> {
    let tokens = command_tokens_without_env_prefix(cmd);
    if tokens.len() == 2 && tokens[0] == "openclaw" && tokens[1] == "gateway" {
        return vec![
            "openclaw".to_string(),
            "gateway".to_string(),
            "run".to_string(),
        ];
    }
    cmd.to_vec()
}

fn is_openclaw_gateway_stop(cmd: &[String]) -> bool {
    let tokens = command_tokens_without_env_prefix(cmd);
    if tokens.len() < 3 {
        return false;
    }
    tokens[0] == "openclaw" && tokens[1] == "gateway" && tokens[2] == "stop"
}

fn command_tokens_without_env_prefix(cmd: &[String]) -> Vec<&str> {
    if cmd.is_empty() {
        return Vec::new();
    }
    if cmd[0] != "env" {
        return cmd.iter().map(String::as_str).collect();
    }
    let mut out: Vec<&str> = Vec::new();
    let mut index = 1usize;
    while index < cmd.len() {
        let token = cmd[index].as_str();
        if token.contains('=') {
            index += 1;
            continue;
        }
        out.extend(cmd[index..].iter().map(String::as_str));
        return out;
    }
    out
}

fn print_openclaw_gateway_port_diagnostics(
    runtime: &::agent_ruler::config::RuntimeState,
    cmd: &[String],
    stdout: &str,
    stderr: &str,
) {
    if !is_openclaw_gateway_launch(cmd) {
        return;
    }

    let managed_home = runtime
        .config
        .runner
        .as_ref()
        .map(|runner| runner.managed_home.clone())
        .unwrap_or_else(|| {
            runtime
                .config
                .runtime_root
                .join("user_data")
                .join("openclaw_home")
        });

    let diagnostics = match maybe_collect_gateway_port_diagnostics(&managed_home, stdout, stderr) {
        Ok(Some(value)) => value,
        Ok(None) => return,
        Err(err) => {
            eprintln!("gateway diagnostics: unable to collect port owner details: {err}");
            return;
        }
    };

    let expected_home = managed_home.to_string_lossy().to_string();
    eprintln!("gateway diagnostics: port/listener conflict detected.");
    if let Some(port) = diagnostics.port {
        eprintln!("gateway diagnostics: listener detected on port {port}.");
    } else {
        eprintln!("gateway diagnostics: listener port could not be inferred from output.");
    }

    if diagnostics.listeners.is_empty() {
        eprintln!("gateway diagnostics: no listener details found from `ss -ltnp`.");
    } else {
        for listener in diagnostics.listeners {
            match (listener.pid, listener.openclaw_home.as_deref()) {
                (Some(pid), Some(home)) => {
                    eprintln!(
                        "gateway diagnostics: pid {} listening; OPENCLAW_HOME={}",
                        pid, home
                    );
                    if home != expected_home {
                        eprintln!(
                            "gateway diagnostics: note this differs from managed OPENCLAW_HOME={}",
                            expected_home
                        );
                    }
                    eprintln!("gateway diagnostics: ss line: {}", listener.ss_line);
                }
                (Some(pid), None) => {
                    eprintln!(
                        "gateway diagnostics: pid {} listening; OPENCLAW_HOME not visible in /proc/{}/environ",
                        pid, pid
                    );
                    eprintln!("gateway diagnostics: ss line: {}", listener.ss_line);
                }
                (None, _) => {
                    eprintln!(
                        "gateway diagnostics: listener (pid unavailable): {}",
                        listener.ss_line
                    );
                }
            }
        }
    }

    eprintln!("gateway diagnostics remediation:");
    eprintln!("  1) openclaw gateway stop");
    eprintln!("  2) systemctl --user stop openclaw-gateway.service");
    eprintln!("  3) if still listening, identify PID above and run: kill <pid>");
}

fn managed_openclaw_home(runtime: &::agent_ruler::config::RuntimeState) -> PathBuf {
    runtime
        .config
        .runner
        .as_ref()
        .map(|runner| runner.managed_home.clone())
        .unwrap_or_else(|| {
            runtime
                .config
                .runtime_root
                .join("user_data")
                .join("openclaw_home")
        })
}

fn print_gateway_telegram_hints(runtime: &::agent_ruler::config::RuntimeState) {
    let managed_home = managed_openclaw_home(runtime);
    let status = match inspect_managed_telegram_config(&managed_home) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("telegram diagnostics: unable to inspect managed config: {err}");
            return;
        }
    };

    if status.enabled && !status.token_present {
        eprintln!(
            "telegram diagnostics: Telegram is enabled but token is missing in managed config (`channels.telegram.botToken` or `channels.telegram.token`)."
        );
        eprintln!("telegram diagnostics: rerun `agent-ruler setup` and choose host import.");
    }
    if status.enabled && !gateway_policy_allows_telegram_host(runtime, "api.telegram.org") {
        eprintln!(
            "telegram diagnostics: current network policy does not explicitly allow outbound HTTPS to `api.telegram.org`."
        );
        eprintln!(
            "telegram diagnostics: allow outbound HTTPS to Telegram endpoints (for example `api.telegram.org`) by adjusting network policy."
        );
    }
}

fn print_openclaw_gateway_telegram_diagnostics(
    runtime: &::agent_ruler::config::RuntimeState,
    cmd: &[String],
    stdout: &str,
    stderr: &str,
) {
    if !is_openclaw_gateway_launch(cmd) {
        return;
    }
    if !looks_like_telegram_command_sync_failure(stdout, stderr) {
        return;
    }
    eprintln!("telegram diagnostics: detected Telegram command sync failure (`setMyCommands`/`deleteMyCommands`).");
    print_gateway_telegram_hints(runtime);
}

fn looks_like_telegram_command_sync_failure(stdout: &str, stderr: &str) -> bool {
    let combined = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    let mentions_command_sync =
        combined.contains("setmycommands") || combined.contains("deletemycommands");
    let mentions_network_failure = combined.contains("network request failed")
        || combined.contains("fetch failed")
        || combined.contains("enotfound")
        || combined.contains("eai_again")
        || combined.contains("etimedout")
        || combined.contains("econnrefused")
        || combined.contains("econnreset");
    mentions_command_sync && mentions_network_failure
}

fn gateway_policy_allows_telegram_host(
    runtime: &::agent_ruler::config::RuntimeState,
    host: &str,
) -> bool {
    network_policy_allows_host(&runtime.policy.rules.network, host)
}

fn network_policy_allows_host(rules: &::agent_ruler::config::NetworkRules, host: &str) -> bool {
    let in_allowlist = rules
        .allowlist_hosts
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(host));
    let in_denylist = rules
        .denylist_hosts
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(host));

    let allowlist_pass = if rules.allowlist_hosts.is_empty() {
        true
    } else if rules.invert_allowlist {
        !in_allowlist
    } else {
        in_allowlist
    };

    let denylist_pass = if rules.denylist_hosts.is_empty() {
        true
    } else if rules.invert_denylist {
        in_denylist
    } else {
        !in_denylist
    };

    if !allowlist_pass || !denylist_pass {
        return false;
    }

    if !rules.default_deny {
        return true;
    }

    (!rules.allowlist_hosts.is_empty() && !rules.invert_allowlist && in_allowlist)
        || (!rules.denylist_hosts.is_empty() && rules.invert_denylist && in_denylist)
}

fn gateway_pid_record_file(runtime: &::agent_ruler::config::RuntimeState) -> PathBuf {
    runtime
        .config
        .runtime_root
        .join("user_data")
        .join("logs")
        .join(GATEWAY_PID_RECORD_FILE_NAME)
}

fn gateway_child_pid_file(runtime: &::agent_ruler::config::RuntimeState) -> PathBuf {
    runtime
        .config
        .runtime_root
        .join("user_data")
        .join("logs")
        .join(GATEWAY_CHILD_PID_FILE_NAME)
}

fn remove_if_exists(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    }
    Ok(())
}

/// Read appended OpenClaw log entries to capture the gateway PID after daemonization.
fn wait_for_gateway_child_pid(
    runtime: &::agent_ruler::config::RuntimeState,
    launcher_pid: u32,
    log_path: &Path,
    log_offset: u64,
) -> Result<u32> {
    let pid_capture_file = gateway_child_pid_file(runtime);
    let started = Instant::now();
    let max_wait = Duration::from_secs(240);
    loop {
        // Prefer daemonized PID from OpenClaw logs when available.
        if let Some(pid) = parse_gateway_pid_from_log_since(log_path, log_offset) {
            let _ = remove_if_exists(&pid_capture_file);
            return Ok(pid);
        }
        // Some OpenClaw variants daemonize without writing a parsable PID line
        // to the managed log quickly enough. Fall back to listener ownership
        // discovery scoped to the managed OPENCLAW_HOME.
        if let Some(pid) = detect_managed_gateway_listener_pid(runtime) {
            let _ = remove_if_exists(&pid_capture_file);
            return Ok(pid);
        }

        // If the launcher already exited and we still cannot resolve a managed
        // gateway listener PID, fail fast with the latest log excerpt.
        if !process_exists(launcher_pid) {
            break;
        }
        if started.elapsed() > max_wait {
            break;
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    let _ = remove_if_exists(&pid_capture_file);
    stop_managed_background_launcher(Some(launcher_pid), 0);
    let excerpt = recent_log_excerpt_since(log_path, log_offset, 8);
    if excerpt.is_empty() {
        Err(anyhow!(
            "failed to capture managed gateway pid; check log file and try again"
        ))
    } else {
        Err(anyhow!(
            "failed to capture managed gateway pid; check log file and try again\nrecent gateway output:\n{}",
            excerpt
        ))
    }
}

fn parse_gateway_pid_from_log_since(log_path: &Path, offset: u64) -> Option<u32> {
    let bytes = fs::read(log_path).ok()?;
    let start = usize::try_from(offset).ok()?.min(bytes.len());
    let chunk = std::str::from_utf8(&bytes[start..]).ok()?;
    chunk.lines().rev().find_map(|line| {
        let lower = line.to_ascii_lowercase();
        if !lower.contains("listening") {
            return None;
        }
        parse_gateway_pid_from_log_line(line)
            .or_else(|| parse_gateway_pid_from_listening_line(line))
    })
}

/// Tolerantly extract a PID value from log lines that mention `pid` in various formats.
fn parse_gateway_pid_from_log_line(line: &str) -> Option<u32> {
    let needle = "pid";
    let lower = line.to_ascii_lowercase();
    let mut search_start = 0usize;
    while let Some(relative) = lower[search_start..].find(needle) {
        let absolute = search_start + relative;
        let mut digits = String::new();
        let mut digits_started = false;
        for ch in line[absolute + needle.len()..].chars() {
            if ch.is_ascii_digit() {
                digits.push(ch);
                digits_started = true;
                continue;
            }
            if !digits_started
                && (ch.is_ascii_whitespace()
                    || matches!(ch, ':' | '=' | '(' | ')' | '[' | ']' | '"'))
            {
                continue;
            }
            if digits_started {
                break;
            }
            break;
        }

        if !digits.is_empty() {
            if let Ok(pid) = digits.parse::<u32>() {
                return Some(pid);
            }
        }

        search_start = absolute + needle.len();
    }

    None
}

fn parse_gateway_pid_from_listening_line(line: &str) -> Option<u32> {
    let lower = line.to_ascii_lowercase();
    if !lower.contains("listening") {
        return None;
    }

    if let Some(start) = line.rfind('(') {
        if let Some(end_rel) = line[start + 1..].find(')') {
            let inner = &line[start + 1..start + 1 + end_rel];
            let digits: String = inner.chars().filter(|ch| ch.is_ascii_digit()).collect();
            if !digits.is_empty() {
                if let Ok(pid) = digits.parse::<u32>() {
                    return Some(pid);
                }
            }
        }
    }
    None
}

fn detect_managed_gateway_listener_pid(
    runtime: &::agent_ruler::config::RuntimeState,
) -> Option<u32> {
    let managed_home = managed_openclaw_home(runtime);
    let pid = find_managed_gateway_listener_pid(&managed_home)
        .ok()
        .flatten()?;
    if process_exists(pid) {
        Some(pid)
    } else {
        None
    }
}

fn recent_log_excerpt_since(log_path: &Path, offset: u64, max_lines: usize) -> String {
    let Ok(bytes) = fs::read(log_path) else {
        return String::new();
    };
    let start = usize::try_from(offset).ok().unwrap_or(0).min(bytes.len());
    let Ok(chunk) = std::str::from_utf8(&bytes[start..]) else {
        return String::new();
    };
    let lines: Vec<&str> = chunk.lines().collect();
    if lines.is_empty() {
        return String::new();
    }
    lines
        .into_iter()
        .rev()
        .take(max_lines)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_gateway_pid_record(
    runtime: &::agent_ruler::config::RuntimeState,
    pid: u32,
    launcher_pid: u32,
    log_path: &Path,
    cmd: &[String],
) -> Result<()> {
    let record_path = gateway_pid_record_file(runtime);
    if let Some(parent) = record_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let payload = serde_json::json!({
        "pid": pid,
        "launcher_pid": launcher_pid,
        "started_at": chrono::Utc::now().to_rfc3339(),
        "log_file": log_path.to_string_lossy().to_string(),
        "command": cmd,
    });
    fs::write(
        &record_path,
        serde_json::to_string_pretty(&payload).context("serialize gateway pid record")?,
    )
    .with_context(|| format!("write {}", record_path.display()))
}

/// Stop the managed gateway using the recorded PID so we never accidentally kill the wrong process.
fn stop_managed_background_gateway(runtime: &::agent_ruler::config::RuntimeState) -> Result<bool> {
    let record_path = gateway_pid_record_file(runtime);
    if !record_path.exists() {
        println!(
            "gateway stop: no managed gateway pid record found ({}).",
            record_path.display()
        );
        println!("gateway stop: nothing to stop in managed detached mode.");
        return Ok(true);
    }

    let raw = fs::read_to_string(&record_path)
        .with_context(|| format!("read {}", record_path.display()))?;
    let payload: serde_json::Value =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", record_path.display()))?;
    let Some(pid) = payload
        .get("pid")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as u32)
    else {
        remove_if_exists(&record_path)?;
        println!(
            "gateway stop: invalid pid record at {}; cleared stale file.",
            record_path.display()
        );
        return Ok(true);
    };
    let launcher_pid = payload
        .get("launcher_pid")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as u32);

    if !process_exists(pid) {
        stop_managed_background_launcher(launcher_pid, pid);
        remove_if_exists(&record_path)?;
        println!(
            "gateway stop: recorded pid {} is not running; cleared pid record.",
            pid
        );
        return Ok(true);
    }

    println!(
        "gateway stop: stopping managed gateway process (pid: {}).",
        pid
    );
    let stopped = kill_gateway_process(pid)?;
    stop_managed_background_launcher(launcher_pid, pid);
    if stopped {
        // Only clear pid record after confirmed stop to avoid losing stop target
        // when TERM/KILL fails and operator needs manual remediation.
        remove_if_exists(&record_path)?;
    }
    Ok(stopped)
}

fn kill_gateway_process(pid: u32) -> Result<bool> {
    kill_process_with_retry(
        pid,
        "managed background OpenClaw gateway process stopped (pid: {})",
        "gateway stop: managed background process pid {} is still alive after TERM/KILL attempts.",
    )
}

fn kill_openclaw_bridge_process(pid: u32) -> Result<bool> {
    kill_process_with_retry(
        pid,
        "managed OpenClaw channel bridge process stopped (pid: {})",
        "gateway stop: managed OpenClaw channel bridge pid {} is still alive after TERM/KILL attempts.",
    )
}

fn kill_background_launcher_process(pid: u32) -> Result<bool> {
    kill_process_with_retry(
        pid,
        "managed background launcher process stopped (pid: {})",
        "gateway stop: managed background launcher pid {} is still alive after TERM/KILL attempts.",
    )
}

fn kill_process_with_retry(
    pid: u32,
    success_template: &str,
    failure_template: &str,
) -> Result<bool> {
    if !process_exists(pid) {
        print_pid_message(success_template, pid, false);
        return Ok(true);
    }

    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
    for _ in 0..40 {
        if !process_exists(pid) {
            print_pid_message(success_template, pid, false);
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let _ = Command::new("kill")
        .arg("-KILL")
        .arg(pid.to_string())
        .status();
    for _ in 0..20 {
        if !process_exists(pid) {
            print_pid_message(success_template, pid, false);
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    print_pid_message(failure_template, pid, true);
    Ok(false)
}

fn print_pid_message(template: &str, pid: u32, stderr: bool) {
    let message = template.replace("{}", &pid.to_string());
    if stderr {
        eprintln!("{message}");
    } else {
        println!("{message}");
    }
}

fn stop_managed_openclaw_bridge(runtime: &::agent_ruler::config::RuntimeState) -> Result<bool> {
    let pid_file = openclaw_bridge_pid_file(runtime);
    if !pid_file.exists() {
        return Ok(true);
    }

    let raw =
        fs::read_to_string(&pid_file).with_context(|| format!("read {}", pid_file.display()))?;
    let pid = match raw.trim().parse::<u32>() {
        Ok(value) => value,
        Err(_) => {
            remove_if_exists(&pid_file)?;
            eprintln!(
                "bridge stop: invalid pid record at {}; cleared stale file.",
                pid_file.display()
            );
            return Ok(true);
        }
    };

    if !process_exists(pid) {
        remove_if_exists(&pid_file)?;
        eprintln!(
            "bridge stop: recorded pid {} is not running; cleared pid record.",
            pid
        );
        return Ok(true);
    }

    eprintln!(
        "bridge stop: stopping managed OpenClaw channel bridge process (pid: {}).",
        pid
    );
    let stopped = kill_openclaw_bridge_process(pid)?;
    if stopped {
        remove_if_exists(&pid_file)?;
    }
    Ok(stopped)
}

fn stop_managed_background_launcher(launcher_pid: Option<u32>, gateway_pid: u32) {
    if let Some(pid) = launcher_pid.filter(|value| *value != gateway_pid) {
        match kill_background_launcher_process(pid) {
            Ok(false) => eprintln!(
                "gateway stop: managed launcher pid {} did not exit cleanly.",
                pid
            ),
            Err(err) => eprintln!(
                "gateway stop: unable to stop managed launcher pid {}: {err}",
                pid
            ),
            Ok(true) => {}
        }
    }
}

fn process_exists(pid: u32) -> bool {
    let proc_path_buf = format!("/proc/{pid}");
    let proc_path = Path::new(&proc_path_buf);
    if !proc_path.exists() {
        return false;
    }
    let stat_path = proc_path.join("stat");
    if let Ok(stat_raw) = fs::read_to_string(stat_path) {
        let parts: Vec<&str> = stat_raw.split_whitespace().collect();
        if parts.get(2) == Some(&"Z") {
            // Zombie processes are already dead and waiting for parent reaping.
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::{
        kill_process_with_retry, network_policy_allows_host, openclaw_command_needs_preflight_api,
        parse_gateway_pid_from_listening_line, parse_gateway_pid_from_log_line,
        parse_gateway_pid_from_log_since, parse_http_status_code, process_exists,
        stop_managed_background_launcher,
    };
    use ::agent_ruler::config::NetworkRules;

    #[test]
    fn network_policy_blocks_telegram_when_not_explicitly_allowed() {
        let rules = NetworkRules {
            default_deny: true,
            allowlist_hosts: vec!["github.com".to_string()],
            require_approval_for_post: true,
            denylist_hosts: Vec::new(),
            invert_allowlist: false,
            invert_denylist: false,
        };
        assert!(
            !network_policy_allows_host(&rules, "api.telegram.org"),
            "telegram host should be blocked when default-deny has no explicit allow"
        );
    }

    #[test]
    fn network_policy_allows_telegram_when_explicitly_allowlisted() {
        let rules = NetworkRules {
            default_deny: true,
            allowlist_hosts: vec!["api.telegram.org".to_string()],
            require_approval_for_post: true,
            denylist_hosts: Vec::new(),
            invert_allowlist: false,
            invert_denylist: false,
        };
        assert!(
            network_policy_allows_host(&rules, "api.telegram.org"),
            "telegram host should be allowed when explicitly allowlisted"
        );
    }

    #[test]
    fn network_policy_allows_open_egress_when_lists_empty_and_not_default_deny() {
        let rules = NetworkRules {
            default_deny: false,
            allowlist_hosts: Vec::new(),
            require_approval_for_post: true,
            denylist_hosts: Vec::new(),
            invert_allowlist: false,
            invert_denylist: false,
        };
        assert!(
            network_policy_allows_host(&rules, "api.telegram.org"),
            "telegram host should be allowed in open policy mode"
        );
    }

    #[test]
    fn openclaw_preflight_requirement_detects_tool_capable_commands() {
        let agent = vec!["openclaw".to_string(), "agent".to_string()];
        assert!(openclaw_command_needs_preflight_api(&agent));

        let gateway_run = vec![
            "openclaw".to_string(),
            "gateway".to_string(),
            "run".to_string(),
        ];
        assert!(openclaw_command_needs_preflight_api(&gateway_run));

        let env_prefixed = vec![
            "env".to_string(),
            "OPENCLAW_HOME=/tmp/openclaw".to_string(),
            "openclaw".to_string(),
            "tui".to_string(),
        ];
        assert!(openclaw_command_needs_preflight_api(&env_prefixed));
    }

    #[test]
    fn openclaw_preflight_requirement_skips_non_tool_commands() {
        let stop = vec![
            "openclaw".to_string(),
            "gateway".to_string(),
            "stop".to_string(),
        ];
        assert!(!openclaw_command_needs_preflight_api(&stop));

        let status = vec![
            "openclaw".to_string(),
            "gateway".to_string(),
            "status".to_string(),
        ];
        assert!(!openclaw_command_needs_preflight_api(&status));

        let plugins = vec![
            "openclaw".to_string(),
            "plugins".to_string(),
            "list".to_string(),
        ];
        assert!(!openclaw_command_needs_preflight_api(&plugins));
    }

    #[test]
    fn preflight_probe_parses_http_status_code_from_response() {
        let response = b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n";
        assert_eq!(parse_http_status_code(response), Some(404));
    }

    #[test]
    fn preflight_probe_rejects_non_http_payloads() {
        let response = b"not-an-http-response";
        assert_eq!(parse_http_status_code(response), None);
    }

    #[test]
    fn kill_process_with_retry_stops_running_process() {
        let mut child = Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("spawn sleep");
        assert!(
            process_exists(child.id()),
            "spawned process should be alive before stop"
        );

        let stopped = kill_process_with_retry(
            child.id(),
            "process stopped (pid: {})",
            "process still alive (pid: {})",
        )
        .expect("kill process");

        assert!(stopped, "expected stop helper to terminate process");
        let _ = child.wait();
    }

    #[test]
    fn stop_managed_background_launcher_terminates_launcher_pid() {
        let mut launcher = Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("spawn launcher");
        assert!(
            process_exists(launcher.id()),
            "launcher process should be alive before stop"
        );

        stop_managed_background_launcher(Some(launcher.id()), 999_999);
        for _ in 0..20 {
            if !process_exists(launcher.id()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(
            !process_exists(launcher.id()),
            "launcher pid should be terminated by stop helper"
        );
        let _ = launcher.wait();
    }

    #[test]
    fn gateway_pid_parser_accepts_pid_prefix_format() {
        let line = "[gateway] listening on ws://127.0.0.1:18789 (PID12345)";
        assert_eq!(parse_gateway_pid_from_log_line(line), Some(12345));
    }

    #[test]
    fn gateway_pid_parser_accepts_parenthesized_digits_format() {
        let line = "[gateway] listening on ws://127.0.0.1:18789 (12345)";
        assert_eq!(parse_gateway_pid_from_listening_line(line), Some(12345));
    }

    #[test]
    fn gateway_pid_parser_ignores_non_listening_bridge_pid_lines() {
        let path = std::env::temp_dir().join(format!(
            "agent-ruler-gateway-log-{}.log",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "bridge diagnostics: managed OpenClaw channel bridge started (pid: 32254, inbound: 127.0.0.1:4661, log: /tmp/bridge.log).\n",
        )
        .expect("write test gateway log");
        assert_eq!(parse_gateway_pid_from_log_since(&path, 0), None);
        let _ = std::fs::remove_file(path);
    }
}
