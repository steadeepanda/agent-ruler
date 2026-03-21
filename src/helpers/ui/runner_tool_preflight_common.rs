//! Runner tool preflight shared helpers.
//!
//! Security boundary:
//! - Runner adapters call this endpoint before executing tool side effects.
//! - Agent Ruler policy/approval logic stays deterministic and centralized.
//! - Every decision appends a receipt; enforcement semantics remain identical
//!   across OpenClaw, Claude Code, and OpenCode.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use axum::extract::{Path as AxPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;
use serde_json::Value as JsonValue;

use crate::approvals::ApprovalStore;
use crate::config::RuntimeState;
use crate::helpers::ui::payloads::OpenClawToolPreflightPayload;
use crate::model::{ActionKind, ActionRequest, Decision, ProcessContext, ReasonCode, Verdict};
use crate::policy::PolicyEngine;
use crate::receipts::ReceiptStore;
use crate::runner::append_receipt;
use crate::sessions::{SessionChannel, SessionRecord, SessionStore};
use crate::ui::{error_response, load_runtime_from_state, WebState};

/// Canonical runner preflight endpoint.
pub async fn api_runner_tool_preflight(
    State(state): State<WebState>,
    AxPath(id): AxPath<String>,
    Json(payload): Json<OpenClawToolPreflightPayload>,
) -> impl IntoResponse {
    let Some(kind) = crate::runners::RunnerKind::from_id(&id) else {
        return error_response(StatusCode::NOT_FOUND, format!("runner `{id}` not found"));
    };
    run_tool_preflight_for_runner(state, payload, kind.id()).await
}

pub async fn run_tool_preflight_for_runner(
    state: WebState,
    payload: OpenClawToolPreflightPayload,
    runner_id: &str,
) -> axum::response::Response {
    let tool_name = normalize_openclaw_tool_name(&payload.tool_name);
    if tool_name.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "tool_name must not be empty".to_string(),
        );
    }

    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };
    let workspace_root =
        match crate::helpers::workspace_root_for_runner_id(&runtime, Some(runner_id)) {
            Ok(path) => path,
            Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
        };
    let runner_session = match register_runner_session(
        &runtime,
        runner_id,
        payload.context.agent_id.as_deref(),
        payload.context.session_key.as_deref(),
    ) {
        Ok(value) => value,
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err),
    };

    let Some(action) = build_openclaw_tool_action(
        &runtime,
        &workspace_root,
        runner_id,
        &tool_name,
        &payload.params,
        payload.context.agent_id.as_deref(),
        payload.context.session_key.as_deref(),
        runner_session.as_ref(),
    ) else {
        // Unknown tool schema or missing target path; caller should continue normally.
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ignored",
                "blocked": false,
                "tool_name": tool_name,
            })),
        )
            .into_response();
    };

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let engine = PolicyEngine::new(runtime.policy.clone(), workspace_root);
    let (mut decision, zone) = engine.evaluate(&action);
    let approval_scope_action = normalize_runner_tool_approval_scope(&action);

    if action
        .metadata
        .get("force_internal_deny")
        .map(|value| value == "true")
        .unwrap_or(false)
    {
        let detail = if action
            .metadata
            .get("operator_only_command")
            .map(|value| value == "agent-ruler")
            .unwrap_or(false)
        {
            "agent-ruler CLI commands are operator-only; use Agent Ruler API tools instead"
                .to_string()
        } else {
            "operator/runtime internals are hidden from agent context".to_string()
        };
        decision = Decision {
            verdict: Verdict::Deny,
            reason: ReasonCode::DenySystemCritical,
            detail,
            approval_ttl_seconds: None,
        };
    }
    if action
        .metadata
        .get("force_delivery_deny")
        .map(|value| value == "true")
        .unwrap_or(false)
    {
        decision = Decision {
            verdict: Verdict::Deny,
            reason: ReasonCode::DenyUserDataWrite,
            detail: "direct writes to user destination are blocked; use stage + deliver flow"
                .to_string(),
            approval_ttl_seconds: None,
        };
    }

    // Agent read paths should never get blocked solely by Zone-2 write approvals.
    // We still preserve explicit deny outcomes (for example, system/secrets boundaries).
    if is_read_like_openclaw_tool(&tool_name) && decision.verdict == Verdict::RequireApproval {
        decision = Decision {
            verdict: Verdict::Allow,
            reason: ReasonCode::AllowedByPolicy,
            detail: "read-style tool bypassed write-only approval boundary".to_string(),
            approval_ttl_seconds: None,
        };
    }

    // Reuse existing active approvals to avoid creating duplicate pending rows when
    // an agent retries the same operation while an operator already approved scope.
    if decision.verdict == Verdict::RequireApproval {
        match approvals.has_active_approval_for(&approval_scope_action) {
            Ok(true) => {
                decision = Decision {
                    verdict: Verdict::Allow,
                    reason: ReasonCode::AllowedByPolicy,
                    detail: "allowed by active approval scope".to_string(),
                    approval_ttl_seconds: None,
                };
            }
            Ok(false) => {}
            Err(err) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("evaluate active approval scope: {err}"),
                );
            }
        }
    }

    if let Err(err) = append_receipt(
        &receipts,
        &runtime,
        action.clone(),
        decision.clone(),
        zone,
        None,
        "openclaw-tool-preflight",
    ) {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("append receipt for runner tool preflight: {err}"),
        );
    }

    let reason = decision_reason_label(decision.reason);
    let verdict = decision_verdict_label(decision.verdict);

    match decision.verdict {
        Verdict::Allow => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "allow",
                "blocked": false,
                "tool_name": tool_name,
                "verdict": verdict,
                "reason": reason,
                "detail": decision.detail,
            })),
        )
            .into_response(),
        Verdict::RequireApproval => {
            let pending = match approvals.create_pending(
                &approval_scope_action,
                &decision,
                "runner tool action requires approval",
            ) {
                Ok(value) => value,
                Err(err) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("create pending approval for runner tool preflight: {err}"),
                    );
                }
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "pending_approval",
                    "blocked": true,
                    "tool_name": tool_name,
                    "verdict": verdict,
                    "reason": reason,
                    "detail": decision.detail,
                    "approval_id": pending.id,
                })),
            )
                .into_response()
        }
        Verdict::Deny => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "denied",
                "blocked": true,
                "tool_name": tool_name,
                "verdict": verdict,
                "reason": reason,
                "detail": decision.detail,
            })),
        )
            .into_response(),
        Verdict::Quarantine => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "quarantined",
                "blocked": true,
                "tool_name": tool_name,
                "verdict": verdict,
                "reason": reason,
                "detail": decision.detail,
            })),
        )
            .into_response(),
    }
}

