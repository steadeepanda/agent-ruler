use std::fs;
use std::net::SocketAddr;
use std::path::{Component, Path};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use walkdir::WalkDir;

use crate::approvals::ApprovalStore;
use crate::config::{save_config, RuntimeState};
use crate::helpers::runtime::resolve_ui_path_update;
use crate::helpers::ui::payloads::{
    FileListItem, FileListQuery, ReceiptPage, ReceiptQuery, RuntimePathsPayload, RuntimePayload,
    StatusPayload, UiLogEventPayload, UiLogPage, UiLogQuery,
};
use crate::openclaw_bridge::{
    ensure_generated_config, generated_config_path, update_generated_config,
    OpenClawBridgeConfigPatch,
};
use crate::receipts::ReceiptStore;
use crate::staged_exports::{StagedExportState, StagedExportStore};
use crate::ui::{error_response, load_runtime_from_state, WebState};
use crate::ui_logs::UiLogStore;

const UI_EVENT_LOG_FILE_NAME: &str = "control-panel-events.jsonl";

pub async fn api_status(State(state): State<WebState>) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let staged = StagedExportStore::new(&runtime.config.staged_exports_file);

    let pending = approvals.list_pending().unwrap_or_default();
    let all_receipts = receipts.read_all().unwrap_or_default();
    let staged_records = staged.list().unwrap_or_default();
    let staged_count = staged_records
        .iter()
        .filter(|r| r.state == StagedExportState::Staged)
        .count();
    let delivered_count = staged_records
        .iter()
        .filter(|r| r.state == StagedExportState::Delivered)
        .count();

    let payload = StatusPayload {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        profile: runtime.policy.profile,
        policy_version: runtime.policy.version,
        policy_hash: runtime.policy_hash,
        pending_approvals: pending.len(),
        receipts_count: all_receipts.len(),
        staged_count,
        delivered_count,
        runtime_root: runtime.config.runtime_root.to_string_lossy().to_string(),
        workspace: runtime.config.workspace.to_string_lossy().to_string(),
        shared_zone: runtime.config.shared_zone_dir.to_string_lossy().to_string(),
        state_dir: runtime.config.state_dir.to_string_lossy().to_string(),
        default_delivery_dir: runtime
            .config
            .default_delivery_dir
            .to_string_lossy()
            .to_string(),
        default_user_destination_dir: runtime
            .config
            .default_delivery_dir
            .to_string_lossy()
            .to_string(),
        ui_bind: runtime.config.ui_bind.clone(),
        allow_degraded_confinement: runtime.config.allow_degraded_confinement,
        ui_show_debug_tools: runtime.config.ui_show_debug_tools,
        approval_wait_timeout_secs: runtime.config.approval_wait_timeout_secs,
    };
    (StatusCode::OK, Json(payload)).into_response()
}

pub async fn api_runtime(State(state): State<WebState>) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let payload = RuntimePayload {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        ruler_root: runtime.config.ruler_root.to_string_lossy().to_string(),
        runtime_root: runtime.config.runtime_root.to_string_lossy().to_string(),
        workspace: runtime.config.workspace.to_string_lossy().to_string(),
        shared_zone: runtime.config.shared_zone_dir.to_string_lossy().to_string(),
        state_dir: runtime.config.state_dir.to_string_lossy().to_string(),
        policy_file: runtime.config.policy_file.to_string_lossy().to_string(),
        receipts_file: runtime.config.receipts_file.to_string_lossy().to_string(),
        approvals_file: runtime.config.approvals_file.to_string_lossy().to_string(),
        staged_exports_file: runtime
            .config
            .staged_exports_file
            .to_string_lossy()
            .to_string(),
        default_delivery_dir: runtime
            .config
            .default_delivery_dir
            .to_string_lossy()
            .to_string(),
        default_user_destination_dir: runtime
            .config
            .default_delivery_dir
            .to_string_lossy()
            .to_string(),
        ui_bind: runtime.config.ui_bind.clone(),
        exec_layer_dir: runtime.config.exec_layer_dir.to_string_lossy().to_string(),
        quarantine_dir: runtime.config.quarantine_dir.to_string_lossy().to_string(),
        ui_show_debug_tools: runtime.config.ui_show_debug_tools,
        approval_wait_timeout_secs: runtime.config.approval_wait_timeout_secs,
    };

    (StatusCode::OK, Json(payload)).into_response()
}

