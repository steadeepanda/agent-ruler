use std::collections::BTreeSet;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use anyhow::{anyhow, Context, Result};
use axum::extract::{Path as AxPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use tower_http::services::{ServeDir, ServeFile};
use walkdir::WalkDir;

use crate::approvals::ApprovalStore;
use crate::config::{
    allowlisted_package_presets, denylisted_package_presets, load_policy, load_runtime,
    reset_layout, safe_domain_allowlist_presets, safe_domain_denylist_presets,
    safe_get_domain_allowlist_presets, safe_post_domain_allowlist_presets, save_policy,
    RuleDisposition, RuntimeState,
};
use crate::helpers::approvals::{
    append_approval_resolution_receipt, append_bulk_approval_resolution_receipt,
};
use crate::helpers::ui::payloads::{
    ApprovalQuery, ApprovalWaitQuery, ApprovalWaitResponse, BulkApprovalResult, DoctorPayload,
    RedactedStatusEvent, ResetRuntimePayload, RunCommandPayload, RunScriptPayload, StatusFeedQuery,
    TogglePayload, UpdateApplyPayload,
};
use crate::helpers::ui::{
    claudecode_tool_preflight as ui_claudecode_tool_preflight,
    openclaw_tool_preflight as ui_openclaw_tool_preflight,
    opencode_tool_preflight as ui_opencode_tool_preflight, pages as ui_pages,
    runner_command_api as ui_runner_command_api,
    runner_tool_preflight_common as ui_runner_tool_preflight_common, runtime_api as ui_runtime_api,
    transfer_api as ui_transfer_api,
};
use crate::helpers::{
    apply_profile_preset, approval_to_view, canonical_profile_id, enforce_minimum_safety_guards,
    maybe_apply_approval_effect, policy_profiles, profile_permissions, redacted_status_event,
};
use crate::model::ApprovalStatus;
use crate::policy::PolicyEngine;
use crate::receipts::ReceiptStore;
use crate::runner::run_confined;
use crate::utils::resolve_command_path;
use crate::{
    doctor,
    doctor::{DoctorOptions, RepairSelection},
};

const DOCS_ROOT_RELATIVE: &str = "docs-site/docs";
const DOCS_DIST_RELATIVE: &str = "docs-site/docs/.vitepress/dist";
const DOCS_PUBLIC_IMAGES_RELATIVE: &str = "docs-site/docs/public/images";

#[derive(Clone)]
pub struct WebState {
    pub ruler_root: PathBuf,
    pub runtime_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct DocsPaths {
    dist_dir: PathBuf,
    images_dir: PathBuf,
}

pub fn build_router(state: WebState) -> Router {
    let docs_paths = resolve_docs_paths(&state.ruler_root);
    let help_dir = docs_paths.dist_dir.clone();
    let help_images_dir = docs_paths.images_dir.clone();
    let help_service = ServeDir::new(help_dir.clone())
        .append_index_html_on_directories(true)
        .not_found_service(ServeFile::new(help_dir.join("index.html")));

    Router::new()
        .route("/", get(ui_pages::index_overview))
        .route("/approvals", get(ui_pages::index_approvals))
        .route("/approvals/:id", get(ui_pages::index_approval_detail))
        .route("/files", get(ui_pages::index_files))
        .route("/policy", get(ui_pages::index_policy))
        .route("/receipts", get(ui_pages::index_receipts))
        .route("/runners", get(ui_pages::index_runners))
        .route("/runtime", get(ui_pages::index_runtime))
        .route("/settings", get(ui_pages::index_settings))
        .route("/execution", get(ui_pages::index_execution))
        .route("/help-feedback", get(ui_pages::index_help_feedback))
        .route("/docs", get(ui_pages::index_docs))
        .route(
            "/assets/design-tokens.css",
            get(ui_pages::design_tokens_css),
        )
        .route("/assets/logo-mark.svg", get(ui_pages::logo_mark_svg))
        .nest_service("/help/images", ServeDir::new(help_images_dir))
        .nest_service("/help", help_service)
        .route("/assets/ui.css", get(ui_pages::ui_css))
        .route("/assets/ui.js", get(ui_pages::ui_js))
        .route("/api/status", get(ui_runtime_api::api_status))
        .route("/api/status/feed", get(api_status_feed))
        .route("/api/runtime", get(ui_runtime_api::api_runtime))
        .route("/api/runners", get(ui_runtime_api::api_runners))
        .route("/api/runners/:id", get(ui_runtime_api::api_runner_get))
        .route("/api/sessions", get(ui_runtime_api::api_sessions))
        .route(
            "/api/sessions/telegram/resolve",
            post(ui_runtime_api::api_session_telegram_resolve),
        )
        .route(
            "/api/sessions/:id/runner-session-key",
            post(ui_runtime_api::api_session_runner_key_set),
        )
        .route("/api/sessions/:id", get(ui_runtime_api::api_session_get))
        .route("/api/update/check", get(api_update_check))
        .route("/api/update/apply", post(api_update_apply))
        .route("/api/config", get(ui_runtime_api::api_config_get))
        .route(
            "/api/config/update",
            post(ui_runtime_api::api_config_update),
        )
        .route(
            "/api/runtime/paths",
            post(ui_runtime_api::api_runtime_paths),
        )
        .route("/api/receipts", get(ui_runtime_api::api_receipts))
        .route("/api/ui/logs", get(ui_runtime_api::api_ui_logs))
        .route("/api/ui/logs/event", post(ui_runtime_api::api_ui_log_event))
        .route("/api/files/list", get(ui_runtime_api::api_files_list))
        .route("/api/approvals", get(api_approvals))
        .route("/api/approvals/:id", get(api_approval_get))
        .route("/api/approvals/:id/wait", get(api_approval_wait))
        .route("/api/approvals/:id/approve", post(api_approve))
        .route("/api/approvals/:id/deny", post(api_deny))
        .route("/api/approvals/approve-all", post(api_approve_all))
        .route("/api/approvals/deny-all", post(api_deny_all))
        .route(
            "/api/exports/staged",
            get(ui_transfer_api::api_staged_exports),
        )
        .route(
            "/api/export/preview",
            post(ui_transfer_api::api_export_preview),
        )
        .route(
            "/api/export/request",
            post(ui_transfer_api::api_export_request),
        )
        .route(
            "/api/export/deliver/preview",
            post(ui_transfer_api::api_deliver_preview),
        )
        .route(
            "/api/export/deliver/request",
            post(ui_transfer_api::api_deliver_request),
        )
        .route(
            "/api/import/preview",
            post(ui_transfer_api::api_import_preview),
        )
        .route(
            "/api/import/upload",
            post(ui_transfer_api::api_import_upload),
        )
        .route(
            "/api/import/request",
            post(ui_transfer_api::api_import_request),
        )
        .route("/api/policy", get(api_policy))
        .route("/api/policy/profiles", get(api_policy_profiles))
        .route("/api/policy/domain-presets", get(api_policy_domain_presets))
        .route("/api/policy/toggles", post(api_policy_toggles))
        .route("/api/capabilities", get(api_capabilities))
        .route("/api/reset-exec", post(api_reset_exec))
        .route("/api/reset-runtime", post(api_reset_runtime))
        .route("/api/run/script", post(api_run_script))
        .route("/api/run/command", post(api_run_command))
        .route("/api/doctor", post(api_doctor))
        .route(
            "/api/runners/:id/tool/preflight",
            post(ui_runner_tool_preflight_common::api_runner_tool_preflight),
        )
        .route(
            "/api/openclaw/tool/preflight",
            post(ui_openclaw_tool_preflight::api_openclaw_tool_preflight),
        )
        .route(
            "/api/claudecode/tool/preflight",
            post(ui_claudecode_tool_preflight::api_claudecode_tool_preflight),
        )
        .route(
            "/api/opencode/tool/preflight",
            post(ui_opencode_tool_preflight::api_opencode_tool_preflight),
        )
        .with_state(state)
}

pub async fn serve(ruler_root: PathBuf, runtime_dir: Option<PathBuf>, bind: String) -> Result<()> {
    if let Err(err) = ensure_help_docs_bundle_current(&ruler_root) {
        let dist_index = resolve_docs_paths(&ruler_root).dist_dir.join("index.html");
        if !dist_index.exists() {
            return Err(err);
        }
        eprintln!("docs warning: {err}");
    }

    let state = WebState {
        ruler_root,
        runtime_dir,
    };
    let app = build_router(state);

    let socket: SocketAddr = bind.parse().context("parse bind address")?;
    let listener = tokio::net::TcpListener::bind(socket)
        .await
        .with_context(|| format!("bind {}", bind))?;

    println!("Agent Ruler UI listening at http://{}", bind);
    if let Some(local_mirror) = loopback_mirror_addr(socket) {
        let local_listener = tokio::net::TcpListener::bind(local_mirror)
            .await
            .with_context(|| format!("bind {}", local_mirror))?;
        println!(
            "Agent Ruler local API mirror listening at http://{}",
            local_mirror
        );
        tokio::try_join!(
            axum::serve(listener, app.clone()),
            axum::serve(local_listener, app)
        )
        .context("serve ui")?;
    } else {
        axum::serve(listener, app).await.context("serve ui")?;
    }
    Ok(())
}

fn loopback_mirror_addr(primary: SocketAddr) -> Option<SocketAddr> {
    match primary {
        SocketAddr::V4(addr) => {
            if addr.ip().is_loopback() || addr.ip().is_unspecified() {
                None
            } else {
                Some(SocketAddr::from(([127, 0, 0, 1], addr.port())))
            }
        }
        SocketAddr::V6(addr) => {
            if addr.ip().is_loopback() || addr.ip().is_unspecified() {
                None
            } else {
                Some(SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], addr.port())))
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use std::net::SocketAddr;
    use std::{fs, path::PathBuf};

    use super::loopback_mirror_addr;
    use super::{
        manifest_docs_dist_dir, resolve_docs_paths, shell_single_quote, ui_command_summary_line,
        wrap_one_shot_script_with_exe,
    };
    use crate::config::{init_layout, load_runtime};
    use tempfile::tempdir;

    #[test]
    fn loopback_mirror_added_for_concrete_ipv4_bind() {
        let primary: SocketAddr = "100.64.12.34:4622".parse().expect("parse socket");
        let mirror = loopback_mirror_addr(primary).expect("expected mirror");
        assert_eq!(mirror.to_string(), "127.0.0.1:4622");
    }

    #[test]
    fn loopback_mirror_not_added_for_loopback_bind() {
        let primary: SocketAddr = "127.0.0.1:4622".parse().expect("parse socket");
        assert!(loopback_mirror_addr(primary).is_none());
    }

    #[test]
    fn resolve_docs_paths_prefers_runtime_bundle() {
        let temp = tempdir().expect("tempdir");
        let ruler_root = temp.path().join("agent-ruler").join("installs");
        let runtime_dist = ruler_root.join("docs-site/docs/.vitepress/dist");
        let runtime_images = ruler_root.join("docs-site/docs/public/images");
        fs::create_dir_all(&runtime_dist).expect("create runtime dist");
        fs::create_dir_all(&runtime_images).expect("create runtime images");
        fs::write(runtime_dist.join("index.html"), "<html></html>").expect("write index");

        let resolved = resolve_docs_paths(&ruler_root);
        assert_eq!(resolved.dist_dir, runtime_dist);
        assert_eq!(resolved.images_dir, runtime_images);
    }

    #[test]
    fn resolve_docs_paths_falls_back_to_manifest_dist() {
        let temp = tempdir().expect("tempdir");
        let resolved = resolve_docs_paths(temp.path());
        let expected: PathBuf = manifest_docs_dist_dir();
        assert_eq!(resolved.dist_dir, expected);
    }

    #[test]
    fn shell_single_quote_escapes_single_quotes() {
        assert_eq!(shell_single_quote("a'b"), "'a'\"'\"'b'");
    }

    #[test]
    fn wrap_one_shot_script_binds_agent_ruler_to_runtime() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        let runtime_root = temp.path().join("runtime");
        init_layout(&project, Some(&runtime_root), None, true).expect("init layout");
        let runtime = load_runtime(&project, Some(&runtime_root)).expect("load runtime");

        let resolved = temp.path().join("agent-ruler");
        fs::write(&resolved, "#!/bin/sh\nexit 0\n").expect("write fake agent-ruler");
        let script =
            wrap_one_shot_script_with_exe(&runtime, "agent-ruler status --json", &resolved)
                .expect("wrap one-shot script");

        assert!(script.contains("alias agent-ruler=agent_ruler_managed"));
        assert!(script.contains(resolved.to_string_lossy().as_ref()));
        assert!(script.contains(runtime.config.runtime_root.to_string_lossy().as_ref()));
        assert!(script.contains(runtime.config.ruler_root.to_string_lossy().as_ref()));
        assert!(script.contains("AGENT_RULER_ROOT="));
        assert!(script.contains("AGENT_RULER_SUPPRESS_UI_AUTOBIND=1"));
        assert!(script.contains("if [ -d"));
        assert!(script.contains("Use `agent-ruler run -- %s ...` in One-Shot Command"));
        assert_eq!(
            ui_command_summary_line(&runtime),
            format!("WebUI runtime: {}", runtime.config.runtime_root.display())
        );
    }
}

fn docs_dist_dir(ruler_root: &Path) -> PathBuf {
    ruler_root.join(DOCS_DIST_RELATIVE)
}

fn docs_public_images_dir(ruler_root: &Path) -> PathBuf {
    ruler_root.join(DOCS_PUBLIC_IMAGES_RELATIVE)
}

fn manifest_docs_root_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DOCS_ROOT_RELATIVE)
}