fn normalize_openclaw_tool_name(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut previous_was_separator = false;
    for ch in value.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch);
            previous_was_separator = false;
            continue;
        }
        if !previous_was_separator {
            normalized.push('_');
        }
        previous_was_separator = true;
    }
    let normalized = normalized.trim_matches('_');

    if normalized.is_empty() {
        return String::new();
    }
    if matches!(
        normalized,
        "write"
            | "edit"
            | "append"
            | "apply_patch"
            | "delete"
            | "remove"
            | "rm"
            | "move"
            | "rename"
            | "read"
            | "cat"
            | "exec"
            | "bash"
            | "run_command"
    ) {
        return normalized.to_string();
    }
    if normalized.contains("apply_patch")
        || (normalized.contains("patch")
            && (normalized.contains("apply") || normalized.contains("edit")))
    {
        return "apply_patch".to_string();
    }
    if normalized.contains("delete")
        || normalized.contains("remove")
        || normalized == "rm"
        || normalized.ends_with("_rm")
    {
        return "delete".to_string();
    }
    if normalized.contains("rename") || normalized.contains("move") {
        return "move".to_string();
    }
    if normalized.contains("read") || normalized.contains("cat") {
        return "read".to_string();
    }
    if normalized.contains("append") {
        return "append".to_string();
    }
    if normalized.contains("edit") {
        return "edit".to_string();
    }
    if normalized.contains("write") {
        return "write".to_string();
    }
    if normalized.contains("run_command")
        || normalized.contains("exec")
        || normalized.contains("bash")
        || normalized.contains("shell")
        || normalized.contains("terminal")
        || normalized.ends_with("_command")
        || normalized == "command"
    {
        return "exec".to_string();
    }

    normalized.to_string()
}