pub async fn api_runtime_paths(
    State(state): State<WebState>,
    Json(payload): Json<RuntimePathsPayload>,
) -> impl IntoResponse {
    let mut runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    if let Err(err) = apply_runtime_config_patch(&mut runtime, &payload) {
        return error_response(StatusCode::BAD_REQUEST, err);
    }

    if let Err(err) = save_config(
        &runtime.config.state_dir.join("config.yaml"),
        &runtime.config,
    ) {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "updated",
            "shared_zone": runtime.config.shared_zone_dir,
            "default_user_destination": runtime.config.default_delivery_dir,
        })),
    )
        .into_response()
}

pub async fn api_config_get(State(state): State<WebState>) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };
    let bridge_config = match ensure_generated_config(&runtime) {
        Ok(config) => config,
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };
    let bridge_config_path = generated_config_path(&runtime);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "app_version": env!("CARGO_PKG_VERSION"),
            "config": runtime.config,
            "openclaw_bridge": {
                "config": bridge_config,
                "config_path": bridge_config_path,
                "note": "This file is generated and reused by managed OpenClaw bridge startup. `ruler_url` stays loopback for local bridge calls; `public_base_url` auto-prefers Tailscale when available and falls back to loopback."
            },
            "note": "ui_bind changes require restarting the UI process to take effect",
        })),
    )
        .into_response()
}

pub async fn api_config_update(
    State(state): State<WebState>,
    Json(payload): Json<RuntimePathsPayload>,
) -> impl IntoResponse {
    let mut runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    if let Err(err) = apply_runtime_config_patch(&mut runtime, &payload) {
        return error_response(StatusCode::BAD_REQUEST, err);
    }
    if let Err(err) = save_config(
        &runtime.config.state_dir.join("config.yaml"),
        &runtime.config,
    ) {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
    }
    let bridge_config = if let Some(bridge_payload) = payload.openclaw_bridge.as_ref() {
        let patch = OpenClawBridgeConfigPatch {
            poll_interval_seconds: bridge_payload.poll_interval_seconds,
            decision_ttl_seconds: bridge_payload.decision_ttl_seconds,
            short_id_length: bridge_payload.short_id_length,
            inbound_bind: bridge_payload.inbound_bind.clone(),
            state_file: bridge_payload.state_file.clone(),
            openclaw_bin: bridge_payload.openclaw_bin.clone(),
            agent_ruler_bin: bridge_payload.agent_ruler_bin.clone(),
        };
        match update_generated_config(&runtime, &patch) {
            Ok(config) => config,
            Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
        }
    } else {
        match ensure_generated_config(&runtime) {
            Ok(config) => config,
            Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
        }
    };
    let bridge_config_path = generated_config_path(&runtime);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "updated",
            "app_version": env!("CARGO_PKG_VERSION"),
            "config": runtime.config,
            "openclaw_bridge": {
                "config": bridge_config,
                "config_path": bridge_config_path,
            },
            "note": "ui_bind changes require restarting the UI process to take effect",
        })),
    )
        .into_response()
}

