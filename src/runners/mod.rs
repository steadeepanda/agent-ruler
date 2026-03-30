//! Runner adapter abstraction and runner-missing reconciliation.
//!
//! This module centralizes project-local runner association state. It must
//! preserve two safety properties:
//! - managed paths stay scoped under runtime root and can be removed safely,
//! - host runner data is never modified by missing-runner remediation choices.

use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{save_config, AppConfig, RuntimeState, CONFIG_FILE_NAME};
use crate::helpers::runners::openclaw::setup_config::runner_api_base_url;
use crate::utils::resolve_command_path;

pub mod claudecode;
pub mod openclaw;
pub mod opencode;

/// Runtime-local container for all runner-managed artifacts.
pub const RUNTIME_USER_DATA_DIR_NAME: &str = "user_data";
/// Runtime-local parent for generic runner-managed state.
pub const RUNTIME_RUNNERS_DIR_NAME: &str = "runners";
/// Default managed OpenClaw home directory name under `user_data`.
pub const OPENCLAW_HOME_DIR_NAME: &str = "openclaw_home";
/// Default managed workspace directory name under `user_data`.
pub const OPENCLAW_WORKSPACE_DIR_NAME: &str = "openclaw_workspace";

/// Supported runner kinds that can be associated with a project runtime.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunnerKind {
    Openclaw,
    Claudecode,
    Opencode,
}

impl RunnerKind {
    /// Stable identifier used in config, API filters, and UI labels.
    pub fn id(self) -> &'static str {
        match self {
            Self::Openclaw => "openclaw",
            Self::Claudecode => "claudecode",
            Self::Opencode => "opencode",
        }
    }

    /// Parse stable runner identifier.
    pub fn from_id(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "openclaw" => Some(Self::Openclaw),
            "claudecode" => Some(Self::Claudecode),
            "opencode" => Some(Self::Opencode),
            _ => None,
        }
    }

    /// Human-friendly name for CLI/UI messages.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Openclaw => "OpenClaw",
            Self::Claudecode => "Claude Code",
            Self::Opencode => "OpenCode",
        }
    }

    /// Executable name expected in PATH for this runner.
    pub fn executable_name(self) -> &'static str {
        match self {
            Self::Openclaw => "openclaw",
            Self::Claudecode => "claude",
            Self::Opencode => "opencode",
        }
    }
}

/// Persisted project-to-runner mapping.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerAssociation {
    pub kind: RunnerKind,
    pub managed_home: PathBuf,
    pub managed_workspace: PathBuf,
    #[serde(default)]
    pub integrations: Vec<String>,
    #[serde(default)]
    pub missing: RunnerMissingState,
}

/// Sticky state recorded when a configured runner executable is missing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerMissingState {
    #[serde(default)]
    pub executable_missing: bool,
    #[serde(default)]
    pub decision: Option<RunnerMissingDecision>,
}

/// Operator choice for missing runner remediation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunnerMissingDecision {
    KeepData,
    DeleteData,
    RerunSetup,
}

/// Reconciliation outcome used by CLI command handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerAvailabilityState {
    NotConfigured,
    Available,
    MissingUnresolved,
    MissingKept,
    MissingReconfigure,
    Removed,
}

/// Controls interactive prompting and output channel during runner checks.
#[derive(Debug, Clone, Copy)]
pub struct RunnerCheckOptions {
    pub allow_prompt: bool,
    pub emit_to_stderr: bool,
}

impl Default for RunnerCheckOptions {
    fn default() -> Self {
        Self {
            allow_prompt: true,
            emit_to_stderr: false,
        }
    }
}

/// Discovered host-side runner installation metadata.
#[derive(Debug, Clone)]
pub struct HostInstall {
    pub home: PathBuf,
    pub detected_by: String,
}

/// Runtime-local directories provisioned for runner execution.
#[derive(Debug, Clone)]
pub struct ProvisionedPaths {
    pub managed_home: PathBuf,
    pub managed_workspace: PathBuf,
}

/// Summary of optional host import performed during setup.
#[derive(Debug, Clone, Default)]
pub struct ImportReport {
    pub imported: bool,
    pub copied_items: Vec<String>,
    pub snapshot_path: Option<PathBuf>,
    pub imported_config: Option<serde_json::Value>,
    pub expected_auth_profiles: bool,
    pub expected_auth_store: bool,
    pub expected_telegram_token: bool,
    pub expected_model_primary: Option<String>,
    pub cloned_configs: Vec<String>,
}

/// Optional setup integration that can mutate managed runner config/runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntegrationOption {
    pub id: &'static str,
    pub label: &'static str,
    pub detail: &'static str,
}

/// Chosen integration option persisted with runner association.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationSelection {
    pub id: String,
}