fn is_read_like_openclaw_tool(tool_name: &str) -> bool {
    matches!(tool_name, "read" | "cat")
}

fn build_openclaw_tool_action(
    runtime: &RuntimeState,
    workspace_root: &Path,
    runner_id: &str,
    tool_name: &str,
    params: &JsonValue,
    agent_id: Option<&str>,
    session_key: Option<&str>,
    session_record: Option<&SessionRecord>,
) -> Option<ActionRequest> {
    let mut metadata = BTreeMap::new();
    metadata.insert("tool_name".to_string(), tool_name.to_string());
    metadata.insert("runner_id".to_string(), runner_id.to_string());
    if let Some(agent_id) = agent_id {
        let trimmed = agent_id.trim();
        if !trimmed.is_empty() {
            metadata.insert("agent_id".to_string(), trimmed.to_string());
        }
    }
    if let Some(session_key) = session_key {
        let trimmed = session_key.trim();
        if !trimmed.is_empty() {
            metadata.insert("session_key".to_string(), trimmed.to_string());
        }
    }
    if let Some(session_record) = session_record {
        metadata.insert("ruler_session_id".to_string(), session_record.id.clone());
        metadata.insert(
            "ruler_session_label".to_string(),
            session_record.display_label(),
        );
    }
    if !params.is_null() {
        if let Ok(raw) = serde_json::to_string(params) {
            metadata.insert("tool_params".to_string(), truncate_for_metadata(&raw, 4096));
        }
    }

    let mut action = ActionRequest {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        kind: ActionKind::FileWrite,
        operation: format!("{runner_id}_tool_{tool_name}"),
        path: None,
        secondary_path: None,
        host: None,
        metadata,
        process: ProcessContext {
            pid: std::process::id(),
            ppid: None,
            command: format!("{runner_id}_tool {tool_name}"),
            process_tree: vec![std::process::id()],
        },
    };

    match tool_name {
        "write" | "edit" | "append" | "apply_patch" => {
            let path = extract_first_param_string(
                params,
                &["path", "file_path", "filePath", "target", "to"],
            )?;
            action.kind = ActionKind::FileWrite;
            action.path = Some(normalize_runner_tool_path(workspace_root, &path));
        }
        "delete" | "remove" | "rm" => {
            let targets = extract_path_values(
                params,
                &[
                    "paths",
                    "targets",
                    "path",
                    "file_path",
                    "filePath",
                    "target",
                ],
            );
            let first = targets.first()?;
            action.kind = ActionKind::FileDelete;
            action.path = Some(normalize_runner_tool_path(workspace_root, first));
            action
                .metadata
                .insert("delete_count".to_string(), targets.len().to_string());
        }
        "move" | "rename" => {
            let source = extract_first_param_string(
                params,
                &[
                    "from",
                    "src",
                    "source",
                    "old_path",
                    "oldPath",
                    "path",
                    "file_path",
                    "filePath",
                ],
            )?;
            let destination = extract_first_param_string(
                params,
                &["to", "dst", "destination", "new_path", "newPath", "target"],
            )?;
            action.kind = ActionKind::FileRename;
            action.path = Some(normalize_runner_tool_path(workspace_root, &destination));
            action.secondary_path = Some(normalize_runner_tool_path(workspace_root, &source));
        }
        "read" | "cat" => {
            let path = extract_first_param_string(
                params,
                &["path", "file_path", "filePath", "target", "from"],
            )?;
            action.kind = ActionKind::FileWrite;
            action.path = Some(normalize_runner_tool_path(workspace_root, &path));
            action
                .metadata
                .insert("non_mutating_hint".to_string(), "read".to_string());
        }
        "exec" | "bash" | "run_command" => {
            let command_text = extract_command_text(params)?;
            let executable = command_text.split_whitespace().next().unwrap_or_default();
            if executable.is_empty() {
                return None;
            }
            let resolved = crate::utils::resolve_command_path(executable)
                .unwrap_or_else(|| PathBuf::from(executable));

            if contains_agent_ruler_cli_invocation(&command_text, &resolved) {
                action
                    .metadata
                    .insert("force_internal_deny".to_string(), "true".to_string());
                action.metadata.insert(
                    "operator_only_command".to_string(),
                    "agent-ruler".to_string(),
                );
            }

            // Check for destructive file commands that should be subject to filesystem
            // zone policies even when executed via shell. This prevents bypassing
            // write/delete restrictions by using `rm` through exec.
            let copy_move_info = extract_copy_move_command_target(&command_text, &resolved);
            let destructive_info = extract_destructive_command_target(&command_text, &resolved);
            if let Some(info) = copy_move_info {
                // Copy/move commands mutate destination paths and must be governed
                // as filesystem actions, not generic execute calls.
                action.kind = if info.is_move {
                    ActionKind::FileRename
                } else {
                    ActionKind::FileWrite
                };
                action.path = Some(normalize_runner_tool_path(workspace_root, &info.dst_path));
                action.secondary_path =
                    Some(normalize_runner_tool_path(workspace_root, &info.src_path));
                action.metadata.insert(
                    "underlying_exec".to_string(),
                    resolved.to_string_lossy().to_string(),
                );
                action.metadata.insert(
                    "argv".to_string(),
                    truncate_for_metadata(&command_text, 4096),
                );
                action.process.command = format!("{runner_id}_tool {tool_name} {command_text}");
            } else if let Some(info) = destructive_info {
                // Treat as filesystem operation (FileDelete or FileWrite) instead of
                // plain Execute so zone policies apply to the target path.
                action.kind = if info.is_delete {
                    ActionKind::FileDelete
                } else {
                    ActionKind::FileWrite
                };
                action.path = Some(normalize_runner_tool_path(
                    workspace_root,
                    &info.target_path,
                ));
                action.metadata.insert(
                    "underlying_exec".to_string(),
                    resolved.to_string_lossy().to_string(),
                );
                if let Some(delete_count) = info.delete_count {
                    action
                        .metadata
                        .insert("delete_count".to_string(), delete_count.to_string());
                }
                if info.has_wildcard {
                    action
                        .metadata
                        .insert("delete_wildcard".to_string(), "true".to_string());
                }
                action.metadata.insert(
                    "argv".to_string(),
                    truncate_for_metadata(&command_text, 4096),
                );
                action.process.command = format!("{runner_id}_tool {tool_name} {command_text}");
            } else if let Some(download_exec) = detect_download_exec_chain(&command_text) {
                // Detect a download->exec chain and evaluate against execution policy
                // using the downloaded artifact path rather than the transfer tool path.
                action.kind = ActionKind::Execute;
                action.path = Some(normalize_runner_tool_path(
                    workspace_root,
                    &download_exec.exec_path,
                ));
                action
                    .metadata
                    .insert("downloaded".to_string(), "true".to_string());
                action.metadata.insert(
                    "download_source".to_string(),
                    download_exec.download_url.clone(),
                );
                if let Some(host) = host_from_url(&download_exec.download_url) {
                    action.host = Some(host);
                }
                action.metadata.insert(
                    "argv".to_string(),
                    truncate_for_metadata(&command_text, 4096),
                );
                action.process.command = format!("{runner_id}_tool {tool_name} {command_text}");
            } else if let Some(redirection_target) = extract_shell_redirection_target(&command_text)
            {
                // Shell redirection (`>`, `>>`, `1>`, `2>`) mutates files and must
                // respect filesystem zones instead of being treated as plain execution.
                action.kind = ActionKind::FileWrite;
                action.path = Some(normalize_runner_tool_path(
                    workspace_root,
                    &redirection_target,
                ));
                action.metadata.insert(
                    "underlying_exec".to_string(),
                    resolved.to_string_lossy().to_string(),
                );
                action
                    .metadata
                    .insert("redirection_write".to_string(), "true".to_string());
                action.metadata.insert(
                    "argv".to_string(),
                    truncate_for_metadata(&command_text, 4096),
                );
                action.process.command = format!("{runner_id}_tool {tool_name} {command_text}");
            } else {
                action.kind = ActionKind::Execute;
                action.path = Some(resolved);
                action.metadata.insert(
                    "argv".to_string(),
                    truncate_for_metadata(&command_text, 4096),
                );
                action.process.command = format!("{runner_id}_tool {tool_name} {command_text}");
            }

            if looks_like_interpreter_stream_exec(&command_text, action.path.as_deref()) {
                action
                    .metadata
                    .insert("stream_exec".to_string(), "true".to_string());
            }
        }
        _ => return None,
    }

    let path_is_internal = action
        .path
        .as_ref()
        .map(|path| is_operator_internal_path(path, runtime))
        .unwrap_or(false);
    let path_is_workspace_scoped = action
        .path
        .as_ref()
        .map(|path| is_workspace_scoped_path(path, workspace_root))
        .unwrap_or(false);
    let secondary_is_internal = action
        .secondary_path
        .as_ref()
        .map(|path| is_operator_internal_path(path, runtime))
        .unwrap_or(false);
    let secondary_is_workspace_scoped = action
        .secondary_path
        .as_ref()
        .map(|path| is_workspace_scoped_path(path, workspace_root))
        .unwrap_or(false);
    if (path_is_internal && !path_is_workspace_scoped)
        || (secondary_is_internal && !secondary_is_workspace_scoped)
    {
        action.kind = ActionKind::FileWrite;
        action
            .metadata
            .insert("force_internal_deny".to_string(), "true".to_string());
    }
    if matches!(
        action.kind,
        ActionKind::FileWrite | ActionKind::FileDelete | ActionKind::FileRename
    ) && action
        .path
        .as_ref()
        .map(|path| is_delivery_destination_path(path, runtime))
        .unwrap_or(false)
    {
        action
            .metadata
            .insert("force_delivery_deny".to_string(), "true".to_string());
    }

    Some(action)
}

