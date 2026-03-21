//! Source: `src/ui.rs`
//! Owning flow: `/api/run/command`
//! Critical invariants:
//! - Runner commands issued through the UI API must execute inside the same
//!   managed runner home/governance context as `agent-ruler run -- ...`.
//! - This helper only prepares the command; it does not launch bridges or UI.

use std::path::Path;

use anyhow::{anyhow, Result};

use crate::config::RuntimeState;
use crate::helpers::runners::command_contract::normalize_runner_command;
use crate::runners::claudecode::{
    enforce_managed_settings_guard, ensure_managed_settings_seed, managed_auth_logged_in,
};
use crate::runners::opencode::{enforce_managed_governance_config_guard, ensure_managed_auth_seed};
use crate::runners::{apply_runner_env_overrides, command_runner_kind, RunnerKind};

const CLAUDECODE_GOVERNANCE_PLUGIN_RELATIVE: &str =
    "bridge/claudecode/claudecode-agent-ruler-tools";

pub fn prepare_ui_command(runtime: &RuntimeState, cmd: &[String]) -> Result<Vec<String>> {
    let Some(target_kind) = command_runner_kind(cmd) else {
        return Ok(cmd.to_vec());
    };

    let configured_kind = runtime.config.runner.as_ref().map(|runner| runner.kind);
    match configured_kind {
        Some(kind) if kind != target_kind => {
            return Err(anyhow!(
                "runner mismatch: runtime is configured for {}, but command targets {}. Run `agent-ruler setup` to switch runner mapping.",
                kind.display_name(),
                target_kind.display_name()
            ));
        }
        None => {
            return Err(anyhow!(
                "runner command `{}` requires setup before use in this project runtime. Run `agent-ruler setup` first.",
                target_kind.executable_name()
            ));
        }
        _ => {}
    }

    let mut prepared = cmd.to_vec();
    match target_kind {
        RunnerKind::Claudecode => {
            if claudecode_command_requires_managed_auth(&prepared) {
                let _ = ensure_managed_settings_seed(runtime);
            }
            let _ = enforce_managed_settings_guard(runtime);
            if claudecode_command_requires_managed_auth(&prepared) {
                if let Some(false) = managed_auth_logged_in(runtime)? {
                    return Err(anyhow!(
                        "Claude Code managed runtime has no usable auth/config. Re-run `agent-ruler setup` to refresh managed Claude settings, or run `agent-ruler run -- claude auth login` if you want OAuth login in this project runtime."
                    ));
                }
            }
            prepared = inject_claudecode_governance_plugin_dir(runtime, &prepared);
        }
        RunnerKind::Opencode => {
            let _ = ensure_managed_auth_seed(runtime);
            let _ = enforce_managed_governance_config_guard(runtime);
        }
        RunnerKind::Openclaw => {}
    }

    prepared = normalize_runner_command(&prepared);
    Ok(apply_runner_env_overrides(runtime, &prepared))
}

fn inject_claudecode_governance_plugin_dir(runtime: &RuntimeState, cmd: &[String]) -> Vec<String> {
    if !claudecode_command_needs_governance_plugin(cmd) {
        return cmd.to_vec();
    }

    let Some(exec_index) = command_exec_index(cmd) else {
        return cmd.to_vec();
    };
    let exec_name = Path::new(&cmd[exec_index])
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !exec_name.eq_ignore_ascii_case("claude") {
        return cmd.to_vec();
    }

    let plugin_dir = runtime
        .config
        .ruler_root
        .join(CLAUDECODE_GOVERNANCE_PLUGIN_RELATIVE);
    if !plugin_dir.is_dir() {
        return cmd.to_vec();
    }

    let tail = &cmd[exec_index + 1..];
    let has_plugin_dir = tail.iter().any(|token| {
        token == "--plugin-dir" || token == "--plugin-dir=" || token.starts_with("--plugin-dir=")
    });
    let inject_after_subcommand =
        matches!(tail.first().map(String::as_str), Some("remote-control"));
    let insert_index = if inject_after_subcommand {
        exec_index + 2
    } else {
        exec_index + 1
    };

    let mut normalized = Vec::with_capacity(cmd.len() + 2);
    normalized.extend_from_slice(&cmd[..insert_index]);
    if !has_plugin_dir {
        normalized.push("--plugin-dir".to_string());
        normalized.push(plugin_dir.to_string_lossy().to_string());
    }
    normalized.extend_from_slice(&cmd[insert_index..]);
    normalized
}

fn claudecode_command_requires_managed_auth(cmd: &[String]) -> bool {
    let tokens = command_tokens_without_env_prefix(cmd);
    if tokens.first().copied() != Some("claude") {
        return false;
    }
    if runner_help_or_version(&tokens[1..]) {
        return false;
    }

    let Some(mode) = tokens.get(1).copied() else {
        return true;
    };
    if mode == "auth" || mode == "setup-token" {
        return false;
    }
    if matches!(
        mode,
        "doctor" | "install" | "update" | "upgrade" | "mcp" | "plugin" | "agents"
    ) {
        return false;
    }
    true
}

fn claudecode_command_needs_governance_plugin(cmd: &[String]) -> bool {
    let tokens = command_tokens_without_env_prefix(cmd);
    if tokens.first().copied() != Some("claude") {
        return false;
    }
    if runner_help_or_version(&tokens[1..]) {
        return false;
    }

    if tokens
        .iter()
        .skip(1)
        .any(|token| *token == "-p" || *token == "--print")
    {
        return true;
    }

    matches!(tokens.get(1).copied(), Some("remote-control"))
}

fn runner_help_or_version(tokens: &[&str]) -> bool {
    tokens
        .iter()
        .any(|token| matches!(*token, "-h" | "--help" | "-v" | "--version"))
}

fn command_tokens_without_env_prefix(cmd: &[String]) -> Vec<&str> {
    let Some(exec_index) = command_exec_index(cmd) else {
        return Vec::new();
    };
    cmd[exec_index..].iter().map(String::as_str).collect()
}

fn command_exec_index(cmd: &[String]) -> Option<usize> {
    if cmd.is_empty() {
        return None;
    }
    if cmd.first().map(String::as_str) != Some("env") {
        return Some(0);
    }
    for (index, token) in cmd.iter().enumerate().skip(1) {
        if token.contains('=') {
            continue;
        }
        return Some(index);
    }
    None
}