impl IntegrationSelection {
    /// Build an integration selection by ID.
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

/// Adapter contract for runner-specific setup, import, and validation behavior.
pub trait RunnerAdapter {
    fn kind(&self) -> RunnerKind;
    fn display_name(&self) -> &'static str;
    fn detect_host_install(&self, host_hint: Option<&Path>) -> Result<Option<HostInstall>>;
    fn provision_project_paths(&self, runtime: &RuntimeState) -> Result<ProvisionedPaths>;
    fn optional_import_from_host(
        &self,
        host_install: Option<&HostInstall>,
        paths: &ProvisionedPaths,
        import_from_host: bool,
    ) -> Result<ImportReport>;
    fn write_runner_config(
        &self,
        runtime: &RuntimeState,
        config: &mut AppConfig,
        paths: &ProvisionedPaths,
        import_report: &ImportReport,
        integrations: &[IntegrationSelection],
    ) -> Result<()>;
    fn validate(&self, config: &AppConfig) -> Result<()>;
    fn print_next_steps(&self, runtime: &RuntimeState, config: &AppConfig);
    fn integration_options(&self) -> &'static [IntegrationOption] {
        &[]
    }
}

/// Returns true when `cmd` targets the currently configured project runner kind.
pub fn configured_runner_targets_command(runtime: &RuntimeState, cmd: &[String]) -> bool {
    let Some(association) = runtime.config.runner.as_ref() else {
        return false;
    };
    command_runner_kind(cmd) == Some(association.kind)
}

/// Resolve runner kind from command token (supports `env KEY=... <runner> ...`).
pub fn command_runner_kind(cmd: &[String]) -> Option<RunnerKind> {
    let command_token = command_token(cmd)?;
    let path = Path::new(command_token);
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(command_token);
    [
        RunnerKind::Openclaw,
        RunnerKind::Claudecode,
        RunnerKind::Opencode,
    ]
    .into_iter()
    .find(|kind| name == kind.executable_name())
}

/// Wrap command with runner-specific environment overrides when needed.
///
/// For OpenClaw this enforces managed `OPENCLAW_HOME`, even if caller provided
/// another value. This keeps command execution inside project-local state.
pub fn apply_runner_env_overrides(runtime: &RuntimeState, cmd: &[String]) -> Vec<String> {
    let Some(association) = runtime.config.runner.as_ref() else {
        return cmd.to_vec();
    };
    if !command_targets_runner_kind(cmd, association.kind) {
        return cmd.to_vec();
    }

    let overrides = runner_env_overrides(
        association.kind,
        &association.managed_home,
        &runtime.config.ui_bind,
        runtime.config.approval_wait_timeout_secs,
    );
    if overrides.is_empty() {
        return cmd.to_vec();
    }

    let mut sanitized = cmd.to_vec();
    for (key, _) in &overrides {
        sanitized = strip_env_assignment(&sanitized, key);
    }

    let mut wrapped = Vec::with_capacity(sanitized.len() + overrides.len() + 1);
    wrapped.push("env".to_string());
    for (key, value) in overrides {
        wrapped.push(format!("{key}={value}"));
    }
    wrapped.extend(sanitized);
    wrapped
}

pub fn apply_runner_env_to_command(
    command: &mut Command,
    kind: RunnerKind,
    managed_home: &Path,
    ui_bind: &str,
    approval_wait_timeout_secs: u64,
) {
    for (key, value) in
        runner_env_overrides(kind, managed_home, ui_bind, approval_wait_timeout_secs)
    {
        command.env(key, value);
    }
}

/// Resolve the effective Zone 0 workspace for a command-targeted runner.
///
/// This keeps runner execution cwd aligned with the workspace roots used by
/// runner-aware UI and transfer APIs. If no configured runner matches the
/// command, fall back to the default runtime workspace.
pub fn workspace_root_for_command(runtime: &RuntimeState, cmd: &[String]) -> PathBuf {
    let Some(kind) = command_runner_kind(cmd) else {
        return runtime.config.workspace.clone();
    };

    crate::helpers::workspace_root_for_runner(runtime, Some(kind))
}

pub fn reconcile_runner_executable(
    runtime: &mut RuntimeState,
    command_name: &str,
) -> Result<RunnerAvailabilityState> {
    reconcile_runner_executable_with_options(runtime, command_name, RunnerCheckOptions::default())
}

/// Reconcile missing-runner state and optionally prompt for remediation choice.
pub fn reconcile_runner_executable_with_options(
    runtime: &mut RuntimeState,
    command_name: &str,
    options: RunnerCheckOptions,
) -> Result<RunnerAvailabilityState> {
    reconcile_runner_executable_with(
        runtime,
        command_name,
        |name| resolve_command_path(name).is_some(),
        None,
        options,
    )
}