fn manifest_docs_dist_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DOCS_DIST_RELATIVE)
}

fn manifest_docs_public_images_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DOCS_PUBLIC_IMAGES_RELATIVE)
}

fn resolve_docs_paths(ruler_root: &Path) -> DocsPaths {
    let runtime_dist = docs_dist_dir(ruler_root);
    let runtime_index = runtime_dist.join("index.html");
    let manifest_dist = manifest_docs_dist_dir();
    let dist_dir = if runtime_index.exists() {
        runtime_dist.clone()
    } else {
        manifest_dist
    };

    let runtime_public_images = docs_public_images_dir(ruler_root);
    let runtime_dist_images = runtime_dist.join("images");
    let manifest_public_images = manifest_docs_public_images_dir();
    let manifest_dist_images = manifest_docs_dist_dir().join("images");
    let images_dir = if runtime_public_images.exists() {
        runtime_public_images
    } else if runtime_dist_images.exists() {
        runtime_dist_images
    } else if manifest_public_images.exists() {
        manifest_public_images
    } else if manifest_dist_images.exists() {
        manifest_dist_images
    } else {
        docs_public_images_dir(ruler_root)
    };

    DocsPaths {
        dist_dir,
        images_dir,
    }
}

fn ensure_help_docs_bundle_current(ruler_root: &Path) -> Result<()> {
    let docs_root = manifest_docs_root_dir();
    let docs_dist = manifest_docs_dist_dir();
    if !docs_root.exists() {
        let resolved = resolve_docs_paths(ruler_root);
        if resolved.dist_dir.join("index.html").exists() {
            return Ok(());
        }
        return Err(anyhow::anyhow!(
            "help docs bundle missing; expected index at {}",
            resolved.dist_dir.join("index.html").display()
        ));
    }

    if !docs_bundle_needs_build(&docs_root, &docs_dist)? {
        return Ok(());
    }

    if !npm_available() {
        return Err(anyhow!(
            "help docs bundle is missing or stale and requires npm to rebuild; install Node.js + npm and run `npm --prefix docs-site run docs:build`"
        ));
    }

    eprintln!("docs: rebuilding help bundle from markdown sources...");
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    build_help_docs_bundle(&manifest_dir)
}

