//! Command execution with security confinement.
//!
//! This module provides the runner for executing agent commands within a
//! sandboxed environment. On Linux, it uses bubblewrap (bwrap) for namespace-based
//! isolation.
//!
//! # Key Components
//!
//! - [`run_confined`] - Main entry point for running commands
//! - [`confinement`] - Platform-specific sandboxing (Linux bubblewrap)
//! - [`preflight`] - Pre-execution policy checks and approval handling
//!
//! # Security Model
//!
//! Commands are executed with the following restrictions:
//! - Filesystem access limited to workspace and allowed paths
//! - Network access controlled by policy allowlist
//! - No access to system-critical paths (/etc, /usr, /bin)
//! - Downloaded files are quarantined before execution
//!
//! # Linux Confinement (bubblewrap)
//!
//! On Linux, commands run in a bubblewrap sandbox with:
//! - Separate mount namespace
//! - Read-only bind mounts for system directories
//! - Writable bind mounts only for workspace
//! - Network namespace (optional, based on policy)
//!
//! # Degraded Mode
//!
//! If bubblewrap is unavailable, the system can run in "degraded" mode with
//! reduced isolation. This is configurable and logged prominently.
//!
//! # Platform Support
//!
//! - Linux: Full support via bubblewrap
//! - Windows/macOS: Not yet implemented (stubs only)
//!
//! # Tests
//!
//! See `/tests/linux_runtime_integration.rs` and `/tests/command_execution_elevation.rs`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;

use crate::approvals::ApprovalStore;
use crate::config::RuntimeState;
use crate::model::{
    ActionKind, ActionRequest, Decision, ProcessContext, ReasonCode, Receipt, Verdict,
};
use crate::policy::PolicyEngine;
use crate::receipts::ReceiptStore;

mod confinement;
mod preflight;

use preflight::{
    finalize_with_approval, preflight_elevation_actions, preflight_interpreter_exec_actions,
    preflight_network_egress_actions, preflight_persistence_actions, preflight_utility_actions,
};

const MANAGED_CHILD_PID_FILE_ENV: &str = "AGENT_RULER_MANAGED_CHILD_PID_FILE";

/// Result of a confined command execution.
#[derive(Debug, Clone)]
pub struct RunResult {
    /// Exit code from the command
    pub exit_code: i32,
    /// Confinement method used (e.g., "linux-bwrap", "degraded")
    pub confinement: String,
    /// Captured stdout
    pub stdout: String,
    /// Captured stderr
    pub stderr: String,
}