/// Remove current project runner association and managed directories.
pub fn remove_configured_runner(runtime: &mut RuntimeState, expected: RunnerKind) -> Result<bool> {
    let Some(existing) = runtime.config.runner.clone() else {
        return Ok(false);
    };
    if existing.kind != expected {
        return Ok(false);
    }

    remove_managed_runner_paths(&runtime.config.runtime_root, &existing)?;
    runtime.config.runner = None;
    persist_runtime_config(&runtime.config)?;
    Ok(true)
}

fn reconcile_runner_executable_with<F>(
    runtime: &mut RuntimeState,
    command_name: &str,
    executable_probe: F,
    forced_decision: Option<RunnerMissingDecision>,
    options: RunnerCheckOptions,
) -> Result<RunnerAvailabilityState>
where
    F: Fn(&str) -> bool,
{
    let Some(existing) = runtime.config.runner.as_ref() else {
        return Ok(RunnerAvailabilityState::NotConfigured);
    };
    let kind = existing.kind;
    let executable_present = executable_probe(kind.executable_name());

    if executable_present {
        let should_clear = runtime
            .config
            .runner
            .as_ref()
            .map(|runner| runner.missing.executable_missing || runner.missing.decision.is_some())
            .unwrap_or(false);
        if should_clear {
            if let Some(runner) = runtime.config.runner.as_mut() {
                runner.missing = RunnerMissingState::default();
            }
            persist_runtime_config(&runtime.config)?;
            print_line(
                options.emit_to_stderr,
                "runner check: {} is available again; cleared missing-runner reminder",
                &[kind.executable_name()],
            );
        }
        return Ok(RunnerAvailabilityState::Available);
    }

    let mut changed = false;
    if let Some(runner) = runtime.config.runner.as_mut() {
        if !runner.missing.executable_missing {
            runner.missing.executable_missing = true;
            runner.missing.decision = None;
            changed = true;
        }
    }
    if changed {
        persist_runtime_config(&runtime.config)?;
    }

    // Missing-runner decisions are sticky per runtime: once operator picks a
    // branch (keep/delete/reconfigure), later commands reuse that decision
    // until executable returns or runner mapping is removed.
    let current_decision = runtime
        .config
        .runner
        .as_ref()
        .and_then(|runner| runner.missing.decision);
    if current_decision.is_none() {
        let decision = match forced_decision {
            Some(decision) => decision,
            None => prompt_missing_runner_decision(kind, command_name, options)?,
        };
        return apply_missing_decision(runtime, decision, command_name, options);
    }

    match current_decision {
        Some(RunnerMissingDecision::KeepData) => {
            print_keep_data_message(kind, command_name, options.emit_to_stderr);
            Ok(RunnerAvailabilityState::MissingKept)
        }
        Some(RunnerMissingDecision::RerunSetup) => {
            print_rerun_setup_message(kind, command_name, options.emit_to_stderr);
            Ok(RunnerAvailabilityState::MissingReconfigure)
        }
        Some(RunnerMissingDecision::DeleteData) => apply_missing_decision(
            runtime,
            RunnerMissingDecision::DeleteData,
            command_name,
            options,
        ),
        None => Ok(RunnerAvailabilityState::MissingUnresolved),
    }
}

fn apply_missing_decision(
    runtime: &mut RuntimeState,
    decision: RunnerMissingDecision,
    command_name: &str,
    options: RunnerCheckOptions,
) -> Result<RunnerAvailabilityState> {
    let Some(association) = runtime.config.runner.clone() else {
        return Ok(RunnerAvailabilityState::Removed);
    };

    match decision {
        RunnerMissingDecision::KeepData => {
            if let Some(runner) = runtime.config.runner.as_mut() {
                runner.missing.executable_missing = true;
                runner.missing.decision = Some(RunnerMissingDecision::KeepData);
            }
            persist_runtime_config(&runtime.config)?;
            print_keep_data_message(association.kind, command_name, options.emit_to_stderr);
            Ok(RunnerAvailabilityState::MissingKept)
        }
        RunnerMissingDecision::RerunSetup => {
            if let Some(runner) = runtime.config.runner.as_mut() {
                runner.missing.executable_missing = true;
                runner.missing.decision = Some(RunnerMissingDecision::RerunSetup);
            }
            persist_runtime_config(&runtime.config)?;
            print_rerun_setup_message(association.kind, command_name, options.emit_to_stderr);
            Ok(RunnerAvailabilityState::MissingReconfigure)
        }
        RunnerMissingDecision::DeleteData => {
            remove_managed_runner_paths(&runtime.config.runtime_root, &association)?;
            runtime.config.runner = None;
            persist_runtime_config(&runtime.config)?;
            print_line(
                options.emit_to_stderr,
                "runner data cleanup complete for {} (host runner state remains untouched)",
                &[association.kind.display_name()],
            );
            Ok(RunnerAvailabilityState::Removed)
        }
    }
}

