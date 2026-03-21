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
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;

use crate::approvals::ApprovalStore;
use crate::config::RuntimeState;
use crate::helpers::runners::command_contract::detect_structured_output_kind;
use crate::model::{
    ActionKind, ActionRequest, Decision, ProcessContext, ReasonCode, Receipt, Verdict,
};
use crate::policy::PolicyEngine;
use crate::receipts::ReceiptStore;
use crate::runners::{command_runner_kind, workspace_root_for_command, RunnerKind};

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
    let workspace_root = workspace_root_for_command(runtime, cmd);
    let receipt_command = redacted_command_for_receipts(cmd);
    let mut metadata = BTreeMap::new();
    metadata.insert("argv".to_string(), receipt_command.clone());
    insert_runner_id_metadata(&mut metadata, cmd);

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
            command: receipt_command.clone(),
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
                &workspace_root,
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
                        insert_runner_id_metadata(&mut m, cmd);
                        m
                    },
                    process: ProcessContext {
                        pid: std::process::id(),
                        ppid: None,
                        command: receipt_command.clone(),
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

        if let Err(probe_error_detail) = confinement::probe_linux_runtime_availability() {
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
                        m.insert("exit_code".to_string(), "1".to_string());
                        m.insert("stderr".to_string(), probe_error_detail.clone());
                        insert_runner_id_metadata(&mut m, cmd);
                        m
                    },
                    process: ProcessContext {
                        pid: std::process::id(),
                        ppid: None,
                        command: receipt_command.clone(),
                        process_tree: vec![std::process::id()],
                    },
                },
                Decision {
                    verdict: Verdict::Deny,
                    reason: ReasonCode::DenyConfinementToolMissing,
                    detail: format!(
                        "confinement unavailable due host namespace policy: {}",
                        probe_error_detail
                    ),
                    approval_ttl_seconds: None,
                },
                zone,
                None,
                "linux-bwrap-unavailable",
            )?;

            if can_auto_degraded_fallback(runtime, cmd) {
                return execute_degraded_fallback(
                    cmd,
                    runtime,
                    receipts,
                    zone,
                    &executable,
                    &probe_error_detail,
                );
            }

            return Err(confinement_unavailable_error(
                runtime,
                cmd,
                &probe_error_detail,
            ));
        }

        let mut wrapped =
            match confinement::build_bwrap_command(cmd, runtime, &workspace_root, engine) {
                Ok(wrapped_cmd) => wrapped_cmd,
                Err(err) => {
                    let confinement_error_detail = err.to_string();
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
                                m.insert("argv".to_string(), receipt_command.clone());
                                insert_runner_id_metadata(&mut m, cmd);
                                m
                            },
                            process: ProcessContext {
                                pid: std::process::id(),
                                ppid: None,
                                command: receipt_command.clone(),
                                process_tree: vec![std::process::id()],
                            },
                        },
                        Decision {
                            verdict: Verdict::Deny,
                            reason: ReasonCode::DenyConfinementToolMissing,
                            detail: confinement_error_detail.clone(),
                            approval_ttl_seconds: None,
                        },
                        zone,
                        None,
                        "linux-bwrap-missing",
                    )?;

                    if can_auto_degraded_fallback(runtime, cmd) {
                        return execute_degraded_fallback(
                            cmd,
                            runtime,
                            receipts,
                            zone,
                            &executable,
                            &confinement_error_detail,
                        );
                    }

                    return Err(err);
                }
            };
        let capture_output = should_capture_command_output(cmd);
        let (status, stdout_text, stderr_text) = if capture_output {
            wrapped.stdout(Stdio::piped()).stderr(Stdio::piped());
            let mut child = wrapped
                .spawn()
                .context("launch confined command with bubblewrap")?;
            write_managed_child_pid_if_requested(child.id())?;
            let stdout_pipe = child
                .stdout
                .take()
                .context("capture confined command stdout pipe")?;
            let stderr_pipe = child
                .stderr
                .take()
                .context("capture confined command stderr pipe")?;
            let stdout_reader = std::thread::spawn(move || stream_command_pipe(stdout_pipe, true));
            let stderr_reader = std::thread::spawn(move || stream_command_pipe(stderr_pipe, false));

            let status = child
                .wait()
                .context("wait for confined command with bubblewrap")?;
            let stdout_text = stdout_reader
                .join()
                .map_err(|_| anyhow!("join confined stdout reader"))??;
            let stderr_text = stderr_reader
                .join()
                .map_err(|_| anyhow!("join confined stderr reader"))??;
            (status, stdout_text, stderr_text)
        } else {
            wrapped
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
            let mut child = wrapped
                .spawn()
                .context("launch confined command with bubblewrap")?;
            write_managed_child_pid_if_requested(child.id())?;
            let status = child
                .wait()
                .context("wait for confined command with bubblewrap")?;
            (status, String::new(), String::new())
        };
        let exit_code = status.code().unwrap_or(1);

        if !status.success() && has_confinement_env_error(&stdout_text, &stderr_text) {
            let confinement_error_detail =
                confinement_error_detail_text(&stdout_text, &stderr_text);
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
                        if !stdout_text.is_empty() {
                            m.insert("stdout".to_string(), stdout_text.clone());
                        }
                        insert_runner_id_metadata(&mut m, cmd);
                        m
                    },
                    process: ProcessContext {
                        pid: std::process::id(),
                        ppid: None,
                        command: receipt_command.clone(),
                        process_tree: vec![std::process::id()],
                    },
                },
                Decision {
                    verdict: Verdict::Deny,
                    reason: ReasonCode::DenyConfinementToolMissing,
                    detail: format!(
                        "confinement unavailable due host namespace policy: {}",
                        confinement_error_detail
                    ),
                    approval_ttl_seconds: None,
                },
                zone,
                None,
                "linux-bwrap-unavailable",
            )?;

            if can_auto_degraded_fallback(runtime, cmd) {
                return execute_degraded_fallback(
                    cmd,
                    runtime,
                    receipts,
                    zone,
                    &executable,
                    &confinement_error_detail,
                );
            }

            return Err(confinement_unavailable_error(
                runtime,
                cmd,
                &confinement_error_detail,
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
                insert_runner_id_metadata(&mut m, cmd);
                m
            },
            process: ProcessContext {
                pid: std::process::id(),
                ppid: None,
                command: receipt_command,
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

fn should_capture_command_output(cmd: &[String]) -> bool {
    if detect_structured_output_kind(cmd).is_some() {
        return true;
    }

    let interactive_runner = matches!(
        command_runner_kind(cmd),
        Some(RunnerKind::Claudecode | RunnerKind::Opencode)
    );
    let stdio_is_tty = std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal();
    !(interactive_runner && stdio_is_tty)
}

fn stream_command_pipe<R>(reader: R, to_stdout: bool) -> Result<String>
where
    R: Read,
{
    let mut output = String::new();
    let mut buffered = BufReader::new(reader);
    let mut line = Vec::new();

    loop {
        line.clear();
        let read = buffered.read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        let chunk = String::from_utf8_lossy(&line);
        output.push_str(&chunk);
        if to_stdout {
            print!("{chunk}");
            std::io::stdout().flush().ok();
        } else {
            eprint!("{chunk}");
            std::io::stderr().flush().ok();
        }
    }
    Ok(output)
}

fn run_unconfined_with_pid_capture(
    cmd: &[String],
    workspace_root: &Path,
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
    command
        .current_dir(workspace_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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

fn has_confinement_env_error(stdout: &str, stderr: &str) -> bool {
    confinement::is_confinement_env_error(stderr) || confinement::is_confinement_env_error(stdout)
}

fn confinement_error_detail_text(stdout: &str, stderr: &str) -> String {
    let stderr_trimmed = stderr.trim();
    if !stderr_trimmed.is_empty() && confinement::is_confinement_env_error(stderr_trimmed) {
        return stderr_trimmed.to_string();
    }

    let stdout_trimmed = stdout.trim();
    if !stdout_trimmed.is_empty() && confinement::is_confinement_env_error(stdout_trimmed) {
        return stdout_trimmed.to_string();
    }

    if !stderr_trimmed.is_empty() {
        return stderr_trimmed.to_string();
    }
    if !stdout_trimmed.is_empty() {
        return stdout_trimmed.to_string();
    }
    "unknown confinement error".to_string()
}

fn is_strict_runner_confinement_command(cmd: &[String]) -> bool {
    matches!(
        runner_id_from_command(cmd).as_deref(),
        Some("claudecode") | Some("opencode")
    )
}

fn can_auto_degraded_fallback(runtime: &RuntimeState, cmd: &[String]) -> bool {
    auto_degraded_fallback_allowed(runtime.config.allow_degraded_confinement, cmd)
}

fn auto_degraded_fallback_allowed(allow_degraded_confinement: bool, cmd: &[String]) -> bool {
    if is_openclaw_gateway_launch_command(cmd) {
        return true;
    }
    if is_strict_runner_confinement_command(cmd) {
        return false;
    }
    allow_degraded_confinement
}

fn strict_runner_display_name(cmd: &[String]) -> Option<&'static str> {
    match runner_id_from_command(cmd).as_deref() {
        Some("claudecode") => Some("Claude Code"),
        Some("opencode") => Some("OpenCode"),
        _ => None,
    }
}

fn confinement_unavailable_error(
    runtime: &RuntimeState,
    cmd: &[String],
    detail: &str,
) -> anyhow::Error {
    if let Some(label) = strict_runner_display_name(cmd) {
        return anyhow!(
            "confinement unavailable: {} ({} integration requires bubblewrap confinement; degraded fallback is disabled for this runner)",
            detail,
            label
        );
    }
    if runtime.config.allow_degraded_confinement {
        return anyhow!("confinement unavailable: {}", detail);
    }
    anyhow!(
        "confinement unavailable: {} (set allow_degraded_confinement=true in config to permit degraded fallback)",
        detail
    )
}

fn degraded_fallback_detail(
    runtime: &RuntimeState,
    cmd: &[String],
    confinement_error_detail: &str,
) -> String {
    if runtime.config.allow_degraded_confinement {
        return format!(
            "confinement degraded: {}; running without bubblewrap",
            confinement_error_detail
        );
    }
    if is_openclaw_gateway_launch_command(cmd) {
        return format!(
            "gateway launch fallback: host blocks bubblewrap ({}); running managed gateway without confinement",
            confinement_error_detail
        );
    }
    format!(
        "fallback: host blocks bubblewrap ({}); running managed command without confinement",
        confinement_error_detail
    )
}

fn execute_degraded_fallback(
    cmd: &[String],
    runtime: &RuntimeState,
    receipts: &ReceiptStore,
    zone: Option<crate::model::Zone>,
    executable: &Path,
    confinement_error_detail: &str,
) -> Result<RunResult> {
    let receipt_command = redacted_command_for_receipts(cmd);
    append_receipt(
        receipts,
        runtime,
        ActionRequest {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            kind: ActionKind::Execute,
            operation: "run_start".to_string(),
            path: Some(executable.to_path_buf()),
            secondary_path: None,
            host: None,
            metadata: {
                let mut m = BTreeMap::new();
                m.insert("argv".to_string(), receipt_command.clone());
                m.insert("degraded".to_string(), "true".to_string());
                insert_runner_id_metadata(&mut m, cmd);
                m
            },
            process: ProcessContext {
                pid: std::process::id(),
                ppid: None,
                command: receipt_command.clone(),
                process_tree: vec![std::process::id()],
            },
        },
        Decision {
            verdict: Verdict::Allow,
            reason: ReasonCode::AllowedByPolicy,
            detail: degraded_fallback_detail(runtime, cmd, confinement_error_detail),
            approval_ttl_seconds: None,
        },
        zone,
        None,
        "degraded-fallback",
    )?;

    let degraded = run_unconfined_with_pid_capture(
        cmd,
        &workspace_root_for_command(runtime, cmd),
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
            path: Some(executable.to_path_buf()),
            secondary_path: None,
            host: None,
            metadata: {
                let mut m = BTreeMap::new();
                m.insert("exit_code".to_string(), degraded.exit_code.to_string());
                insert_runner_id_metadata(&mut m, cmd);
                m
            },
            process: ProcessContext {
                pid: std::process::id(),
                ppid: None,
                command: receipt_command,
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
    Ok(degraded)
}

pub fn redacted_command_for_receipts(cmd: &[String]) -> String {
    let tokens = command_tokens_without_env_prefix(cmd);
    let Some(exec) = tokens.first().copied() else {
        return String::new();
    };
    let executable = Path::new(exec)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(exec)
        .to_string();
    let mut summary = vec![executable];

    let mut allow_format_value = false;
    for token in tokens.iter().skip(1).map(|value| value.trim()) {
        if token.is_empty() {
            break;
        }
        if token.starts_with('-') {
            if is_safe_command_summary_token(token) {
                summary.push(token.to_string());
                allow_format_value = token == "--format" || token == "--output-format";
                continue;
            }
            break;
        }
        if allow_format_value && matches!(token, "json" | "stream-json") {
            summary.push(token.to_string());
            allow_format_value = false;
            continue;
        }
        if summary.len() == 1 && is_safe_command_subcommand(token) {
            summary.push(token.to_string());
            allow_format_value = false;
            continue;
        }
        break;
    }

    summary.join(" ")
}

fn is_safe_command_summary_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 64
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | ':' | '='))
}

fn is_safe_command_subcommand(token: &str) -> bool {
    matches!(
        token,
        "run"
            | "web"
            | "gateway"
            | "remote-control"
            | "auth"
            | "doctor"
            | "status"
            | "stop"
            | "start"
            | "mcp"
            | "plugin"
    )
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

pub(crate) fn runner_id_from_command(cmd: &[String]) -> Option<String> {
    let token = command_tokens_without_env_prefix(cmd).first().copied()?;
    let name = Path::new(token)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(token);
    for kind in [
        RunnerKind::Openclaw,
        RunnerKind::Claudecode,
        RunnerKind::Opencode,
    ] {
        if name == kind.executable_name() {
            return Some(kind.id().to_string());
        }
    }
    None
}

pub(crate) fn insert_runner_id_metadata(metadata: &mut BTreeMap<String, String>, cmd: &[String]) {
    if metadata.contains_key("runner_id") {
        return;
    }
    if let Some(id) = runner_id_from_command(cmd) {
        metadata.insert("runner_id".to_string(), id);
    }
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
    use super::{
        auto_degraded_fallback_allowed, confinement_error_detail_text, has_confinement_env_error,
        is_openclaw_gateway_launch_command, is_strict_runner_confinement_command,
        redacted_command_for_receipts, should_capture_command_output,
    };

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

    #[test]
    fn strict_runner_confinement_detects_claude() {
        let cmd = vec!["claude".to_string(), "-p".to_string(), "hello".to_string()];
        assert!(is_strict_runner_confinement_command(&cmd));
    }

    #[test]
    fn strict_runner_confinement_detects_opencode() {
        let cmd = vec![
            "opencode".to_string(),
            "--command".to_string(),
            "hello".to_string(),
        ];
        assert!(is_strict_runner_confinement_command(&cmd));
    }

    #[test]
    fn strict_runner_confinement_rejects_openclaw() {
        let cmd = vec!["openclaw".to_string(), "gateway".to_string()];
        assert!(!is_strict_runner_confinement_command(&cmd));
    }

    #[test]
    fn redacted_command_summary_strips_runner_prompt_arguments() {
        let cmd = vec![
            "env".to_string(),
            "AGENT_RULER_RUNNER_ID=opencode".to_string(),
            "opencode".to_string(),
            "run".to_string(),
            "--format".to_string(),
            "json".to_string(),
            "please summarize this thread".to_string(),
        ];
        assert_eq!(
            redacted_command_for_receipts(&cmd),
            "opencode run --format json"
        );
    }

    #[test]
    fn redacted_command_summary_keeps_basic_cli_shape() {
        let cmd = vec![
            "bash".to_string(),
            "-lc".to_string(),
            "echo hello".to_string(),
        ];
        assert_eq!(redacted_command_for_receipts(&cmd), "bash -lc");
    }

    #[test]
    fn degraded_fallback_stays_disabled_for_runner_commands_even_when_flag_enabled() {
        let claude = vec!["claude".to_string(), "-p".to_string(), "hello".to_string()];
        let opencode = vec![
            "opencode".to_string(),
            "run".to_string(),
            "hello".to_string(),
        ];
        assert!(!auto_degraded_fallback_allowed(true, &claude));
        assert!(!auto_degraded_fallback_allowed(true, &opencode));
    }

    #[test]
    fn degraded_fallback_allows_openclaw_gateway_even_without_global_flag() {
        let cmd = vec![
            "openclaw".to_string(),
            "gateway".to_string(),
            "run".to_string(),
        ];
        assert!(auto_degraded_fallback_allowed(false, &cmd));
    }

    #[test]
    fn degraded_fallback_respects_global_flag_for_non_runner_commands() {
        let cmd = vec!["bash".to_string(), "-lc".to_string(), "echo ok".to_string()];
        assert!(!auto_degraded_fallback_allowed(false, &cmd));
        assert!(auto_degraded_fallback_allowed(true, &cmd));
    }

    #[test]
    fn confinement_env_error_detects_stdout_only_failure() {
        assert!(has_confinement_env_error(
            "bwrap: setting up uid map: Permission denied",
            ""
        ));
    }

    #[test]
    fn confinement_error_detail_prefers_confinement_text() {
        let detail =
            confinement_error_detail_text("bwrap: setting up uid map: Permission denied", "");
        assert!(detail.contains("setting up uid map"));
    }

    #[test]
    fn structured_commands_always_capture_output() {
        let cmd = vec![
            "claude".to_string(),
            "-p".to_string(),
            "--output-format".to_string(),
            "json".to_string(),
            "hello".to_string(),
        ];
        assert!(should_capture_command_output(&cmd));
    }
}