fn docs_bundle_needs_build(docs_root: &Path, docs_dist: &Path) -> Result<bool> {
    let dist_index = docs_dist.join("index.html");
    if !dist_index.exists() {
        return Ok(true);
    }

    let dist_modified = modified_time(&dist_index)?;
    let source_modified = newest_docs_source_mtime(docs_root)?.unwrap_or(SystemTime::UNIX_EPOCH);
    Ok(source_modified > dist_modified)
}

fn newest_docs_source_mtime(docs_root: &Path) -> Result<Option<SystemTime>> {
    let mut newest: Option<SystemTime> = None;
    let dist_dir = docs_root.join(".vitepress").join("dist");
    let cache_dir = docs_root.join(".vitepress").join("cache");

    for entry in WalkDir::new(docs_root)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        let path = entry.path();
        if path.starts_with(&dist_dir) || path.starts_with(&cache_dir) {
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }

        let modified = modified_time(path)?;
        newest = Some(match newest {
            Some(current) => current.max(modified),
            None => modified,
        });
    }

    let docs_site_dir = docs_root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("docs root is missing parent directory"))?;
    for extra in [
        docs_site_dir.join("package.json"),
        docs_site_dir.join("scripts").join("sync-curated-docs.mjs"),
    ] {
        if !extra.exists() {
            continue;
        }
        let modified = modified_time(&extra)?;
        newest = Some(match newest {
            Some(current) => current.max(modified),
            None => modified,
        });
    }

    Ok(newest)
}

fn modified_time(path: &Path) -> Result<SystemTime> {
    fs::metadata(path)
        .with_context(|| format!("read metadata for {}", path.display()))?
        .modified()
        .with_context(|| format!("read modified time for {}", path.display()))
}

fn build_help_docs_bundle(manifest_dir: &Path) -> Result<()> {
    let status = Command::new("npm")
        .args(["--prefix", "docs-site", "run", "docs:build"])
        .current_dir(manifest_dir)
        .status()
        .context("run `npm --prefix docs-site run docs:build`")?;
    if !status.success() {
        return Err(anyhow::anyhow!(
            "help docs build failed (exit: {})",
            status.code().map_or_else(
                || "terminated by signal".to_string(),
                |code| code.to_string()
            )
        ));
    }
    Ok(())
}