pub async fn api_receipts(
    State(state): State<WebState>,
    Query(query): Query<ReceiptQuery>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let mut items = receipts.read_all().unwrap_or_default();
    items.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    if let Some(date) = query.date.as_deref() {
        items.retain(|item| item.timestamp.to_rfc3339().starts_with(date));
    }

    if let Some(verdict) = query.verdict.as_deref() {
        let verdict = verdict.to_ascii_lowercase();
        items.retain(|item| {
            serde_json::to_string(&item.decision.verdict)
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains(&verdict)
        });
    }

    if let Some(action) = query.action.as_deref() {
        let action = action.to_ascii_lowercase();
        items.retain(|item| item.action.operation.to_ascii_lowercase().contains(&action));
    }

    if let Some(q) = query.q.as_deref() {
        let q = q.to_ascii_lowercase();
        items.retain(|item| {
            serde_json::to_string(item)
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains(&q)
        });
    }

    let total = items.len();
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let offset = query.offset.unwrap_or(0).min(total);
    let mut page_items = items
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    if !query.include_details.unwrap_or(false) {
        for item in &mut page_items {
            // Store the original command for summary before clearing
            let original_command = item.action.process.command.clone();

            // Clear sensitive details for non-debug view
            item.diff_summary = None;
            item.action.process.command.clear();

            // Create a safe command summary for display
            let command_summary = if original_command.is_empty() {
                String::new()
            } else {
                // Parse command to extract executable and first argument
                let parts: Vec<&str> = original_command.split_whitespace().collect();
                if parts.is_empty() {
                    String::new()
                } else {
                    // Extract just the executable name
                    let exe = parts[0]
                        .split('/')
                        .next_back()
                        .unwrap_or(parts[0])
                        .to_string();

                    // Add first argument if it exists and doesn't look like a flag
                    if parts.len() > 1 && !parts[1].starts_with('-') {
                        let arg = parts[1];

                        // Redact potential secrets in paths
                        if arg.contains("id_rsa")
                            || arg.contains("id_ed25519")
                            || arg.contains(".key")
                            || arg.contains(".pem")
                            || arg.contains("token")
                            || arg.contains("password")
                        {
                            format!("{} <redacted_path>", exe)
                        } else if arg.len() > 100 {
                            // Truncate very long paths
                            format!("{} {}...", exe, &arg[..80])
                        } else {
                            format!("{} {}", exe, arg)
                        }
                    } else {
                        exe
                    }
                }
            };

            // Store summary in metadata for frontend to use
            item.action
                .metadata
                .insert("_command_summary".to_string(), command_summary);
        }
    }
    let has_more = offset.saturating_add(page_items.len()) < total;

    (
        StatusCode::OK,
        Json(ReceiptPage {
            items: page_items,
            total,
            limit,
            offset,
            has_more,
        }),
    )
        .into_response()
}

pub fn append_control_panel_log(
    runtime: &RuntimeState,
    level: impl Into<String>,
    source: impl Into<String>,
    message: impl Into<String>,
    details: Option<serde_json::Value>,
) {
    let store = ui_log_store(runtime);
    let _ = store.append_event(level, source, message, details);
}

pub async fn api_ui_logs(
    State(state): State<WebState>,
    Query(query): Query<UiLogQuery>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let store = ui_log_store(&runtime);
    let mut items = store.read_all().unwrap_or_default();
    items.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    if let Some(level) = query.level.as_deref() {
        let level = level.to_ascii_lowercase();
        items.retain(|item| item.level.to_ascii_lowercase().contains(&level));
    }

    if let Some(source) = query.source.as_deref() {
        let source = source.to_ascii_lowercase();
        items.retain(|item| item.source.to_ascii_lowercase().contains(&source));
    }

    if let Some(q) = query.q.as_deref() {
        let q = q.to_ascii_lowercase();
        items.retain(|item| {
            serde_json::to_string(item)
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains(&q)
        });
    }

    let total = items.len();
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let offset = query.offset.unwrap_or(0).min(total);
    let page_items = items
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    let has_more = offset.saturating_add(page_items.len()) < total;

    (
        StatusCode::OK,
        Json(UiLogPage {
            items: page_items,
            total,
            limit,
            offset,
            has_more,
        }),
    )
        .into_response()
}

pub async fn api_ui_log_event(
    State(state): State<WebState>,
    Json(payload): Json<UiLogEventPayload>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let level = payload.level.trim().to_ascii_lowercase();
    let source = payload.source.trim();
    let message = payload.message.trim();

    if level.is_empty() || source.is_empty() || message.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "level, source, and message must not be empty".to_string(),
        );
    }

    append_control_panel_log(
        &runtime,
        level,
        source.to_string(),
        message.to_string(),
        payload.details,
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "logged" })),
    )
        .into_response()
}