pub fn run_confined(
    cmd: &[String],
    runtime: &RuntimeState,
    engine: &PolicyEngine,
    approvals: &ApprovalStore,
    receipts: &ReceiptStore,
) -> Result<RunResult> {
    if cmd.is_empty() {
        return Err(anyhow!("empty command"));
    }

    let executable =
        crate::utils::resolve_command_path(&cmd[0]).unwrap_or_else(|| PathBuf::from(&cmd[0]));
    let mut metadata = BTreeMap::new();
    metadata.insert("argv".to_string(), cmd.join(" "));

    let exec_request = ActionRequest {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        kind: ActionKind::Execute,
        operation: "run_start".to_string(),
        path: Some(executable.clone()),
        secondary_path: None,
        host: None,
        metadata,
        process: ProcessContext {
            pid: std::process::id(),
            ppid: None,
            command: cmd.join(" "),
            process_tree: vec![std::process::id()],
        },
    };

    preflight_elevation_actions(cmd, runtime, approvals, receipts)?;
    preflight_utility_actions(cmd, runtime, engine, approvals, receipts)?;
    preflight_persistence_actions(cmd, runtime, engine, approvals, receipts)?;
    preflight_interpreter_exec_actions(cmd, runtime, engine, approvals, receipts)?;
    preflight_network_egress_actions(cmd, runtime, engine, approvals, receipts)?;

    let (decision, zone) = engine.evaluate(&exec_request);
    let final_decision = finalize_with_approval(decision, approvals, &exec_request)?;

    match final_decision.verdict {
        Verdict::Allow => {
            append_receipt(
                receipts,
                runtime,
                exec_request.clone(),
                final_decision.clone(),
                zone,
                None,
                "linux-bwrap-preflight",
            )?;
        }
        Verdict::RequireApproval => {
            let pending = approvals.create_pending(
                &exec_request,
                &final_decision,
                "run command requires approval",
            )?;
            append_receipt(
                receipts,
                runtime,
                exec_request,
                final_decision,
                zone,
                None,
                "approval-pending",
            )?;
            return Err(anyhow!(
                "approval required before running command; pending id: {}",
                pending.id
            ));
        }
        Verdict::Deny => {
            append_receipt(
                receipts,
                runtime,
                exec_request,
                final_decision,
                zone,
                None,
                "denied-preflight",
            )?;
            return Err(anyhow!("command blocked by policy"));
        }
        Verdict::Quarantine => {
            if executable.exists() {
                let _ = quarantine_path(&runtime.config.quarantine_dir, &executable);
            }
            append_receipt(
                receipts,
                runtime,
                exec_request,
                final_decision,
                zone,
                None,
                "quarantine-preflight",
            )?;
            return Err(anyhow!("command quarantined by policy"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        if is_openclaw_gateway_launch_command(cmd) {
            let host_launch = run_unconfined_with_pid_capture(
                cmd,
                runtime,
                "managed-openclaw-host",
                "launch managed OpenClaw gateway command",
                "wait for managed OpenClaw gateway command",
            )?;
            if !host_launch.stdout.is_empty() {
                print!("{}", host_launch.stdout);
            }
            if !host_launch.stderr.is_empty() {
                eprint!("{}", host_launch.stderr);
            }
            append_receipt(
                receipts,
                runtime,
                ActionRequest {
                    id: uuid::Uuid::new_v4().to_string(),
                    timestamp: Utc::now(),
                    kind: ActionKind::Execute,
                    operation: "run_end".to_string(),
                    path: Some(executable),
                    secondary_path: None,
                    host: None,
                    metadata: {
                        let mut m = BTreeMap::new();
                        m.insert("exit_code".to_string(), host_launch.exit_code.to_string());
                        m
                    },
                    process: ProcessContext {
                        pid: std::process::id(),
                        ppid: None,
                        command: cmd.join(" "),
                        process_tree: vec![std::process::id()],
                    },
                },
                Decision {
                    verdict: Verdict::Allow,
                    reason: ReasonCode::AllowedByPolicy,
                    detail: format!(
                        "managed OpenClaw gateway launch exited with code {}",
                        host_launch.exit_code
                    ),
                    approval_ttl_seconds: None,
                },
                zone,
                None,
                "managed-openclaw-host",
            )?;
            return Ok(host_launch);
        }

        let mut wrapped = match confinement::build_bwrap_command(cmd, runtime, engine) {
            Ok(cmd) => cmd,
            Err(err) => {
                append_receipt(
                    receipts,
                    runtime,
                    ActionRequest {
                        id: uuid::Uuid::new_v4().to_string(),
                        timestamp: Utc::now(),
                        kind: ActionKind::Execute,
                        operation: "run_start".to_string(),
                        path: Some(executable.clone()),
                        secondary_path: None,
                        host: None,
                        metadata: {
                            let mut m = BTreeMap::new();
                            m.insert("argv".to_string(), cmd.join(" "));
                            m
                        },
                        process: ProcessContext {
                            pid: std::process::id(),
                            ppid: None,
                            command: cmd.join(" "),
                            process_tree: vec![std::process::id()],
                        },
                    },
                    Decision {
                        verdict: Verdict::Deny,
                        reason: ReasonCode::DenyConfinementToolMissing,
                        detail: err.to_string(),
                        approval_ttl_seconds: None,
                    },
                    zone,
                    None,
                    "linux-bwrap-missing",
                )?;
                return Err(err);
            }
        };
        let child = wrapped
            .spawn()
            .context("launch confined command with bubblewrap")?;
        write_managed_child_pid_if_requested(child.id())?;
        let output = child
            .wait_with_output()
            .context("wait for confined command with bubblewrap")?;

        let stdout_text = String::from_utf8_lossy(&output.stdout).to_string();
        let exit_code = output.status.code().unwrap_or(1);
        let stderr_text = String::from_utf8_lossy(&output.stderr).to_string();

        if !stdout_text.is_empty() {
            print!("{}", stdout_text);
        }
        if !stderr_text.is_empty() {
            eprint!("{}", stderr_text);
        }

        if !output.status.success() && confinement::is_confinement_env_error(&stderr_text) {
            append_receipt(
                receipts,
                runtime,
                ActionRequest {
                    id: uuid::Uuid::new_v4().to_string(),
                    timestamp: Utc::now(),
                    kind: ActionKind::Execute,
                    operation: "run_end".to_string(),
                    path: Some(executable.clone()),
                    secondary_path: None,
                    host: None,
                    metadata: {
                        let mut m = BTreeMap::new();
                        m.insert("exit_code".to_string(), exit_code.to_string());
                        m.insert("stderr".to_string(), stderr_text.clone());
                        m
                    },
                    process: ProcessContext {
                        pid: std::process::id(),
                        ppid: None,
                        command: cmd.join(" "),
                        process_tree: vec![std::process::id()],
                    },
                },
                Decision {
                    verdict: Verdict::Deny,
                    reason: ReasonCode::DenyConfinementToolMissing,
                    detail: format!(
                        "confinement unavailable due host namespace policy: {}",
                        stderr_text.trim()
                    ),
                    approval_ttl_seconds: None,
                },
                zone,
                None,
                "linux-bwrap-unavailable",
            )?;

            let allow_unconfined_fallback = runtime.config.allow_degraded_confinement
                || is_openclaw_gateway_launch_command(cmd);
            if allow_unconfined_fallback {
                let degraded_detail = if runtime.config.allow_degraded_confinement {
                    format!(
                        "confinement degraded: {}; running without bubblewrap",
                        stderr_text.trim()
                    )
                } else {
                    format!(
                        "gateway launch fallback: host blocks bubblewrap ({}); running managed gateway without confinement",
                        stderr_text.trim()
                    )
                };
                append_receipt(
                    receipts,
                    runtime,
                    ActionRequest {
                        id: uuid::Uuid::new_v4().to_string(),
                        timestamp: Utc::now(),
                        kind: ActionKind::Execute,
                        operation: "run_start".to_string(),
                        path: Some(executable.clone()),
                        secondary_path: None,
                        host: None,
                        metadata: {
                            let mut m = BTreeMap::new();
                            m.insert("argv".to_string(), cmd.join(" "));
                            m.insert("degraded".to_string(), "true".to_string());
                            m
                        },
                        process: ProcessContext {
                            pid: std::process::id(),
                            ppid: None,
                            command: cmd.join(" "),
                            process_tree: vec![std::process::id()],
                        },
                    },
                    Decision {
                        verdict: Verdict::Allow,
                        reason: ReasonCode::AllowedByPolicy,
                        detail: degraded_detail,
                        approval_ttl_seconds: None,
                    },
                    zone,
                    None,
                    "degraded-fallback",
                )?;

                let degraded = run_unconfined_with_pid_capture(
                    cmd,
                    runtime,
                    "degraded-no-confinement",
                    "launch degraded unconfined command",
                    "wait for degraded unconfined command",
                )?;
                if !degraded.stdout.is_empty() {
                    print!("{}", degraded.stdout);
                }
                if !degraded.stderr.is_empty() {
                    eprint!("{}", degraded.stderr);
                }
                append_receipt(
                    receipts,
                    runtime,
                    ActionRequest {
                        id: uuid::Uuid::new_v4().to_string(),
                        timestamp: Utc::now(),
                        kind: ActionKind::Execute,
                        operation: "run_end".to_string(),
                        path: Some(executable),
                        secondary_path: None,
                        host: None,
                        metadata: {
                            let mut m = BTreeMap::new();
                            m.insert("exit_code".to_string(), degraded.exit_code.to_string());
                            m
                        },
                        process: ProcessContext {
                            pid: std::process::id(),
                            ppid: None,
                            command: cmd.join(" "),
                            process_tree: vec![std::process::id()],
                        },
                    },
                    Decision {
                        verdict: Verdict::Allow,
                        reason: ReasonCode::AllowedByPolicy,
                        detail: format!(
                            "degraded fallback process exited with code {}",
                            degraded.exit_code
                        ),
                        approval_ttl_seconds: None,
                    },
                    zone,
                    None,
                    "degraded-no-bwrap",
                )?;
                return Ok(degraded);
            }

            return Err(anyhow!(
                "confinement unavailable: {} (set allow_degraded_confinement=true in config to permit degraded fallback)",
                stderr_text.trim()
            ));
        }

        let outcome_request = ActionRequest {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            kind: ActionKind::Execute,
            operation: "run_end".to_string(),
            path: Some(executable),
            secondary_path: None,
            host: None,
            metadata: {
                let mut m = BTreeMap::new();
                m.insert("exit_code".to_string(), exit_code.to_string());
                m
            },
            process: ProcessContext {
                pid: std::process::id(),
                ppid: None,
                command: cmd.join(" "),
                process_tree: vec![std::process::id()],
            },
        };

        append_receipt(
            receipts,
            runtime,
            outcome_request,
            Decision {
                verdict: Verdict::Allow,
                reason: ReasonCode::AllowedByPolicy,
                detail: format!("confined process exited with code {}", exit_code),
                approval_ttl_seconds: None,
            },
            zone,
            None,
            "linux-bwrap",
        )?;
        Ok(RunResult {
            exit_code,
            confinement: "linux-bwrap".to_string(),
            stdout: stdout_text,
            stderr: stderr_text,
        })
    }

    #[cfg(not(target_os = "linux"))]
    {
        append_receipt(
            receipts,
            runtime,
            exec_request,
            Decision {
                verdict: Verdict::Deny,
                reason: ReasonCode::DenyUnsupportedPlatform,
                detail: "linux confinement is required for v0.1 run command".to_string(),
                approval_ttl_seconds: None,
            },
            zone,
            None,
            "unsupported-platform",
        )?;

        Err(anyhow!(
            "run is only supported on Linux for v0.1; current platform unsupported"
        ))
    }
}

fn write_managed_child_pid_if_requested(pid: u32) -> Result<()> {
    let Ok(path) = std::env::var(MANAGED_CHILD_PID_FILE_ENV) else {
        return Ok(());
    };
    let pid_path = PathBuf::from(path.trim());
    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(&pid_path, format!("{pid}\n"))
        .with_context(|| format!("write {}", pid_path.display()))?;
    Ok(())
}

fn run_unconfined_with_pid_capture(
    cmd: &[String],
    runtime: &RuntimeState,
    confinement_label: &str,
    launch_context: &str,
    wait_context: &str,
) -> Result<RunResult> {
    if cmd.is_empty() {
        return Err(anyhow!("empty command"));
    }

    let mut command = Command::new(&cmd[0]);
    for part in cmd.iter().skip(1) {
        command.arg(part);
    }
    command.current_dir(&runtime.config.workspace);
    let child = command
        .spawn()
        .with_context(|| launch_context.to_string())?;
    write_managed_child_pid_if_requested(child.id())?;
    let output = child
        .wait_with_output()
        .with_context(|| wait_context.to_string())?;

    Ok(RunResult {
        exit_code: output.status.code().unwrap_or(1),
        confinement: confinement_label.to_string(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn is_openclaw_gateway_launch_command(cmd: &[String]) -> bool {
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

pub fn append_receipt(
    receipts: &ReceiptStore,
    runtime: &RuntimeState,
    action: ActionRequest,
    decision: Decision,
    zone: Option<crate::model::Zone>,
    diff: Option<crate::model::DiffSummary>,
    confinement: &str,
) -> Result<()> {
    receipts.append(&Receipt {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        action,
        decision,
        zone,
        policy_version: runtime.policy.version.clone(),
        policy_hash: runtime.policy_hash.clone(),
        diff_summary: diff,
        confinement: confinement.to_string(),
    })
}

pub(super) fn quarantine_path(quarantine_dir: &Path, path: &Path) -> Result<PathBuf> {
    fs::create_dir_all(quarantine_dir)
        .with_context(|| format!("create quarantine directory {}", quarantine_dir.display()))?;

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "artifact".to_string());
    let out = quarantine_dir.join(format!("{}-{}", chrono::Utc::now().timestamp(), name));
    fs::rename(path, &out)
        .with_context(|| format!("move {} -> {}", path.display(), out.display()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::is_openclaw_gateway_launch_command;

    #[test]
    fn openclaw_gateway_launch_detects_plain_run() {
        let cmd = vec![
            "openclaw".to_string(),
            "gateway".to_string(),
            "run".to_string(),
        ];
        assert!(is_openclaw_gateway_launch_command(&cmd));
    }

    #[test]
    fn openclaw_gateway_launch_detects_env_prefixed_run() {
        let cmd = vec![
            "env".to_string(),
            "OPENCLAW_HOME=/tmp/openclaw".to_string(),
            "openclaw".to_string(),
            "gateway".to_string(),
            "run".to_string(),
        ];
        assert!(is_openclaw_gateway_launch_command(&cmd));
    }

    #[test]
    fn openclaw_gateway_launch_rejects_stop() {
        let cmd = vec![
            "openclaw".to_string(),
            "gateway".to_string(),
            "stop".to_string(),
        ];
        assert!(!is_openclaw_gateway_launch_command(&cmd));
    }
}