fn npm_available() -> bool {
    Command::new("npm")
        .arg("--version")
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn normalize_runner_filter(value: Option<&str>) -> Option<String> {
    let trimmed = value.map(str::trim).unwrap_or_default();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("all") {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

async fn api_approvals(
    State(state): State<WebState>,
    Query(query): Query<ApprovalQuery>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let runner_filter = normalize_runner_filter(query.runner.as_deref());
    let data = approvals
        .list_pending()
        .unwrap_or_default()
        .into_iter()
        .filter(|approval| match runner_filter.as_deref() {
            Some(filter) => approval
                .action
                .metadata
                .get("runner_id")
                .map(|value| value.trim().eq_ignore_ascii_case(filter))
                .unwrap_or(false),
            None => true,
        })
        .map(approval_to_view)
        .collect::<Vec<_>>();
    (StatusCode::OK, Json(data)).into_response()
}

async fn api_approval_get(
    State(state): State<WebState>,
    AxPath(id): AxPath<String>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    match approvals.get(&id) {
        Ok(Some(record)) => {
            let view = approval_to_view(record);
            (StatusCode::OK, Json(view)).into_response()
        }
        Ok(None) => error_response(StatusCode::NOT_FOUND, format!("Approval {} not found", id)),
        Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    }
}

async fn api_approval_wait(
    State(state): State<WebState>,
    AxPath(id): AxPath<String>,
    Query(query): Query<ApprovalWaitQuery>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let engine = PolicyEngine::new(runtime.policy.clone(), runtime.config.workspace.clone());
    let timeout_secs = query
        .timeout
        .unwrap_or(runtime.config.approval_wait_timeout_secs)
        .clamp(1, 300);
    let poll_ms = query.poll_ms.unwrap_or(500).clamp(100, 2000);
    let started = std::time::Instant::now();
    let timeout = tokio::time::Duration::from_secs(timeout_secs);

    loop {
        match approvals.get(&id) {
            Ok(Some(record)) => {
                if record.status != ApprovalStatus::Pending {
                    // Return a redacted status snapshot so agent clients can
                    // act on resolution without learning sensitive file details.
                    let event = redacted_status_event(&engine, &record);
                    return (
                        StatusCode::OK,
                        Json(ApprovalWaitResponse {
                            resolved: true,
                            timeout: false,
                            event,
                        }),
                    )
                        .into_response();
                }

                if started.elapsed() >= timeout {
                    // Timeout still returns the latest redacted state instead of
                    // an error to support polling clients.
                    let event = redacted_status_event(&engine, &record);
                    return (
                        StatusCode::OK,
                        Json(ApprovalWaitResponse {
                            resolved: false,
                            timeout: true,
                            event,
                        }),
                    )
                        .into_response();
                }
            }
            Ok(None) => {
                return error_response(StatusCode::NOT_FOUND, format!("Approval {} not found", id));
            }
            Err(err) => {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(poll_ms)).await;
    }
}

async fn api_status_feed(
    State(state): State<WebState>,
    Query(query): Query<StatusFeedQuery>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let _ = approvals.list_pending();
    let mut records = approvals.list_all().unwrap_or_default();
    let engine = PolicyEngine::new(runtime.policy.clone(), runtime.config.workspace.clone());
    records.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    if !query.include_resolved.unwrap_or(true) {
        records.retain(|record| record.status == ApprovalStatus::Pending);
    }

    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let events: Vec<RedactedStatusEvent> = records
        .into_iter()
        .take(limit)
        // Feed payload is intentionally redacted by helper to avoid exposing
        // full action metadata to all API consumers.
        .map(|record| redacted_status_event(&engine, &record))
        .collect();

    (StatusCode::OK, Json(events)).into_response()
}

async fn api_approve(
    State(state): State<WebState>,
    AxPath(id): AxPath<String>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    match approvals.approve_idempotent(&id) {
        Ok(update) => {
            if update.changed {
                if let Err(err) = append_approval_resolution_receipt(
                    &receipts,
                    &runtime,
                    &update.approval,
                    "approval-resolution-approve",
                ) {
                    return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
                }
                if let Err(err) = maybe_apply_approval_effect(&runtime, &update.approval, &receipts)
                {
                    return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
                }
            }
            (StatusCode::OK, Json(update.approval)).into_response()
        }
        Err(err) => error_response(StatusCode::BAD_REQUEST, err.to_string()),
    }
}

async fn api_deny(State(state): State<WebState>, AxPath(id): AxPath<String>) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    match approvals.deny_idempotent(&id) {
        Ok(update) => {
            if update.changed {
                if let Err(err) = append_approval_resolution_receipt(
                    &receipts,
                    &runtime,
                    &update.approval,
                    "approval-resolution-deny",
                ) {
                    return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
                }
            }
            (StatusCode::OK, Json(update.approval)).into_response()
        }
        Err(err) => error_response(StatusCode::BAD_REQUEST, err.to_string()),
    }
}

async fn api_approve_all(State(state): State<WebState>) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let pending = approvals.list_pending().unwrap_or_default();

    let mut updated = Vec::new();
    let mut failed = Vec::new();
    let mut resolved = Vec::new();
    for item in pending {
        match approvals.approve(&item.id) {
            Ok(approval) => {
                if let Err(err) = maybe_apply_approval_effect(&runtime, &approval, &receipts) {
                    failed.push(format!("{}: {}", approval.id, err));
                    continue;
                }
                updated.push(approval.id.clone());
                resolved.push(approval);
            }
            Err(err) => failed.push(format!("{}: {}", item.id, err)),
        }
    }
    if let Err(err) = append_bulk_approval_resolution_receipt(
        &receipts,
        &runtime,
        &resolved,
        true,
        "approval-resolution-approve-all",
    ) {
        failed.push(format!("bulk: {}", err));
    }

    (StatusCode::OK, Json(BulkApprovalResult { updated, failed })).into_response()
}

async fn api_deny_all(State(state): State<WebState>) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let pending = approvals.list_pending().unwrap_or_default();

    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let mut updated = Vec::new();
    let mut failed = Vec::new();
    let mut resolved = Vec::new();
    for item in pending {
        match approvals.deny(&item.id) {
            Ok(approval) => {
                updated.push(approval.id.clone());
                resolved.push(approval);
            }
            Err(err) => failed.push(format!("{}: {}", item.id, err)),
        }
    }
    if let Err(err) = append_bulk_approval_resolution_receipt(
        &receipts,
        &runtime,
        &resolved,
        false,
        "approval-resolution-deny-all",
    ) {
        failed.push(format!("bulk: {}", err));
    }

    (StatusCode::OK, Json(BulkApprovalResult { updated, failed })).into_response()
}

async fn api_policy(State(state): State<WebState>) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    (StatusCode::OK, Json(runtime.policy)).into_response()
}

async fn api_policy_profiles() -> impl IntoResponse {
    (StatusCode::OK, Json(policy_profiles())).into_response()
}