fn prompt_missing_runner_decision(
    kind: RunnerKind,
    command_name: &str,
    options: RunnerCheckOptions,
) -> Result<RunnerMissingDecision> {
    print_line(
        options.emit_to_stderr,
        "runner check: project is configured for {}, but `{}` is missing from PATH while running `{}`",
        &[kind.display_name(), kind.executable_name(), command_name],
    );

    // In non-interactive flows (JSON status output, CI, or agent invocation),
    // default to keeping data to avoid destructive surprises.
    if !options.allow_prompt || !io::stdin().is_terminal() {
        print_line(
            options.emit_to_stderr,
            "non-interactive/structured output mode detected; defaulting to option 1 (keep data)",
            &[],
        );
        return Ok(RunnerMissingDecision::KeepData);
    }

    print_line(
        options.emit_to_stderr,
        "Choose how Agent Ruler should handle project-local runner data:",
        &[],
    );
    print_line(
        options.emit_to_stderr,
        "  1) Keep Ruler-managed runner data for this project (default)",
        &[],
    );
    print_line(
        options.emit_to_stderr,
        "  2) Delete Ruler-managed runner data for this project",
        &[],
    );
    print_line(
        options.emit_to_stderr,
        "  3) Re-run setup / change runner",
        &[],
    );
    print_line(
        options.emit_to_stderr,
        "Host runner state is never modified by these choices.",
        &[],
    );

    loop {
        let choice = read_line("Selection [1]: ")?;
        match choice.trim() {
            "" | "1" => return Ok(RunnerMissingDecision::KeepData),
            "2" => return Ok(RunnerMissingDecision::DeleteData),
            "3" => return Ok(RunnerMissingDecision::RerunSetup),
            _ => println!("invalid choice; enter 1, 2, or 3"),
        }
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

fn command_targets_runner_kind(cmd: &[String], kind: RunnerKind) -> bool {
    command_runner_kind(cmd) == Some(kind)
}

fn command_token(cmd: &[String]) -> Option<&str> {
    let first = cmd.first()?;
    let first_name = Path::new(first)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(first.as_str());
    if first_name != "env" {
        return Some(first.as_str());
    }

    for token in cmd.iter().skip(1) {
        if token.contains('=') {
            continue;
        }
        return Some(token.as_str());
    }
    None
}

fn strip_env_assignment(cmd: &[String], key: &str) -> Vec<String> {
    let Some(first) = cmd.first() else {
        return Vec::new();
    };
    let first_name = Path::new(first)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(first.as_str());
    if first_name != "env" {
        return cmd.to_vec();
    }

    let key_prefix = format!("{key}=");
    let mut output = Vec::with_capacity(cmd.len());
    output.push(first.clone());
    let mut index = 1usize;
    while index < cmd.len() {
        let token = &cmd[index];
        if token.contains('=') {
            if !token.starts_with(&key_prefix) {
                output.push(token.clone());
            }
            index += 1;
            continue;
        }

        output.extend(cmd[index..].iter().cloned());
        return output;
    }

    output
}

fn print_keep_data_message(kind: RunnerKind, command_name: &str, emit_to_stderr: bool) {
    print_line(
        emit_to_stderr,
        "runner reminder: `{}` is still missing during `{}`; keeping Ruler-managed {} data as requested",
        &[kind.executable_name(), command_name, kind.display_name()],
    );
    print_line(
        emit_to_stderr,
        "resolve with `agent-ruler setup` or `agent-ruler runner remove {}`",
        &[kind.id()],
    );
}

fn print_rerun_setup_message(kind: RunnerKind, command_name: &str, emit_to_stderr: bool) {
    print_line(
        emit_to_stderr,
        "runner reminder: `{}` is still missing during `{}`; run `agent-ruler setup` to reconfigure runner mapping",
        &[kind.executable_name(), command_name],
    );
    print_line(
        emit_to_stderr,
        "or remove managed data with `agent-ruler runner remove {}`",
        &[kind.id()],
    );
}

fn runner_env_overrides(
    kind: RunnerKind,
    managed_home: &Path,
    ui_bind: &str,
    approval_wait_timeout_secs: u64,
) -> Vec<(&'static str, String)> {
    let home = managed_home.to_string_lossy().to_string();
    let base_url = runner_api_base_url(ui_bind);
    let approval_wait = approval_wait_timeout_secs.clamp(1, 300).to_string();
    match kind {
        RunnerKind::Openclaw => vec![
            ("OPENCLAW_HOME", home.clone()),
            ("HOME", home.clone()),
            (
                "XDG_CONFIG_HOME",
                managed_home.join(".config").display().to_string(),
            ),
            (
                "XDG_DATA_HOME",
                managed_home.join(".local/share").display().to_string(),
            ),
            (
                "XDG_STATE_HOME",
                managed_home.join(".local/state").display().to_string(),
            ),
            (
                "XDG_CACHE_HOME",
                managed_home.join(".cache").display().to_string(),
            ),
        ],
        RunnerKind::Claudecode => vec![
            ("CLAUDE_CONFIG_DIR", home.clone()),
            ("HOME", home),
            ("AGENT_RULER_BASE_URL", base_url),
            (
                "AGENT_RULER_RUNNER_ID",
                RunnerKind::Claudecode.id().to_string(),
            ),
            ("AGENT_RULER_AUTO_WAIT_FOR_APPROVALS", "1".to_string()),
            (
                "AGENT_RULER_APPROVAL_WAIT_TIMEOUT_SECS",
                approval_wait.clone(),
            ),
        ],
        RunnerKind::Opencode => vec![
            ("HOME", home.clone()),
            (
                "XDG_CONFIG_HOME",
                managed_home.join(".config").display().to_string(),
            ),
            (
                "XDG_DATA_HOME",
                managed_home.join(".local/share").display().to_string(),
            ),
            (
                "XDG_STATE_HOME",
                managed_home.join(".local/state").display().to_string(),
            ),
            (
                "XDG_CACHE_HOME",
                managed_home.join(".cache").display().to_string(),
            ),
            ("AGENT_RULER_BASE_URL", base_url),
            (
                "AGENT_RULER_RUNNER_ID",
                RunnerKind::Opencode.id().to_string(),
            ),
            ("AGENT_RULER_AUTO_WAIT_FOR_APPROVALS", "1".to_string()),
            ("AGENT_RULER_APPROVAL_WAIT_TIMEOUT_SECS", approval_wait),
        ],
    }
}

fn remove_managed_runner_paths(runtime_root: &Path, runner: &RunnerAssociation) -> Result<()> {
    safe_remove_managed_path(runtime_root, &runner.managed_home)?;
    safe_remove_managed_path(runtime_root, &runner.managed_workspace)?;
    Ok(())
}

fn safe_remove_managed_path(runtime_root: &Path, target: &Path) -> Result<()> {
    // Cleanup is intentionally constrained to runtime root so a corrupted or
    // malicious config cannot trick runner removal into deleting arbitrary paths.
    if !target.starts_with(runtime_root) {
        return Err(anyhow!(
            "refusing to delete path outside runtime root: {}",
            target.display()
        ));
    }
    if target == runtime_root {
        return Err(anyhow!(
            "refusing to delete runtime root directly via runner cleanup"
        ));
    }
    if !target.exists() {
        return Ok(());
    }

    if target.is_dir() {
        fs::remove_dir_all(target).with_context(|| format!("remove {}", target.display()))?;
    } else {
        fs::remove_file(target).with_context(|| format!("remove {}", target.display()))?;
    }
    Ok(())
}

fn persist_runtime_config(config: &AppConfig) -> Result<()> {
    let path = config.state_dir.join(CONFIG_FILE_NAME);
    save_config(&path, config)
}

fn print_line(emit_to_stderr: bool, message: &str, args: &[&str]) {
    let rendered = if args.is_empty() {
        message.to_string()
    } else {
        format_message(message, args)
    };
    if emit_to_stderr {
        eprintln!("{rendered}");
    } else {
        println!("{rendered}");
    }
}

fn format_message(message: &str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut parts = message.split("{}");
    if let Some(first) = parts.next() {
        out.push_str(first);
    }
    for (idx, part) in parts.enumerate() {
        if let Some(arg) = args.get(idx) {
            out.push_str(arg);
        } else {
            out.push_str("{}");
        }
        out.push_str(part);
    }
    out
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::config::{init_layout, load_runtime};

    use super::{
        apply_runner_env_overrides, command_runner_kind, reconcile_runner_executable_with,
        remove_configured_runner, runner_api_base_url, workspace_root_for_command,
        RunnerAssociation, RunnerAvailabilityState, RunnerCheckOptions, RunnerKind,
        RunnerMissingDecision, RunnerMissingState,
    };

    #[test]
    fn missing_runner_decision_persists_until_executable_returns() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");
        init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");

        let mut runtime =
            load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");
        let managed_home = runtime
            .config
            .runtime_root
            .join("user_data")
            .join("openclaw_home");
        let managed_workspace = runtime.config.workspace.clone();
        runtime.config.runner = Some(RunnerAssociation {
            kind: RunnerKind::Openclaw,
            managed_home,
            managed_workspace,
            integrations: Vec::new(),
            missing: RunnerMissingState::default(),
        });

        let first = reconcile_runner_executable_with(
            &mut runtime,
            "status",
            |_| false,
            Some(RunnerMissingDecision::KeepData),
            RunnerCheckOptions::default(),
        )
        .expect("first reconcile");
        assert_eq!(first, RunnerAvailabilityState::MissingKept);
        let state_after_first = runtime
            .config
            .runner
            .as_ref()
            .expect("runner after first reconcile");
        assert!(state_after_first.missing.executable_missing);
        assert_eq!(
            state_after_first.missing.decision,
            Some(RunnerMissingDecision::KeepData)
        );

        let second = reconcile_runner_executable_with(
            &mut runtime,
            "status",
            |_| false,
            None,
            RunnerCheckOptions::default(),
        )
        .expect("second reconcile");
        assert_eq!(second, RunnerAvailabilityState::MissingKept);
        let state_after_second = runtime
            .config
            .runner
            .as_ref()
            .expect("runner after second reconcile");
        assert_eq!(
            state_after_second.missing.decision,
            Some(RunnerMissingDecision::KeepData)
        );

        let third = reconcile_runner_executable_with(
            &mut runtime,
            "status",
            |_| true,
            None,
            RunnerCheckOptions::default(),
        )
        .expect("third reconcile");
        assert_eq!(third, RunnerAvailabilityState::Available);
        let state_after_third = runtime
            .config
            .runner
            .as_ref()
            .expect("runner after third reconcile");
        assert!(!state_after_third.missing.executable_missing);
        assert!(state_after_third.missing.decision.is_none());
    }

    #[test]
    fn missing_runner_decision_is_scoped_per_runtime() {
        let project_a = tempdir().expect("project a");
        let runtime_a = tempdir().expect("runtime a");
        init_layout(project_a.path(), Some(runtime_a.path()), None, true).expect("init runtime a");
        let mut state_a =
            load_runtime(project_a.path(), Some(runtime_a.path())).expect("load runtime a");
        state_a.config.runner = Some(RunnerAssociation {
            kind: RunnerKind::Openclaw,
            managed_home: state_a
                .config
                .runtime_root
                .join("user_data")
                .join("openclaw_home"),
            managed_workspace: state_a.config.workspace.clone(),
            integrations: Vec::new(),
            missing: RunnerMissingState::default(),
        });

        let project_b = tempdir().expect("project b");
        let runtime_b = tempdir().expect("runtime b");
        init_layout(project_b.path(), Some(runtime_b.path()), None, true).expect("init runtime b");
        let mut state_b =
            load_runtime(project_b.path(), Some(runtime_b.path())).expect("load runtime b");
        state_b.config.runner = Some(RunnerAssociation {
            kind: RunnerKind::Openclaw,
            managed_home: state_b
                .config
                .runtime_root
                .join("user_data")
                .join("openclaw_home"),
            managed_workspace: state_b.config.workspace.clone(),
            integrations: Vec::new(),
            missing: RunnerMissingState::default(),
        });

        let _ = reconcile_runner_executable_with(
            &mut state_a,
            "status",
            |_| false,
            Some(RunnerMissingDecision::KeepData),
            RunnerCheckOptions::default(),
        )
        .expect("reconcile runtime a");

        assert_eq!(
            state_a
                .config
                .runner
                .as_ref()
                .and_then(|runner| runner.missing.decision),
            Some(RunnerMissingDecision::KeepData)
        );
        assert_eq!(
            state_b
                .config
                .runner
                .as_ref()
                .and_then(|runner| runner.missing.decision),
            None,
            "runtime b should keep independent missing-runner state"
        );
    }

    #[test]
    fn remove_runner_clears_association_and_missing_state() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");
        init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
        let mut runtime =
            load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");
        runtime.config.runner = Some(RunnerAssociation {
            kind: RunnerKind::Openclaw,
            managed_home: runtime
                .config
                .runtime_root
                .join("user_data")
                .join("openclaw_home"),
            managed_workspace: runtime.config.workspace.clone(),
            integrations: Vec::new(),
            missing: RunnerMissingState {
                executable_missing: true,
                decision: Some(RunnerMissingDecision::KeepData),
            },
        });

        let removed = remove_configured_runner(&mut runtime, RunnerKind::Openclaw)
            .expect("remove configured runner");
        assert!(removed, "runner association should be removed");
        assert!(
            runtime.config.runner.is_none(),
            "runner association should be cleared"
        );
    }

    #[test]
    fn apply_runner_env_overrides_enforces_claudecode_managed_home() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");
        init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
        let mut runtime =
            load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");
        let managed_home = runtime
            .config
            .runtime_root
            .join("user_data/runners/claudecode/home");
        let managed_workspace = runtime
            .config
            .runtime_root
            .join("user_data/runners/claudecode/workspace");
        runtime.config.runner = Some(RunnerAssociation {
            kind: RunnerKind::Claudecode,
            managed_home: managed_home.clone(),
            managed_workspace,
            integrations: Vec::new(),
            missing: RunnerMissingState::default(),
        });

        let cmd = vec![
            "env".to_string(),
            "CLAUDE_CONFIG_DIR=/tmp/host-claude".to_string(),
            "HOME=/tmp/host-home".to_string(),
            "claude".to_string(),
            "-p".to_string(),
            "hello".to_string(),
        ];
        let wrapped = apply_runner_env_overrides(&runtime, &cmd);
        let joined = wrapped.join(" ");
        let managed_home_text = managed_home.to_string_lossy().to_string();
        let expected_base_url = runner_api_base_url(&runtime.config.ui_bind);

        assert!(joined.contains(&format!("CLAUDE_CONFIG_DIR={managed_home_text}")));
        assert!(joined.contains(&format!("HOME={managed_home_text}")));
        assert!(joined.contains(&format!("AGENT_RULER_BASE_URL={expected_base_url}")));
        assert!(joined.contains("AGENT_RULER_APPROVAL_WAIT_TIMEOUT_SECS="));
        assert!(
            !joined.contains("CLAUDE_CONFIG_DIR=/tmp/host-claude"),
            "host override should be stripped"
        );
        assert!(
            !joined.contains("HOME=/tmp/host-home"),
            "host override should be stripped"
        );
    }

    #[test]
    fn apply_runner_env_overrides_enforces_openclaw_managed_home_and_xdg() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");
        init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
        let mut runtime =
            load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");
        let managed_home = runtime.config.runtime_root.join("user_data/openclaw_home");
        let managed_workspace = runtime.config.workspace.clone();
        runtime.config.runner = Some(RunnerAssociation {
            kind: RunnerKind::Openclaw,
            managed_home: managed_home.clone(),
            managed_workspace,
            integrations: Vec::new(),
            missing: RunnerMissingState::default(),
        });

        let cmd = vec![
            "env".to_string(),
            "OPENCLAW_HOME=/tmp/host-openclaw".to_string(),
            "HOME=/tmp/host-home".to_string(),
            "XDG_CONFIG_HOME=/tmp/host-config".to_string(),
            "XDG_DATA_HOME=/tmp/host-data".to_string(),
            "XDG_STATE_HOME=/tmp/host-state".to_string(),
            "XDG_CACHE_HOME=/tmp/host-cache".to_string(),
            "openclaw".to_string(),
            "status".to_string(),
        ];
        let wrapped = apply_runner_env_overrides(&runtime, &cmd);
        let joined = wrapped.join(" ");
        let managed_home_text = managed_home.to_string_lossy().to_string();
        let expected_xdg = managed_home.join(".config").to_string_lossy().to_string();
        let expected_data = managed_home
            .join(".local/share")
            .to_string_lossy()
            .to_string();
        let expected_state = managed_home
            .join(".local/state")
            .to_string_lossy()
            .to_string();
        let expected_cache = managed_home.join(".cache").to_string_lossy().to_string();

        assert!(joined.contains(&format!("OPENCLAW_HOME={managed_home_text}")));
        assert!(joined.contains(&format!("HOME={managed_home_text}")));
        assert!(joined.contains(&format!("XDG_CONFIG_HOME={expected_xdg}")));
        assert!(joined.contains(&format!("XDG_DATA_HOME={expected_data}")));
        assert!(joined.contains(&format!("XDG_STATE_HOME={expected_state}")));
        assert!(joined.contains(&format!("XDG_CACHE_HOME={expected_cache}")));
        assert!(
            !joined.contains("OPENCLAW_HOME=/tmp/host-openclaw"),
            "host OPENCLAW_HOME override should be stripped"
        );
        assert!(
            !joined.contains("HOME=/tmp/host-home"),
            "host HOME override should be stripped"
        );
        assert!(
            !joined.contains("XDG_CONFIG_HOME=/tmp/host-config"),
            "host XDG config override should be stripped"
        );
        assert!(
            !joined.contains("XDG_DATA_HOME=/tmp/host-data"),
            "host XDG data override should be stripped"
        );
        assert!(
            !joined.contains("XDG_STATE_HOME=/tmp/host-state"),
            "host XDG state override should be stripped"
        );
        assert!(
            !joined.contains("XDG_CACHE_HOME=/tmp/host-cache"),
            "host XDG cache override should be stripped"
        );
    }

    #[test]
    fn apply_runner_env_overrides_enforces_opencode_managed_home_and_xdg() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");
        init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
        let mut runtime =
            load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");
        let managed_home = runtime
            .config
            .runtime_root
            .join("user_data/runners/opencode/home");
        let managed_workspace = runtime
            .config
            .runtime_root
            .join("user_data/runners/opencode/workspace");
        runtime.config.runner = Some(RunnerAssociation {
            kind: RunnerKind::Opencode,
            managed_home: managed_home.clone(),
            managed_workspace,
            integrations: Vec::new(),
            missing: RunnerMissingState::default(),
        });

        let cmd = vec![
            "env".to_string(),
            "HOME=/tmp/host-home".to_string(),
            "XDG_CONFIG_HOME=/tmp/host-config".to_string(),
            "XDG_DATA_HOME=/tmp/host-data".to_string(),
            "XDG_STATE_HOME=/tmp/host-state".to_string(),
            "XDG_CACHE_HOME=/tmp/host-cache".to_string(),
            "opencode".to_string(),
            "run".to_string(),
            "hello".to_string(),
        ];
        let wrapped = apply_runner_env_overrides(&runtime, &cmd);
        let joined = wrapped.join(" ");
        let managed_home_text = managed_home.to_string_lossy().to_string();
        let expected_xdg = managed_home.join(".config").to_string_lossy().to_string();
        let expected_data = managed_home
            .join(".local/share")
            .to_string_lossy()
            .to_string();
        let expected_state = managed_home
            .join(".local/state")
            .to_string_lossy()
            .to_string();
        let expected_cache = managed_home.join(".cache").to_string_lossy().to_string();
        let expected_base_url = runner_api_base_url(&runtime.config.ui_bind);

        assert!(joined.contains(&format!("HOME={managed_home_text}")));
        assert!(joined.contains(&format!("XDG_CONFIG_HOME={expected_xdg}")));
        assert!(joined.contains(&format!("XDG_DATA_HOME={expected_data}")));
        assert!(joined.contains(&format!("XDG_STATE_HOME={expected_state}")));
        assert!(joined.contains(&format!("XDG_CACHE_HOME={expected_cache}")));
        assert!(joined.contains(&format!("AGENT_RULER_BASE_URL={expected_base_url}")));
        assert!(joined.contains("AGENT_RULER_APPROVAL_WAIT_TIMEOUT_SECS="));
        assert!(
            !joined.contains("HOME=/tmp/host-home"),
            "host HOME override should be stripped"
        );
        assert!(
            !joined.contains("XDG_CONFIG_HOME=/tmp/host-config"),
            "host XDG config override should be stripped"
        );
        assert!(
            !joined.contains("XDG_DATA_HOME=/tmp/host-data"),
            "host XDG data override should be stripped"
        );
        assert!(
            !joined.contains("XDG_STATE_HOME=/tmp/host-state"),
            "host XDG state override should be stripped"
        );
        assert!(
            !joined.contains("XDG_CACHE_HOME=/tmp/host-cache"),
            "host XDG cache override should be stripped"
        );
    }

    #[test]
    fn workspace_root_for_command_uses_managed_workspace_for_claudecode() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");
        init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
        let mut runtime =
            load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");
        let managed_home = runtime
            .config
            .runtime_root
            .join("user_data/runners/claudecode/home");
        let managed_workspace = runtime
            .config
            .runtime_root
            .join("user_data/runners/claudecode/workspace");
        runtime.config.runner = Some(RunnerAssociation {
            kind: RunnerKind::Claudecode,
            managed_home,
            managed_workspace: managed_workspace.clone(),
            integrations: Vec::new(),
            missing: RunnerMissingState::default(),
        });

        let cmd = vec![
            "claude".to_string(),
            "-p".to_string(),
            "reply with exactly ok".to_string(),
        ];
        assert_eq!(
            workspace_root_for_command(&runtime, &cmd),
            managed_workspace
        );
    }

    #[test]
    fn command_runner_kind_detects_env_prefixed_commands() {
        let env_prefixed = vec![
            "env".to_string(),
            "FOO=bar".to_string(),
            "claude".to_string(),
            "-p".to_string(),
            "hello".to_string(),
        ];
        assert_eq!(
            command_runner_kind(&env_prefixed),
            Some(RunnerKind::Claudecode)
        );

        let unknown = vec!["bash".to_string(), "-lc".to_string(), "echo ok".to_string()];
        assert_eq!(command_runner_kind(&unknown), None);
    }
}