fn register_runner_session(
    runtime: &RuntimeState,
    runner_id: &str,
    agent_id: Option<&str>,
    session_key: Option<&str>,
) -> Result<Option<SessionRecord>, String> {
    let Some(runner_kind) = crate::runners::RunnerKind::from_id(runner_id) else {
        return Ok(None);
    };
    let Some(session_key) = session_key.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let label = agent_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("Agent {value}"));
    let store = SessionStore::new(SessionStore::default_path(&runtime.config.state_dir));
    store
        .touch_runner_session(
            runner_kind,
            session_key,
            SessionChannel::Tui,
            label.as_deref(),
            None,
        )
        .map(Some)
        .map_err(|err| format!("register runner session: {err}"))
}

fn normalize_runner_tool_approval_scope(action: &ActionRequest) -> ActionRequest {
    let mut scoped = action.clone();
    // Session/tool payload metadata changes across retries, but approval scope
    // should stay stable for the same governed operation/path/host tuple.
    scoped
        .metadata
        .retain(|key, _| !matches!(key.as_str(), "agent_id" | "session_key" | "tool_params"));
    scoped
}

fn extract_first_param_string(params: &JsonValue, keys: &[&str]) -> Option<String> {
    let record = params.as_object()?;
    for key in keys {
        let Some(value) = record.get(*key) else {
            continue;
        };
        match value {
            JsonValue::String(raw) => {
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
            JsonValue::Array(items) => {
                for item in items {
                    let Some(raw) = item.as_str() else {
                        continue;
                    };
                    let trimmed = raw.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn extract_path_values(params: &JsonValue, keys: &[&str]) -> Vec<String> {
    let mut paths = BTreeSet::new();
    let Some(record) = params.as_object() else {
        return Vec::new();
    };

    for key in keys {
        let Some(value) = record.get(*key) else {
            continue;
        };
        match value {
            JsonValue::String(raw) => {
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    paths.insert(trimmed.to_string());
                }
            }
            JsonValue::Array(items) => {
                for item in items {
                    let Some(raw) = item.as_str() else {
                        continue;
                    };
                    let trimmed = raw.trim();
                    if !trimmed.is_empty() {
                        paths.insert(trimmed.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    paths.into_iter().collect()
}

fn extract_command_text(params: &JsonValue) -> Option<String> {
    let record = params.as_object()?;
    for key in ["command", "cmd", "script", "argv"] {
        let Some(value) = record.get(key) else {
            continue;
        };
        match value {
            JsonValue::String(raw) => {
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
            JsonValue::Array(items) => {
                let parts = items
                    .iter()
                    .filter_map(JsonValue::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>();
                if !parts.is_empty() {
                    return Some(parts.join(" "));
                }
            }
            _ => {}
        }
    }
    None
}

fn contains_agent_ruler_cli_invocation(command_text: &str, resolved_exec: &Path) -> bool {
    if resolved_exec
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("agent-ruler"))
        .unwrap_or(false)
    {
        return true;
    }

    command_text
        .split_whitespace()
        .map(clean_command_token)
        .filter(|token| !token.is_empty())
        .any(|token| {
            Path::new(&token)
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case("agent-ruler"))
                .unwrap_or(false)
        })
}

/// Detect destructive file commands (rm, rmdir, shred, etc.) and extract their target paths.
/// Returns (target_path, is_delete) where is_delete indicates if it's a delete vs write operation.
/// This prevents bypassing filesystem zone policies by using shell commands.
struct DestructiveTarget {
    target_path: String,
    is_delete: bool,
    delete_count: Option<usize>,
    has_wildcard: bool,
}

struct CopyMoveTarget {
    src_path: String,
    dst_path: String,
    is_move: bool,
}

struct DownloadExecChain {
    download_url: String,
    exec_path: String,
}

fn extract_copy_move_command_target(
    command_text: &str,
    resolved_exec: &Path,
) -> Option<CopyMoveTarget> {
    let exec_name = resolved_exec
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let direct_mode = if exec_name.eq_ignore_ascii_case("mv") {
        Some(true)
    } else if exec_name.eq_ignore_ascii_case("cp") || exec_name.eq_ignore_ascii_case("install") {
        Some(false)
    } else {
        None
    };

    let parts: Vec<&str> = command_text.split_whitespace().collect();
    if let Some(is_move) = direct_mode {
        let targets = collect_copy_move_targets(&parts, 1)?;
        return Some(CopyMoveTarget {
            src_path: targets.0,
            dst_path: targets.1,
            is_move,
        });
    }

    // Fallback: detect nested cp/mv snippets in shell commands.
    for idx in 0..parts.len() {
        let token = clean_command_token(parts[idx]);
        if token.is_empty() {
            continue;
        }
        let name = Path::new(&token)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let nested_mode = if name.eq_ignore_ascii_case("mv") {
            Some(true)
        } else if name.eq_ignore_ascii_case("cp") || name.eq_ignore_ascii_case("install") {
            Some(false)
        } else {
            None
        };
        let Some(is_move) = nested_mode else {
            continue;
        };
        let targets = collect_copy_move_targets(&parts, idx + 1)?;
        return Some(CopyMoveTarget {
            src_path: targets.0,
            dst_path: targets.1,
            is_move,
        });
    }

    None
}

fn collect_copy_move_targets(parts: &[&str], start_idx: usize) -> Option<(String, String)> {
    let mut targets: Vec<String> = Vec::new();
    for raw in parts.iter().skip(start_idx) {
        let token = clean_command_token(raw);
        if token.is_empty() {
            continue;
        }
        if is_shell_separator(&token) {
            break;
        }
        if token == "--" || token.starts_with('-') {
            continue;
        }
        targets.push(token);
    }
    if targets.len() < 2 {
        return None;
    }
    let src = targets.first()?.clone();
    let dst = targets.last()?.clone();
    Some((src, dst))
}

fn extract_destructive_command_target(
    command_text: &str,
    resolved_exec: &Path,
) -> Option<DestructiveTarget> {
    let exec_name = resolved_exec
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();

    // Commands that delete files/directories
    let delete_commands = ["rm", "rmdir", "shred", "unlink"];
    // Commands that modify files (truncate, overwrite)
    let write_commands = ["truncate", "dd", "tee"];

    let direct_mode = if delete_commands.contains(&exec_name) {
        Some(true)
    } else if write_commands.contains(&exec_name) {
        Some(false)
    } else {
        None
    };

    let parts: Vec<&str> = command_text.split_whitespace().collect();

    if let Some(is_delete) = direct_mode {
        let mut targets: Vec<String> = Vec::new();
        let mut after_command = false;
        for (idx, raw) in parts.iter().enumerate() {
            if !after_command {
                if idx == 0 {
                    after_command = true;
                    continue;
                }
                continue;
            }

            let token = clean_command_token(raw);
            if token.is_empty() {
                continue;
            }
            if is_shell_separator(&token) {
                break;
            }
            if token == "--" || token.starts_with('-') {
                continue;
            }
            targets.push(token);
        }

        if let Some(first) = targets.first() {
            let has_wildcard = targets.iter().any(|value| contains_glob_wildcard(value));
            return Some(DestructiveTarget {
                target_path: first.clone(),
                is_delete,
                delete_count: if is_delete { Some(targets.len()) } else { None },
                has_wildcard,
            });
        }
    }

    // Fallback: detect nested destructive commands embedded in shell snippets.
    for idx in 0..parts.len() {
        let token = clean_command_token(parts[idx]);
        if token.is_empty() {
            continue;
        }
        let name = Path::new(&token)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let nested_is_delete = if delete_commands.contains(&name) {
            true
        } else if write_commands.contains(&name) {
            false
        } else {
            continue;
        };

        let mut targets: Vec<String> = Vec::new();
        for raw in parts.iter().skip(idx + 1) {
            let candidate = clean_command_token(raw);
            if candidate.is_empty() {
                continue;
            }
            if is_shell_separator(&candidate) {
                break;
            }
            if candidate == "--" || candidate.starts_with('-') {
                continue;
            }
            targets.push(candidate);
        }
        if let Some(first) = targets.first() {
            let has_wildcard = targets.iter().any(|value| contains_glob_wildcard(value));
            return Some(DestructiveTarget {
                target_path: first.clone(),
                is_delete: nested_is_delete,
                delete_count: if nested_is_delete {
                    Some(targets.len())
                } else {
                    None
                },
                has_wildcard,
            });
        }
    }

    None
}

fn extract_shell_redirection_target(command_text: &str) -> Option<String> {
    let parts: Vec<&str> = command_text.split_whitespace().collect();
    let operators = [">", ">>", "1>", "1>>", "2>", "2>>"];

    for (idx, raw) in parts.iter().enumerate() {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }

        if operators.contains(&token) {
            if let Some(next) = parts.get(idx + 1) {
                let candidate = clean_command_token(next);
                if is_valid_redirection_target(&candidate) {
                    return Some(candidate);
                }
            }
            continue;
        }

        for operator in operators {
            if let Some(rest) = token.strip_prefix(operator) {
                let candidate = clean_command_token(rest);
                if is_valid_redirection_target(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }

    None
}

fn is_valid_redirection_target(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    if token.starts_with('&') {
        // Descriptor merge (`2>&1`) is not a file write target.
        return false;
    }
    !is_shell_separator(token)
}

fn detect_download_exec_chain(command_text: &str) -> Option<DownloadExecChain> {
    let lowered = command_text.to_ascii_lowercase();
    let downloader = if lowered.contains("curl ") || lowered.starts_with("curl ") {
        "curl"
    } else if lowered.contains("wget ") || lowered.starts_with("wget ") {
        "wget"
    } else {
        return None;
    };

    let parts: Vec<&str> = command_text.split_whitespace().collect();
    let mut download_url: Option<String> = None;
    let mut output_path: Option<String> = None;
    for idx in 0..parts.len() {
        let token = clean_command_token(parts[idx]);
        if token.starts_with("http://") || token.starts_with("https://") {
            download_url = Some(token.clone());
        }
        if token == "-o" || token == "--output" {
            if let Some(next) = parts.get(idx + 1) {
                let value = clean_command_token(next);
                if !value.is_empty() {
                    output_path = Some(value);
                }
            }
        }
    }

    let url = download_url?;
    let output = output_path?;
    let output_quoted = output.replace('"', "\\\"");
    let exec_markers = [
        format!(" {}", output),
        format!(" ./{}", output),
        format!(" {output_quoted}"),
        format!(";{}", output),
    ];
    let executes_output = exec_markers
        .iter()
        .any(|marker| command_text.contains(marker))
        || lowered.contains("chmod +x");

    if executes_output {
        return Some(DownloadExecChain {
            download_url: url,
            exec_path: output,
        });
    }

    // Streamed variants are handled by stream_exec detection.
    let _ = downloader;
    None
}

fn looks_like_interpreter_stream_exec(command_text: &str, resolved_path: Option<&Path>) -> bool {
    let lowered = command_text.to_ascii_lowercase();
    let is_interpreter = resolved_path
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .map(|name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "bash" | "sh" | "zsh" | "dash" | "python" | "python3" | "node" | "perl" | "ruby"
            )
        })
        .unwrap_or(false);

    if !is_interpreter {
        return false;
    }

    lowered.contains("<(")
        || lowered.contains("| bash")
        || lowered.contains("| sh")
        || lowered.contains("| python")
        || lowered.contains("$(curl")
        || lowered.contains("$(wget")
}

fn clean_command_token(token: &str) -> String {
    token
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\'' | ';' | '|' | '&' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | '>' | '<'
            )
        })
        .to_string()
}

fn is_shell_separator(token: &str) -> bool {
    matches!(token, ";" | "&&" | "||" | "|")
}

fn contains_glob_wildcard(token: &str) -> bool {
    token.contains('*') || token.contains('?') || token.contains('[')
}

fn host_from_url(url: &str) -> Option<String> {
    let no_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host_port = no_scheme.split('/').next().unwrap_or_default().trim();
    if host_port.is_empty() {
        return None;
    }
    let host = host_port.split(':').next().unwrap_or_default().trim();
    if host.is_empty() {
        return None;
    }
    Some(host.to_string())
}

fn is_operator_internal_path(path: &Path, runtime: &RuntimeState) -> bool {
    if is_operator_internal_path_shallow(path, runtime) {
        return true;
    }

    if let Ok(canonical) = path.canonicalize() {
        if canonical != path && is_operator_internal_path_shallow(&canonical, runtime) {
            return true;
        }
    }

    false
}

fn is_delivery_destination_path(path: &Path, runtime: &RuntimeState) -> bool {
    if is_delivery_destination_path_shallow(path, runtime) {
        return true;
    }

    if let Ok(canonical) = path.canonicalize() {
        if canonical != path && is_delivery_destination_path_shallow(&canonical, runtime) {
            return true;
        }
    }

    false
}

fn is_delivery_destination_path_shallow(path: &Path, runtime: &RuntimeState) -> bool {
    let delivery_root = runtime
        .config
        .default_delivery_dir
        .canonicalize()
        .unwrap_or_else(|_| runtime.config.default_delivery_dir.clone());

    path == delivery_root || path.starts_with(&delivery_root)
}

fn is_operator_internal_path_shallow(path: &Path, runtime: &RuntimeState) -> bool {
    let normalized = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();

    // Current runtime state internals are never agent-visible.
    if path.starts_with(&runtime.config.state_dir) {
        return true;
    }

    // Protect Agent Ruler source tree in dev builds (compile-time source root).
    let source_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if path.starts_with(&source_root) {
        return true;
    }

    // Protect known Agent Ruler runtime/install internals across installs.
    if normalized.contains("/agent-ruler/projects/")
        && (normalized.contains("/state/") || normalized.ends_with("/state"))
    {
        return true;
    }
    if normalized.contains("/agent-ruler/installs/") {
        return true;
    }

    false
}

fn normalize_runner_tool_path(workspace_root: &Path, raw: &str) -> PathBuf {
    let expanded_home = expand_home_path(raw);
    let candidate = expanded_home.unwrap_or_else(|| PathBuf::from(raw));
    if candidate.is_absolute() {
        candidate
    } else {
        workspace_root.join(candidate)
    }
}

fn is_workspace_scoped_path(path: &Path, workspace_root: &Path) -> bool {
    if path.starts_with(workspace_root) {
        return true;
    }

    let canonical_path = path.canonicalize();
    let canonical_workspace = workspace_root.canonicalize();
    if let (Ok(path), Ok(workspace)) = (canonical_path, canonical_workspace) {
        return path.starts_with(workspace);
    }

    false
}

fn expand_home_path(raw: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let trimmed = raw.trim();
    if trimmed == "~" {
        return Some(home);
    }
    if let Some(suffix) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        return Some(home.join(suffix));
    }
    None
}

fn truncate_for_metadata(raw: &str, max_len: usize) -> String {
    if raw.len() <= max_len {
        return raw.to_string();
    }
    let mut out = raw.chars().take(max_len).collect::<String>();
    out.push_str("...");
    out
}

fn decision_verdict_label(verdict: Verdict) -> String {
    serde_json::to_value(verdict)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "unknown".to_string())
}

fn decision_reason_label(reason: ReasonCode) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "unknown".to_string())
}