async fn api_policy_domain_presets() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            // Legacy keys kept for compatibility with older UIs.
            "safe_defaults": safe_domain_allowlist_presets(),
            "denylist_defaults": safe_domain_denylist_presets(),
            // Preferred explicit keys.
            "post_allowlist_defaults": safe_post_domain_allowlist_presets(),
            "get_allowlist_defaults": safe_get_domain_allowlist_presets(),
            "allowlisted_packages": allowlisted_package_presets(),
            "denylisted_packages": denylisted_package_presets(),
            "note": "Safe defaults are optional and editable. You can disable or remove entries anytime.",
        })),
    )
        .into_response()
}

async fn api_policy_toggles(
    State(state): State<WebState>,
    Json(payload): Json<TogglePayload>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let mut policy = match load_policy(&runtime.config.policy_file) {
        Ok(policy) => policy,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };
    if let Some(canonical) = canonical_profile_id(&policy.profile) {
        policy.profile = canonical.to_string();
    }

    if let Some(profile) = payload.profile.as_deref() {
        let normalized = match canonical_profile_id(profile) {
            Some(profile) => profile,
            None => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    format!("unsupported profile: {profile}"),
                )
            }
        };
        if let Err(err) = apply_profile_preset(&mut policy, normalized) {
            return error_response(StatusCode::BAD_REQUEST, err.to_string());
        }
    }

    if payload.create_custom_profile.unwrap_or(false) {
        let active = canonical_profile_id(&policy.profile).unwrap_or("strict");
        if !profile_permissions(active).can_create_custom_profile {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("profile `{active}` cannot create another custom profile"),
            );
        }
        policy.profile = "custom".to_string();
    }

    let active_profile = canonical_profile_id(&policy.profile).unwrap_or("strict");
    let permissions = profile_permissions(active_profile);

    if toggle_payload_requests_elevation_changes(&payload)
        && !permissions.allow_elevation_customization
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "profile `{}` locks elevation controls; switch to Balanced, Coding/Nerd, I DON'T CARE, or Custom",
                active_profile
            ),
        );
    }

    if toggle_payload_requests_advanced_changes(&payload) && !permissions.allow_rule_customization {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "profile `{}` locks advanced filesystem/execution/persistence controls",
                active_profile
            ),
        );
    }

    if let Some(value) = payload.network_default_deny {
        policy.rules.network.default_deny = value;
    }
    if let Some(value) = payload.network_require_approval_for_post {
        policy.rules.network.require_approval_for_post = value;
    }
    if let Some(values) = payload.network_allowlist_hosts {
        policy.rules.network.allowlist_hosts = normalize_nonempty_unique(values);
    }
    if let Some(values) = payload.network_denylist_hosts {
        policy.rules.network.denylist_hosts = normalize_nonempty_unique(values);
    }
    if let Some(value) = payload.network_invert_allowlist {
        policy.rules.network.invert_allowlist = value;
    }
    if let Some(value) = payload.network_invert_denylist {
        policy.rules.network.invert_denylist = value;
    }

    if let Some(value) = payload.elevation_enabled {
        policy.rules.elevation.enabled = value;
    }
    if let Some(value) = payload.elevation_require_operator_auth {
        policy.rules.elevation.require_operator_auth = value;
    }
    if let Some(value) = payload.elevation_use_allowlist {
        policy.rules.elevation.use_allowlist = value;
    }
    if let Some(values) = payload.elevation_allowed_packages {
        policy.rules.elevation.allowed_packages = normalize_nonempty_unique(values);
    }
    if let Some(values) = payload.elevation_denied_packages {
        policy.rules.elevation.denied_packages = normalize_nonempty_unique(values);
    }

    if let Some(value) = payload.require_shared_approval {
        policy.rules.filesystem.shared = if value {
            RuleDisposition::Approval
        } else {
            RuleDisposition::Allow
        };
    }

    if let Some(value) = payload.filesystem_workspace {
        policy.rules.filesystem.workspace = value;
    }
    if let Some(value) = payload.filesystem_user_data {
        policy.rules.filesystem.user_data = value;
    }
    if let Some(value) = payload.filesystem_shared {
        policy.rules.filesystem.shared = value;
    }
    if let Some(value) = payload.filesystem_secrets {
        policy.rules.filesystem.secrets = value;
    }

    if let Some(value) = payload.execution_deny_workspace_exec {
        policy.rules.execution.deny_workspace_exec = value;
    }
    if let Some(value) = payload.execution_deny_tmp_exec {
        policy.rules.execution.deny_tmp_exec = value;
    }
    if let Some(value) = payload.execution_quarantine_on_download_exec_chain {
        policy.rules.execution.quarantine_on_download_exec_chain = value;
    }
    if let Some(values) = payload.execution_allowed_exec_prefixes {
        policy.rules.execution.allowed_exec_prefixes = normalize_nonempty_unique(values);
    }

    if let Some(value) = payload.persistence_deny_autostart {
        policy.rules.persistence.deny_autostart = value;
    }
    if let Some(values) = payload.persistence_approval_paths {
        policy.rules.persistence.approval_paths = normalize_nonempty_unique(values);
    }
    if let Some(values) = payload.persistence_deny_paths {
        policy.rules.persistence.deny_paths = normalize_nonempty_unique(values);
    }

    if let Some(value) = payload.safeguards_mass_delete_threshold {
        match parse_mass_delete_threshold(value) {
            Ok(threshold) => policy.safeguards.mass_delete_threshold = threshold,
            Err(err) => return error_response(StatusCode::BAD_REQUEST, err),
        }
    }

    enforce_minimum_safety_guards(&mut policy);
    if let Err(err) = save_policy(&runtime.config.policy_file, &policy) {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
    }

    let refreshed = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };

    (StatusCode::OK, Json(refreshed.policy)).into_response()
}