pub async fn api_files_list(
    State(state): State<WebState>,
    Query(query): Query<FileListQuery>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let root = match query.zone.as_str() {
        "workspace" => runtime.config.workspace.clone(),
        "shared" => runtime.config.shared_zone_dir.clone(),
        "deliver" => runtime.config.default_delivery_dir.clone(),
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "zone must be workspace|shared|deliver".to_string(),
            )
        }
    };

    let prefix = query.prefix.unwrap_or_default();
    let scan_root = match resolve_scan_root(&root, &prefix) {
        Ok(path) => path,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err),
    };

    if !scan_root.exists() {
        return (StatusCode::OK, Json(Vec::<FileListItem>::new())).into_response();
    }

    let needle = query.q.unwrap_or_default().to_ascii_lowercase();
    let dirs_only = query.dirs_only.unwrap_or(false);
    let limit = query.limit.unwrap_or(300).clamp(1, 2000);
    let mut items = Vec::new();

    for entry in WalkDir::new(&scan_root)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        let path = entry.path();
        if path == scan_root {
            continue;
        }

        let relative = match path.strip_prefix(&root) {
            Ok(p) => p,
            Err(_) => continue,
        };

        let rel_text = relative.to_string_lossy().to_string();
        if !needle.is_empty() && !rel_text.to_ascii_lowercase().contains(&needle) {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(meta) => meta,
            Err(_) => continue,
        };

        if dirs_only && !metadata.is_dir() {
            continue;
        }

        let kind = if metadata.is_dir() {
            "dir"
        } else if metadata.is_file() {
            "file"
        } else {
            "other"
        };

        items.push(FileListItem {
            path: rel_text,
            kind: kind.to_string(),
            bytes: if metadata.is_file() {
                metadata.len()
            } else {
                0
            },
        });

        if items.len() >= limit {
            break;
        }
    }

    items.sort_by(|a, b| a.path.cmp(&b.path));
    (StatusCode::OK, Json(items)).into_response()
}

fn resolve_scan_root(root: &Path, prefix: &str) -> Result<std::path::PathBuf, String> {
    let trimmed = prefix.trim();
    if trimmed.is_empty() {
        return Ok(root.to_path_buf());
    }

    let normalized = trimmed.trim_start_matches('/');
    let rel = Path::new(normalized);
    if rel.is_absolute() {
        return Err("prefix must be a relative path inside the selected zone".to_string());
    }
    if rel.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("prefix cannot include parent directory traversals".to_string());
    }

    let candidate = root.join(rel);
    let candidate_canonical = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.clone());
    let root_canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if !candidate_canonical.starts_with(&root_canonical) {
        return Err("prefix resolved outside of selected zone".to_string());
    }

    Ok(candidate)
}

fn ui_log_store(runtime: &RuntimeState) -> UiLogStore {
    UiLogStore::new(
        runtime
            .config
            .runtime_root
            .join("user_data")
            .join("logs")
            .join(UI_EVENT_LOG_FILE_NAME),
    )
}

fn apply_runtime_config_patch(
    runtime: &mut crate::config::RuntimeState,
    payload: &RuntimePathsPayload,
) -> Result<(), String> {
    if let Some(shared_zone) = payload.shared_zone_path.as_deref() {
        let trimmed = shared_zone.trim();
        if !trimmed.is_empty() {
            let path = resolve_ui_path_update(
                &runtime.config.runtime_root,
                trimmed,
                payload.shared_zone_absolute.unwrap_or(false),
            );
            runtime.config.shared_zone_dir = path;
        }
    }

    if let Some(delivery) = payload.default_user_destination_path.as_deref() {
        let trimmed = delivery.trim();
        if !trimmed.is_empty() {
            let path = resolve_ui_path_update(
                &runtime.config.ruler_root,
                trimmed,
                payload.default_user_destination_absolute.unwrap_or(false),
            );
            runtime.config.default_delivery_dir = path;
        }
    }

    if let Some(bind) = payload.ui_bind.as_deref() {
        let trimmed = bind.trim();
        if trimmed.is_empty() {
            return Err("ui_bind must not be empty".to_string());
        }
        let parsed: SocketAddr = trimmed
            .parse()
            .map_err(|_| "ui_bind must be in host:port format".to_string())?;
        runtime.config.ui_bind = parsed.to_string();
    }

    if let Some(show_debug) = payload.ui_show_debug_tools {
        runtime.config.ui_show_debug_tools = show_debug;
    }

    if let Some(allow_degraded) = payload.allow_degraded_confinement {
        runtime.config.allow_degraded_confinement = allow_degraded;
    }

    if let Some(wait_timeout_secs) = payload.approval_wait_timeout_secs {
        if wait_timeout_secs == 0 {
            return Err("approval_wait_timeout_secs must be between 1 and 300".to_string());
        }
        runtime.config.approval_wait_timeout_secs = wait_timeout_secs.clamp(1, 300);
    }

    fs::create_dir_all(&runtime.config.shared_zone_dir)
        .map_err(|err| format!("create shared-zone dir failed: {err}"))?;
    fs::create_dir_all(&runtime.config.default_delivery_dir)
        .map_err(|err| format!("create default user destination dir failed: {err}"))?;

    Ok(())
}