/// Agent-safe capabilities contract endpoint.
///
/// This endpoint provides a minimal, safe contract for confined agents and runner adapters
/// to discover what API surface is available to them. It is designed to be:
/// - Safe to call from within confinement (no sensitive data exposed)
/// - Stable across versions (structure won't change unexpectedly)
/// - Self-documenting (agents can understand capabilities without reading docs)
///
/// # Security Guarantees
///
/// This endpoint NEVER exposes:
/// - Runtime filesystem paths
/// - Policy contents or current toggle states
/// - Receipt internals or approval queue internals
/// - Secrets or operator-only details
/// - Configuration file locations
///
/// # Agent-Safe API Surface
///
/// The following endpoints are safe for confined agents to call:
/// - GET /api/status/feed - Redacted approval status feed
/// - GET /api/approvals/:id/wait - Wait for approval resolution
/// - POST /api/runners/:id/tool/preflight - Preflight evaluation for tool calls
/// - POST /api/export/request - Request staging of files (may require approval)
/// - POST /api/export/deliver/request - Request delivery (may require approval)
/// - POST /api/import/request - Request import into workspace (may require approval)
///
/// # Operator-Only Endpoints (NOT available to agents)
///
/// These endpoints require operator access (WebUI):
/// - All /api/policy/* endpoints
/// - GET /api/status (exposes runtime paths)
/// - GET /api/runtime (exposes filesystem paths)
/// - GET /api/approvals (full approval queue)
/// - /api/approvals/:id/approve
/// - /api/approvals/:id/deny
/// - /api/approvals/approve-all
/// - /api/approvals/deny-all
/// - /api/reset-exec
/// - /api/reset-runtime
/// - /api/runtime/paths
async fn api_capabilities() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "api_version": env!("CARGO_PKG_VERSION"),
            "name": "agent-ruler",
            "description": "Deterministic reference monitor and confinement runner for AI agents",

            // Feature flags for agent-safe functionality
            "features": {
                "approval_wait": true,
                "approval_wait_timeout_configurable": true,
                "status_feed": true,
                "redacted_views": true,
                "tool_preflight": true,
                "export_staging": true,
                "delivery_request": true,
                "import_request": true
            },

            // Agent-safe API surface - what confined agents can call
            "agent_safe_endpoints": {
                "status_feed": {
                    "method": "GET",
                    "path": "/api/status/feed",
                    "description": "Redacted approval status feed for polling",
                    "params": {
                        "include_resolved": "boolean (default true)",
                        "limit": "integer (default 100, max 500)"
                    },
                    "returns": ["approval_id", "verdict", "reason_code", "category", "guidance", "updated_at"],
                    "redaction_guarantee": "No raw paths, no policy content, no secrets"
                },
                "approval_wait": {
                    "method": "GET",
                    "path": "/api/approvals/:id/wait",
                    "description": "Long-poll for approval resolution",
                    "params": {
                        "id": "approval UUID from status feed",
                        "timeout": "seconds (default from Control Panel setting; initial 90, max 300)",
                        "poll_ms": "milliseconds (default 500, range 100-2000)"
                    },
                    "returns": ["approval_id", "verdict", "reason_code", "resolved", "timeout"]
                },
                "tool_preflight": {
                    "method": "POST",
                    "path": "/api/runners/:id/tool/preflight",
                    "description": "Preflight evaluation for tool calls before execution",
                    "params": {
                        "id": "runner id (openclaw | claudecode | opencode)",
                        "tool": "tool name (write, edit, delete, move, read, exec)",
                        "args": "tool-specific arguments"
                    },
                    "returns": ["verdict", "reason_code", "approval_id (if pending)", "guidance"],
                    "note": "Used by runner adapter before tool execution"
                },
                "export_request": {
                    "method": "POST",
                    "path": "/api/export/request",
                    "description": "Request staging of workspace files to shared-zone",
                    "params": {
                        "src": "source path in workspace",
                        "dst": "optional destination filename"
                    },
                    "returns": ["status", "approval_id (if pending)", "staged_path"],
                    "note": "May require approval depending on policy"
                },
                "deliver_request": {
                    "method": "POST",
                    "path": "/api/export/deliver/request",
                    "description": "Request delivery from shared-zone to external destination",
                    "params": {
                        "src": "source path in shared-zone",
                        "dst": "destination path or URL"
                    },
                    "returns": ["status", "approval_id (if pending)", "delivered_to"],
                    "note": "Requires approval for external destinations"
                },
                "import_request": {
                    "method": "POST",
                    "path": "/api/import/request",
                    "description": "Request import into workspace from a user-provided source",
                    "params": {
                        "src": "source path to import (host/shared/runtime upload)",
                        "dst": "optional destination path under workspace"
                    },
                    "returns": ["status", "approval_id (if pending)", "message"],
                    "note": "May require approval depending on policy"
                }
            },

            // Operator-only endpoints - agents MUST NOT use these
            "operator_only_endpoints": [
                "/api/status",
                "/api/runtime",
                "/api/runners",
                "/api/runners/:id",
                "/api/policy",
                "/api/policy/profiles",
                "/api/policy/domain-presets",
                "/api/policy/toggles",
                "/api/approvals",
                "/api/approvals/:id/approve",
                "/api/approvals/:id/deny",
                "/api/approvals/approve-all",
                "/api/approvals/deny-all",
                "/api/reset-exec",
                "/api/reset-runtime",
                "/api/runtime/paths",
                "/api/update/check",
                "/api/update/apply",
                "/api/ui/logs",
                "/api/ui/logs/event",
                "/api/run/script",
                "/api/run/command",
                "/api/doctor"
            ],

            // Tool-to-endpoint mapping for runner adapters (OpenClaw, etc.)
            "tool_mapping": {
                "agent_ruler_status_feed": "/api/status/feed",
                "agent_ruler_wait_for_approval": "/api/approvals/:id/wait",
                "agent_ruler_request_export_stage": "/api/export/request",
                "agent_ruler_request_delivery": "/api/export/deliver/request",
                "agent_ruler_request_import": "/api/import/request",
                "before_tool_call": "/api/runners/:id/tool/preflight",
                "before_tool_call_openclaw": "/api/openclaw/tool/preflight",
                "before_tool_call_claudecode": "/api/claudecode/tool/preflight",
                "before_tool_call_opencode": "/api/opencode/tool/preflight"
            },

            // Canonical workflow hints for ambiguous user intents.
            "workflow_contract": {
                "send_or_share_to_user": [
                    "call /api/export/request first (stage workspace artifact)",
                    "then call /api/export/deliver/request (deliver staged artifact)"
                ],
                "import_into_workspace": [
                    "call /api/import/request"
                ],
                "pending_approval_behavior": [
                    "wait using /api/approvals/:id/wait",
                    "approval/deny decisions are user-only via operator channels",
                    "do not submit approval decisions from agent context"
                ],
                "forbidden_bypass_patterns": [
                    "direct shell copy across boundary zones",
                    "direct outbound send outside delivery workflow",
                    "calling operator-only endpoints"
                ]
            },

            // Data classes that are NEVER exposed to agents
            "excluded_data_classes": [
                "runtime_filesystem_paths",
                "policy_contents",
                "policy_toggles",
                "receipt_internals",
                "approval_queue_internals",
                "secrets",
                "operator_auth_details",
                "configuration_files"
            ],

            // Redaction guarantees
            "redaction_guarantees": {
                "paths": "Filesystem paths are replaced with zone classifications",
                "policy": "Policy content is never exposed",
                "secrets": "Secret values are never included",
                "internals": "Runtime state internals are not accessible"
            }
        })),
    )
        .into_response()
}

async fn api_reset_exec(State(state): State<WebState>) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    if runtime.config.exec_layer_dir.exists() {
        if let Err(err) = fs::remove_dir_all(&runtime.config.exec_layer_dir) {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("remove exec layer failed: {err}"),
            );
        }
    }

    match fs::create_dir_all(&runtime.config.exec_layer_dir) {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "reset",
                "message": "Ephemeral execution artifacts were reset. Workspace and policy were not deleted.",
                "exec_layer": runtime.config.exec_layer_dir,
            })),
        )
            .into_response(),
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("create exec layer failed: {err}"),
        ),
    }
}

async fn api_reset_runtime(
    State(state): State<WebState>,
    Json(payload): Json<ResetRuntimePayload>,
) -> impl IntoResponse {
    let keep_config = payload.keep_config.unwrap_or(false);
    let runtime_before = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let ruler_root = runtime_before.config.ruler_root.clone();
    let runtime_dir = state.runtime_dir.clone();
    let reset_result = match reset_layout(&ruler_root, runtime_dir.as_deref(), keep_config) {
        Ok(config) => config,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let message = if keep_config {
        "Runtime reset completed. Existing config/policy paths were preserved."
    } else {
        "Runtime reset completed. Config and policy were restored to defaults."
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "reset",
            "keep_config": keep_config,
            "message": message,
            "runtime_root": reset_result.runtime_root,
            "workspace": reset_result.workspace,
            "shared_zone": reset_result.shared_zone_dir,
            "state_dir": reset_result.state_dir,
            "config_file": reset_result.state_dir.join("config.yaml"),
            "policy_file": reset_result.state_dir.join("policy.yaml"),
            "config_impact": if keep_config {
                "preserved_existing_config_and_policy"
            } else {
                "restored_default_config_and_policy"
            },
        })),
    )
        .into_response()
}

async fn api_run_script(
    State(state): State<WebState>,
    Json(payload): Json<RunScriptPayload>,
) -> impl IntoResponse {
    if payload.script.trim().is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "script must not be empty".to_string(),
        );
    }

    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let script = match wrap_one_shot_script(&runtime, &payload.script) {
        Ok(script) => script,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };
    let cmd = vec![
        "env".to_string(),
        "AGENT_RULER_UI_ONE_SHOT=1".to_string(),
        "bash".to_string(),
        "-lc".to_string(),
        script,
    ];
    let summary = ui_command_summary_line(&runtime);

    run_ui_command(runtime, cmd, Some(summary))
}

async fn api_run_command(
    State(state): State<WebState>,
    Json(payload): Json<RunCommandPayload>,
) -> impl IntoResponse {
    if payload.cmd.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "cmd must contain at least one token".to_string(),
        );
    }

    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let prepared = match ui_runner_command_api::prepare_ui_command(&runtime, &payload.cmd) {
        Ok(value) => value,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let summary = ui_command_summary_line(&runtime);
    run_ui_command(runtime, prepared, Some(summary))
}

async fn api_doctor(
    State(state): State<WebState>,
    Json(payload): Json<DoctorPayload>,
) -> impl IntoResponse {
    let mut runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let repair = payload.repair.unwrap_or(false);
    match doctor::run(
        &mut runtime,
        DoctorOptions {
            repair: if repair {
                RepairSelection::All
            } else {
                RepairSelection::None
            },
        },
    ) {
        Ok(report) => (StatusCode::OK, Json(serde_json::json!(report))).into_response(),
        Err(err) => error_response(StatusCode::BAD_REQUEST, err.to_string()),
    }
}

async fn api_update_check(State(state): State<WebState>) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };
    match run_update_subcommand(&runtime, &["--check", "--json"], false) {
        Ok(payload) => {
            let latest_tag = payload
                .get("check")
                .and_then(|check| check.get("latest_tag"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let update_available = payload
                .get("check")
                .and_then(|check| check.get("update_available"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            ui_runtime_api::append_control_panel_log(
                &runtime,
                "info",
                "update-check",
                "Update check completed",
                Some(serde_json::json!({
                    "latest_tag": latest_tag,
                    "update_available": update_available
                })),
            );
            (StatusCode::OK, Json(payload)).into_response()
        }
        Err(err) => {
            let message = err.to_string();
            ui_runtime_api::append_control_panel_log(
                &runtime,
                "warning",
                "update-check",
                format!("Update check failed: {message}"),
                None,
            );
            error_response(StatusCode::BAD_REQUEST, message)
        }
    }
}

async fn api_update_apply(
    State(state): State<WebState>,
    Json(payload): Json<UpdateApplyPayload>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let mut args = vec![
        "--yes".to_string(),
        "--json".to_string(),
        "--from-ui".to_string(),
    ];
    if let Some(version) = payload.version.as_deref() {
        let trimmed = version.trim();
        if trimmed.is_empty() {
            return error_response(
                StatusCode::BAD_REQUEST,
                "version must not be empty".to_string(),
            );
        }
        args.push("--version".to_string());
        args.push(trimmed.to_string());
    }

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match run_update_subcommand(&runtime, &arg_refs, true) {
        Ok(result) => {
            let target_tag = result
                .get("result")
                .and_then(|inner| inner.get("target_tag"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_else(|| payload.version.as_deref().unwrap_or("latest"))
                .to_string();
            ui_runtime_api::append_control_panel_log(
                &runtime,
                "info",
                "update-apply",
                format!("Update applied to {target_tag}"),
                Some(serde_json::json!({ "target_tag": target_tag })),
            );
            (StatusCode::OK, Json(result)).into_response()
        }
        Err(err) => {
            let message = err.to_string();
            ui_runtime_api::append_control_panel_log(
                &runtime,
                "error",
                "update-apply",
                format!("Update failed: {message}"),
                None,
            );
            error_response(StatusCode::BAD_REQUEST, message)
        }
    }
}

fn run_update_subcommand(
    runtime: &RuntimeState,
    update_args: &[&str],
    skip_stop: bool,
) -> Result<serde_json::Value> {
    let current_exe =
        std::env::current_exe().context("resolve current agent-ruler executable for update")?;
    let mut command = Command::new(current_exe);
    command
        .arg("--runtime-dir")
        .arg(&runtime.config.runtime_root)
        .arg("update");
    for arg in update_args {
        command.arg(arg);
    }
    if skip_stop {
        command.env("AGENT_RULER_INSTALL_SKIP_STOP", "1");
    }

    let output = command
        .output()
        .context("run child update command for WebUI request")?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        if !stderr.is_empty() {
            return Err(anyhow!(stderr));
        }
        if !stdout.is_empty() {
            return Err(anyhow!(stdout));
        }
        return Err(anyhow!(
            "update command failed with status {}",
            output.status
        ));
    }

    serde_json::from_str::<serde_json::Value>(&stdout)
        .with_context(|| "parse update command JSON output")
}

fn run_ui_command(
    runtime: RuntimeState,
    cmd: Vec<String>,
    summary_line: Option<String>,
) -> Response {
    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let engine = PolicyEngine::new(runtime.policy.clone(), runtime.config.workspace.clone());

    match run_confined(&cmd, &runtime, &engine, &approvals, &receipts) {
        Ok(result) if result.exit_code == 0 => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "completed",
                "exit_code": result.exit_code,
                "confinement": result.confinement,
                "stdout": result.stdout,
                "stderr": result.stderr,
                "summary_line": summary_line,
            })),
        )
            .into_response(),
        Ok(result) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "failed",
                "exit_code": result.exit_code,
                "confinement": result.confinement,
                "stdout": result.stdout,
                "stderr": result.stderr,
                "summary_line": summary_line,
                "error": format!("command exited with code {}", result.exit_code),
            })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "failed",
                "summary_line": summary_line,
                "error": err.to_string(),
            })),
        )
            .into_response(),
    }
}

fn ui_command_summary_line(runtime: &RuntimeState) -> String {
    format!("WebUI runtime: {}", runtime.config.runtime_root.display())
}

fn wrap_one_shot_script(runtime: &RuntimeState, script: &str) -> Result<String> {
    let agent_ruler_exe = resolve_agent_ruler_executable_for_ui()?;
    wrap_one_shot_script_with_exe(runtime, script, &agent_ruler_exe)
}

fn wrap_one_shot_script_with_exe(
    runtime: &RuntimeState,
    script: &str,
    agent_ruler_exe: &Path,
) -> Result<String> {
    let exe = shell_single_quote(agent_ruler_exe.to_string_lossy().as_ref());
    let runtime_dir = shell_single_quote(runtime.config.runtime_root.to_string_lossy().as_ref());
    let ruler_root = shell_single_quote(runtime.config.ruler_root.to_string_lossy().as_ref());
    Ok(format!(
        r#"shopt -s expand_aliases
agent_ruler_managed() {{
  (
    if [ -d {ruler_root} ]; then
      cd {ruler_root}
    fi
    AGENT_RULER_ROOT={ruler_root} AR_DIR={ruler_root} AGENT_RULER_SUPPRESS_UI_AUTOBIND=1 {exe} --runtime-dir {runtime_dir} "$@"
  )
}}
alias agent-ruler=agent_ruler_managed
ui_one_shot_runner_block() {{
  local runner="$1"
  printf 'Use `agent-ruler run -- %s ...` in One-Shot Command so the current WebUI runtime remains authoritative.\n' "$runner" >&2
  return 64
}}
openclaw() {{ ui_one_shot_runner_block openclaw; }}
claude() {{ ui_one_shot_runner_block claude; }}
opencode() {{ ui_one_shot_runner_block opencode; }}

{script}"#
    ))
}

fn resolve_agent_ruler_executable_for_ui() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_agent-ruler") {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    if let Ok(current_exe) = std::env::current_exe() {
        let is_agent_ruler = current_exe
            .file_name()
            .and_then(|value| value.to_str())
            .map(|value| value.starts_with("agent-ruler"))
            .unwrap_or(false);
        if is_agent_ruler {
            return Ok(current_exe);
        }
    }

    resolve_command_path("agent-ruler")
        .ok_or_else(|| anyhow!("resolve `agent-ruler` executable for WebUI one-shot command"))
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(crate) fn load_runtime_from_state(state: &WebState) -> Result<RuntimeState> {
    load_runtime(&state.ruler_root, state.runtime_dir.as_deref())
}

pub(crate) fn error_response(code: StatusCode, message: String) -> Response {
    (code, Json(serde_json::json!({ "error": message }))).into_response()
}

fn parse_mass_delete_threshold(value: f64) -> std::result::Result<usize, String> {
    if !value.is_finite() || value < 1.0 {
        return Err("safeguards_mass_delete_threshold must be >= 1".to_string());
    }
    let floored = value.floor();
    if floored > usize::MAX as f64 {
        return Ok(usize::MAX);
    }
    Ok(floored as usize)
}

fn toggle_payload_requests_elevation_changes(payload: &TogglePayload) -> bool {
    payload.elevation_enabled.is_some()
        || payload.elevation_require_operator_auth.is_some()
        || payload.elevation_use_allowlist.is_some()
        || payload.elevation_allowed_packages.is_some()
        || payload.elevation_denied_packages.is_some()
}

fn toggle_payload_requests_advanced_changes(payload: &TogglePayload) -> bool {
    payload.require_shared_approval.is_some()
        || payload.filesystem_workspace.is_some()
        || payload.filesystem_user_data.is_some()
        || payload.filesystem_shared.is_some()
        || payload.filesystem_secrets.is_some()
        || payload.execution_deny_workspace_exec.is_some()
        || payload.execution_deny_tmp_exec.is_some()
        || payload
            .execution_quarantine_on_download_exec_chain
            .is_some()
        || payload.execution_allowed_exec_prefixes.is_some()
        || payload.persistence_deny_autostart.is_some()
        || payload.persistence_approval_paths.is_some()
        || payload.persistence_deny_paths.is_some()
        || payload.safeguards_mass_delete_threshold.is_some()
}

fn normalize_nonempty_unique(values: Vec<String>) -> Vec<String> {
    let mut uniq = BTreeSet::new();
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            uniq.insert(trimmed.to_string());
        }
    }
    uniq.into_iter().collect()
}
