#![cfg(target_os = "linux")]

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use agent_ruler::approvals::ApprovalStore;
use agent_ruler::config::{init_layout, load_runtime, save_config, RuntimeState, CONFIG_FILE_NAME};
use agent_ruler::helpers::workspace_root_for_runner_id;
use agent_ruler::model::{
    ActionKind, ActionRequest, Decision, ProcessContext, ReasonCode, Receipt, Verdict,
};
use agent_ruler::policy::PolicyEngine;
use agent_ruler::receipts::ReceiptStore;
use agent_ruler::runner::run_confined;
use agent_ruler::runners::{RunnerAssociation, RunnerKind, RunnerMissingState};
use agent_ruler::sessions::{
    SessionChannel, SessionListQuery, SessionRecord, SessionStatus, SessionStore,
};
use agent_ruler::ui::{build_router, WebState};
use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use chrono::Utc;
use serde_json::{json, Value};
use tower::util::ServiceExt;

use common::TestRuntimeDir;

struct UiHarness {
    project: TestRuntimeDir,
    runtime: TestRuntimeDir,
    app: axum::Router,
}

impl UiHarness {
    fn new(label: &str) -> Self {
        let project = TestRuntimeDir::new(&format!("{label}-project"));
        let runtime = TestRuntimeDir::new(&format!("{label}-runtime"));

        init_layout(project.path(), Some(runtime.path()), None, true).expect("init runtime layout");

        let state = WebState {
            ruler_root: project.path().to_path_buf(),
            runtime_dir: Some(runtime.path().to_path_buf()),
        };

        Self {
            project,
            runtime,
            app: build_router(state),
        }
    }

    fn runtime_state(&self) -> RuntimeState {
        load_runtime(self.project.path(), Some(self.runtime.path())).expect("load runtime")
    }
}

fn persist_runner_association(
    runtime: &RuntimeState,
    kind: RunnerKind,
    managed_workspace: PathBuf,
) {
    let managed_home = runtime
        .config
        .runtime_root
        .join("user_data")
        .join("runners")
        .join(kind.id())
        .join("home");
    fs::create_dir_all(&managed_home).expect("create managed home");
    fs::create_dir_all(&managed_workspace).expect("create managed workspace");

    let mut updated = runtime.clone();
    updated.config.runner = Some(RunnerAssociation {
        kind,
        managed_home,
        managed_workspace,
        integrations: Vec::new(),
        missing: RunnerMissingState::default(),
    });
    save_config(
        &updated.config.state_dir.join(CONFIG_FILE_NAME),
        &updated.config,
    )
    .expect("save updated runtime config");
}

fn write_runner_shim(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
    fs::create_dir_all(dir).expect("create shim dir");
    let path = dir.join(name);
    let script = format!("#!/usr/bin/env bash\nset -euo pipefail\n{body}\n");
    fs::write(&path, script).expect("write runner shim");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("chmod");
    }

    path
}

async fn call_json(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let req_body = match body {
        Some(payload) => {
            builder = builder.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&payload).expect("serialize request payload"))
        }
        None => Body::empty(),
    };

    let response = app
        .clone()
        .oneshot(builder.body(req_body).expect("build request"))
        .await
        .expect("dispatch request");

    let status = response.status();
    let body_bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .expect("read response body");
    let parsed = if body_bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body_bytes).expect("parse response json")
    };

    (status, parsed)
}

async fn call_text(app: &axum::Router, method: Method, uri: &str) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("dispatch request");

    let status = response.status();
    let body_bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .expect("read response body");
    let body = String::from_utf8(body_bytes.to_vec()).expect("utf8 response body");
    (status, body)
}

fn make_pending_action(label: &str, path: PathBuf) -> ActionRequest {
    ActionRequest {
        id: format!("action-{label}"),
        timestamp: Utc::now(),
        kind: ActionKind::ExportCommit,
        operation: format!("export-{label}"),
        path: Some(path),
        secondary_path: Some(PathBuf::from("/tmp/source.txt")),
        host: None,
        metadata: BTreeMap::new(),
        process: ProcessContext {
            pid: 4242,
            ppid: Some(1),
            command: "ui-api-test".to_string(),
            process_tree: vec![1, 4242],
        },
    }
}

fn make_pending_action_with_runner(label: &str, path: PathBuf, runner_id: &str) -> ActionRequest {
    let mut action = make_pending_action(label, path);
    action
        .metadata
        .insert("runner_id".to_string(), runner_id.to_string());
    action
}

fn make_receipt_with_runner(id: &str, runner_id: &str) -> Receipt {
    let mut action = make_pending_action(id, PathBuf::from(format!("/tmp/{id}.txt")));
    action
        .metadata
        .insert("runner_id".to_string(), runner_id.to_string());

    Receipt {
        id: format!("receipt-{id}"),
        timestamp: Utc::now(),
        action,
        decision: Decision {
            verdict: Verdict::Allow,
            reason: ReasonCode::AllowedByPolicy,
            detail: format!("receipt {id}"),
            approval_ttl_seconds: None,
        },
        zone: None,
        policy_version: "1".to_string(),
        policy_hash: "test-hash".to_string(),
        diff_summary: None,
        confinement: "test".to_string(),
    }
}

fn approval_decision() -> Decision {
    Decision {
        verdict: Verdict::RequireApproval,
        reason: ReasonCode::ApprovalRequiredExport,
        detail: "requires approval".to_string(),
        approval_ttl_seconds: Some(1800),
    }
}

fn is_confinement_env_error(error: &str) -> bool {
    error.contains("Operation not permitted")
        || error.contains("Failed RTM_NEWADDR")
        || error.contains("setting up uid map")
        || error.contains("uid map")
        || error.contains("bubblewrap")
        || error.contains("setns")
}

#[tokio::test]
async fn index_page_contains_primary_navigation_sections() {
    let harness = UiHarness::new("ui-index-page");
    let (status, body) = call_text(&harness.app, Method::GET, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Agent Ruler"));
    assert!(body.contains("Overview"));
    assert!(body.contains("Approvals"));
    assert!(body.contains("Import / Export"));
    assert!(body.contains("Timeline")); // Updated for new UI design
    assert!(body.contains("Runners"));
    assert!(body.contains("Documentation")); // Updated for new UI design
}

#[tokio::test]
async fn ui_js_esc_function_uses_safe_html_entities() {
    let harness = UiHarness::new("ui-esc-entities");
    let (status, js) = call_text(&harness.app, Method::GET, "/assets/ui.js").await;

    assert_eq!(status, StatusCode::OK);
    assert!(js.contains(".replace(/&/g, '&amp;')"));
    assert!(js.contains(".replace(/</g, '&lt;')"));
    assert!(js.contains(".replace(/>/g, '&gt;')"));
    assert!(js.contains(".replace(/\"/g, '&quot;')"));
    assert!(js.contains(".replace(/'/g, '&#39;')"));
}

#[tokio::test]
async fn ui_receipts_filter_does_not_default_to_today() {
    let harness = UiHarness::new("ui-receipts-filter-default");
    let (status, js) = call_text(&harness.app, Method::GET, "/assets/ui.js").await;

    assert_eq!(status, StatusCode::OK);
    assert!(js.contains("date: ''"));
    assert!(js.contains("state.receipts.filters.date = '';"));
}

#[tokio::test]
async fn ui_global_runtime_path_toggle_wiring_is_present() {
    let harness = UiHarness::new("ui-global-runtime-path-toggle");
    let (status, js) = call_text(&harness.app, Method::GET, "/assets/ui.js").await;

    assert_eq!(status, StatusCode::OK);
    assert!(js.contains("settings-runtime-path-labels"));
    assert!(js.contains("setRuntimeAliasVisibility("));
    assert!(
        !js.contains("filter-runtime-aliases"),
        "timeline-local runtime alias toggle should not exist"
    );
    assert!(
        js.contains("workspacePathEl.textContent = aliasRuntimePath"),
        "files page should render path hints through runtime alias helper"
    );
    assert!(
        js.contains("Zone 0 (workspace):</span> <span class=\"mono\">${esc(aliasRuntimePath("),
        "runners page zone visibility should render through runtime alias helper"
    );
}

#[tokio::test]
async fn status_runtime_and_policy_toggles_work() {
    let harness = UiHarness::new("ui-status-policy");

    let (status_code, status) = call_json(&harness.app, Method::GET, "/api/status", None).await;
    assert_eq!(status_code, StatusCode::OK);
    assert_eq!(status["pending_approvals"], 0);
    assert!(status["workspace"]
        .as_str()
        .unwrap_or_default()
        .contains("workspace"));
    assert!(status["runtime_root"]
        .as_str()
        .unwrap_or_default()
        .contains("agent-ruler"));
    assert_eq!(status["policy_version"], "1");
    assert!(status["policy_hash"].as_str().unwrap_or_default().len() >= 10);
    assert_eq!(status["ui_show_debug_tools"], false);
    assert_eq!(status["approval_wait_timeout_secs"], 90);

    let (runtime_code, runtime) = call_json(&harness.app, Method::GET, "/api/runtime", None).await;
    assert_eq!(runtime_code, StatusCode::OK);
    assert!(runtime["state_dir"]
        .as_str()
        .unwrap_or_default()
        .contains("state"));
    assert!(runtime["approvals_file"]
        .as_str()
        .unwrap_or_default()
        .ends_with("approvals.json"));
    assert_eq!(runtime["approval_wait_timeout_secs"], 90);

    let (receipts_code, receipts_payload) =
        call_json(&harness.app, Method::GET, "/api/receipts?limit=10", None).await;
    assert_eq!(receipts_code, StatusCode::OK);
    assert!(receipts_payload["items"].is_array());
    assert_eq!(receipts_payload["limit"], 10);

    let (_, policy_before) = call_json(&harness.app, Method::GET, "/api/policy", None).await;
    assert_eq!(policy_before["profile"], "strict");
    assert_eq!(policy_before["rules"]["network"]["default_deny"], true);
    assert_eq!(policy_before["rules"]["filesystem"]["shared"], "approval");

    let (profiles_code, profiles) =
        call_json(&harness.app, Method::GET, "/api/policy/profiles", None).await;
    assert_eq!(profiles_code, StatusCode::OK);
    assert!(profiles
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|item| item["id"] == "strict"));
    assert!(profiles
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|item| item["id"] == "simple_user"));
    assert!(profiles
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|item| item["id"] == "balanced"));
    assert!(profiles
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|item| item["id"] == "coding_nerd"));
    assert!(profiles
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|item| item["id"] == "i_dont_care"));
    assert!(profiles
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|item| item["id"] == "custom"));
    let strict_profile = profiles
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == "strict"))
        .expect("strict profile exists");
    let simple_profile = profiles
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == "simple_user"))
        .expect("simple_user profile exists");
    let balanced_profile = profiles
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == "balanced"))
        .expect("balanced profile exists");
    let coding_profile = profiles
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == "coding_nerd"))
        .expect("coding_nerd profile exists");
    let idc_profile = profiles
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == "i_dont_care"))
        .expect("i_dont_care profile exists");
    let custom_profile = profiles
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == "custom"))
        .expect("custom profile exists");

    assert_eq!(strict_profile["allow_rule_customization"], false);
    assert_eq!(simple_profile["allow_rule_customization"], false);
    assert_eq!(balanced_profile["allow_rule_customization"], false);
    assert_eq!(coding_profile["allow_rule_customization"], true);
    assert_eq!(idc_profile["allow_rule_customization"], true);
    assert_eq!(custom_profile["allow_rule_customization"], true);
    assert_eq!(strict_profile["allow_elevation_customization"], false);
    assert_eq!(simple_profile["allow_elevation_customization"], false);
    assert_eq!(balanced_profile["allow_elevation_customization"], true);
    assert_eq!(coding_profile["allow_elevation_customization"], true);
    assert_eq!(custom_profile["can_create_custom_profile"], false);
    assert_ne!(
        strict_profile["description"].as_str().unwrap_or_default(),
        balanced_profile["description"].as_str().unwrap_or_default()
    );
    assert_ne!(
        strict_profile["description"].as_str().unwrap_or_default(),
        idc_profile["description"].as_str().unwrap_or_default()
    );

    let (balanced_update_code, balanced_update_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/policy/toggles",
        Some(json!({
            "profile": "balanced"
        })),
    )
    .await;
    assert_eq!(balanced_update_code, StatusCode::OK);
    assert_eq!(balanced_update_payload["profile"], "balanced");

    let (advanced_locked_code, advanced_locked_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/policy/toggles",
        Some(json!({
            "filesystem_shared": "allow"
        })),
    )
    .await;
    assert_eq!(advanced_locked_code, StatusCode::BAD_REQUEST);
    assert!(advanced_locked_payload["error"]
        .as_str()
        .unwrap_or_default()
        .contains("locks advanced"));

    let (network_toggle_code, network_toggle_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/policy/toggles",
        Some(json!({
            "network_default_deny": true,
            "network_denylist_hosts": ["example.com", "example.com", ""]
        })),
    )
    .await;
    assert_eq!(network_toggle_code, StatusCode::OK);
    assert_eq!(
        network_toggle_payload["rules"]["network"]["default_deny"],
        true
    );
    assert_eq!(
        network_toggle_payload["rules"]["network"]["denylist_hosts"],
        json!(["example.com"])
    );

    let (create_custom_code, create_custom_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/policy/toggles",
        Some(json!({
            "create_custom_profile": true
        })),
    )
    .await;
    assert_eq!(create_custom_code, StatusCode::OK);
    assert_eq!(create_custom_payload["profile"], "custom");

    let (custom_rules_code, custom_rules_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/policy/toggles",
        Some(json!({
            "filesystem_shared": "allow",
            "filesystem_secrets": "allow",
            "execution_quarantine_on_download_exec_chain": false,
            "safeguards_mass_delete_threshold": 1
        })),
    )
    .await;
    assert_eq!(custom_rules_code, StatusCode::OK);
    assert_eq!(
        custom_rules_payload["rules"]["filesystem"]["system_critical"],
        "deny"
    );
    assert_eq!(
        custom_rules_payload["rules"]["filesystem"]["secrets"],
        "deny"
    );
    assert_eq!(
        custom_rules_payload["rules"]["execution"]["quarantine_on_download_exec_chain"],
        true
    );
    assert!(
        custom_rules_payload["safeguards"]["mass_delete_threshold"]
            .as_u64()
            .unwrap_or_default()
            >= 20
    );

    let (_, policy_after) = call_json(&harness.app, Method::GET, "/api/policy", None).await;
    assert_eq!(policy_after["profile"], "custom");
    assert_eq!(policy_after["rules"]["network"]["default_deny"], true);
    assert_eq!(policy_after["rules"]["filesystem"]["shared"], "allow");

    let (presets_code, presets_payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/policy/domain-presets",
        None,
    )
    .await;
    assert_eq!(presets_code, StatusCode::OK);
    assert!(presets_payload["safe_defaults"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|item| item == "api.github.com"));
    assert!(presets_payload["post_allowlist_defaults"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|item| item == "api.openai.com"));
    assert!(presets_payload["get_allowlist_defaults"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|item| item == "github.com"));
    assert!(presets_payload["allowlisted_packages"]["apt"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|item| item == "git"));
    assert!(presets_payload["denylisted_packages"]["pip"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|item| item == "mitmproxy"));
}

#[tokio::test]
async fn runners_introspection_endpoints_return_stable_shapes() {
    let harness = UiHarness::new("ui-runners-introspection");

    let (list_status, list_payload) =
        call_json(&harness.app, Method::GET, "/api/runners", None).await;
    assert_eq!(list_status, StatusCode::OK);
    let items = list_payload["items"].as_array().expect("items array");
    assert_eq!(items.len(), 3, "expected openclaw, claudecode, opencode");

    let ids: std::collections::HashSet<&str> = items
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert!(ids.contains("openclaw"));
    assert!(ids.contains("claudecode"));
    assert!(ids.contains("opencode"));

    let (item_status, item_payload) =
        call_json(&harness.app, Method::GET, "/api/runners/openclaw", None).await;
    assert_eq!(item_status, StatusCode::OK);
    assert_eq!(item_payload["id"], "openclaw");
    assert!(item_payload["binary"].is_object());
    assert!(item_payload["health"].is_object());
    assert!(item_payload["mode"].is_object());
    assert!(item_payload["capabilities"].is_array());
    assert!(item_payload["warnings"].is_array());
    assert!(item_payload["config"].is_object());

    let (claude_status, claude_payload) =
        call_json(&harness.app, Method::GET, "/api/runners/claudecode", None).await;
    assert_eq!(claude_status, StatusCode::OK);
    assert_eq!(claude_payload["id"], "claudecode");
    assert_eq!(claude_payload["label"], "Claude Code");
    assert!(claude_payload["mode"]["supported"].is_array());
    assert!(claude_payload["config"]["masked"].is_object());

    let (opencode_status, opencode_payload) =
        call_json(&harness.app, Method::GET, "/api/runners/opencode", None).await;
    assert_eq!(opencode_status, StatusCode::OK);
    assert_eq!(opencode_payload["id"], "opencode");
    assert_eq!(opencode_payload["label"], "OpenCode");
    assert!(opencode_payload["mode"]["supported"].is_array());
    assert!(opencode_payload["config"]["masked"].is_object());

    let (missing_status, missing_payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/runners/unknown-runner",
        None,
    )
    .await;
    assert_eq!(missing_status, StatusCode::NOT_FOUND);
    assert!(missing_payload["error"]
        .as_str()
        .unwrap_or_default()
        .contains("not found"));
}

#[tokio::test]
async fn runners_introspection_cache_and_force_flags_work() {
    let harness = UiHarness::new("ui-runners-cache");

    let (first_status, first_payload) =
        call_json(&harness.app, Method::GET, "/api/runners", None).await;
    assert_eq!(first_status, StatusCode::OK);
    assert_eq!(
        first_payload["cached"].as_bool(),
        Some(false),
        "first read should materialize fresh metadata"
    );
    assert!(first_payload["items"].is_array());

    let (second_status, second_payload) =
        call_json(&harness.app, Method::GET, "/api/runners", None).await;
    assert_eq!(second_status, StatusCode::OK);
    assert_eq!(
        second_payload["cached"].as_bool(),
        Some(true),
        "repeat read should serve from cache"
    );

    let (force_status, force_payload) =
        call_json(&harness.app, Method::GET, "/api/runners?force=true", None).await;
    assert_eq!(force_status, StatusCode::OK);
    assert_eq!(
        force_payload["cached"].as_bool(),
        Some(false),
        "force refresh should bypass cache"
    );
}

#[tokio::test]
async fn sessions_api_filters_paginates_and_returns_detail() {
    let harness = UiHarness::new("ui-sessions-api");
    let runtime = harness.runtime_state();
    let sessions_path = SessionStore::default_path(&runtime.config.state_dir);
    let now = Utc::now();
    let seeded = vec![
        SessionRecord {
            id: "session-claude-telegram-a".to_string(),
            runner_kind: RunnerKind::Claudecode,
            created_at: now,
            last_active_at: now,
            status: SessionStatus::Active,
            title: Some("Claude Telegram A".to_string()),
            label: None,
            runner_session_key: Some("claude-a".to_string()),
            channels: vec![SessionChannel::Telegram],
            telegram_chat_id: Some("-1001".to_string()),
            telegram_thread_id: Some(10),
            telegram_message_anchor_id: Some(100),
        },
        SessionRecord {
            id: "session-claude-telegram-b".to_string(),
            runner_kind: RunnerKind::Claudecode,
            created_at: now - chrono::Duration::minutes(5),
            last_active_at: now - chrono::Duration::minutes(5),
            status: SessionStatus::Active,
            title: Some("Claude Telegram B".to_string()),
            label: None,
            runner_session_key: Some("claude-b".to_string()),
            channels: vec![SessionChannel::Telegram],
            telegram_chat_id: Some("-1001".to_string()),
            telegram_thread_id: Some(11),
            telegram_message_anchor_id: Some(101),
        },
        SessionRecord {
            id: "session-opencode-tui".to_string(),
            runner_kind: RunnerKind::Opencode,
            created_at: now - chrono::Duration::days(10),
            last_active_at: now - chrono::Duration::days(10),
            status: SessionStatus::Archived,
            title: None,
            label: Some("Agent main".to_string()),
            runner_session_key: Some("opencode-z".to_string()),
            channels: vec![SessionChannel::Tui],
            telegram_chat_id: None,
            telegram_thread_id: None,
            telegram_message_anchor_id: None,
        },
    ];
    fs::write(
        &sessions_path,
        serde_json::to_string_pretty(&seeded).expect("serialize sessions fixtures"),
    )
    .expect("write sessions fixtures");

    let (page_status, page_payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/sessions?runner=claudecode&channel=telegram&status=active&activity=recent&limit=1&cursor=0&q=Claude",
        None,
    )
    .await;
    assert_eq!(page_status, StatusCode::OK);
    assert_eq!(page_payload["total"], 2);
    assert_eq!(page_payload["items"].as_array().map(|v| v.len()), Some(1));
    assert_eq!(page_payload["has_more"], true);
    assert_eq!(page_payload["next_cursor"], "1");
    assert_eq!(page_payload["items"][0]["runner_label"], "Claude Code");
    assert_eq!(page_payload["items"][0]["channels"][0], "telegram");

    let detail_id = page_payload["items"][0]["id"]
        .as_str()
        .expect("session id in first page");
    let (detail_status, detail_payload) = call_json(
        &harness.app,
        Method::GET,
        &format!("/api/sessions/{detail_id}"),
        None,
    )
    .await;
    assert_eq!(detail_status, StatusCode::OK);
    assert_eq!(detail_payload["id"], detail_id);
    assert_eq!(detail_payload["telegram_thread_id"], 10);

    let store = SessionStore::new(sessions_path);
    let direct_page = store
        .page(&SessionListQuery::default())
        .expect("list sessions directly");
    assert_eq!(direct_page.total, 3);
}

#[tokio::test]
async fn telegram_session_resolve_endpoint_creates_reuses_and_blocks_runner_switch() {
    let harness = UiHarness::new("ui-telegram-session-resolve");

    let (create_status, create_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/sessions/telegram/resolve",
        Some(json!({
            "runner_kind": "claudecode",
            "chat_id": "-1004242",
            "thread_id": 77,
            "message_anchor_id": 501,
            "title": "Daily notes"
        })),
    )
    .await;
    assert_eq!(create_status, StatusCode::OK);
    assert_eq!(create_payload["created"], true);
    assert_eq!(create_payload["session"]["runner_kind"], "claudecode");
    assert_eq!(create_payload["session"]["runner_label"], "Claude Code");
    assert_eq!(create_payload["session"]["telegram_thread_id"], 77);

    let session_id = create_payload["session"]["id"]
        .as_str()
        .expect("created session id")
        .to_string();

    let (reuse_status, reuse_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/sessions/telegram/resolve",
        Some(json!({
            "runner_kind": "claudecode",
            "chat_id": "-1004242",
            "thread_id": 77,
            "message_anchor_id": 999,
            "title": "Replacement title"
        })),
    )
    .await;
    assert_eq!(reuse_status, StatusCode::OK);
    assert_eq!(reuse_payload["created"], false);
    assert_eq!(reuse_payload["session"]["id"], session_id);
    assert_eq!(reuse_payload["session"]["telegram_message_anchor_id"], 501);

    let (conflict_status, conflict_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/sessions/telegram/resolve",
        Some(json!({
            "runner_kind": "opencode",
            "chat_id": "-1004242",
            "thread_id": 77
        })),
    )
    .await;
    assert_eq!(conflict_status, StatusCode::CONFLICT);
    assert!(conflict_payload["error"]
        .as_str()
        .unwrap_or_default()
        .contains("already bound to runner `claudecode`"));
}

#[tokio::test]
async fn runtime_and_file_listing_apis_are_runner_aware_for_workspace_zone() {
    let harness = UiHarness::new("ui-runtime-runner-zones");
    let runtime = harness.runtime_state();

    let openclaw_workspace = runtime.config.workspace.clone();
    let opencode_workspace = runtime
        .config
        .runtime_root
        .join("user_data")
        .join("runners")
        .join("opencode")
        .join("workspace");
    let claudecode_workspace = runtime
        .config
        .runtime_root
        .join("user_data")
        .join("runners")
        .join("claudecode")
        .join("workspace");

    fs::create_dir_all(&openclaw_workspace).expect("create openclaw workspace");
    fs::create_dir_all(&opencode_workspace).expect("create opencode workspace");
    fs::create_dir_all(&claudecode_workspace).expect("create claudecode workspace");
    fs::write(openclaw_workspace.join("openclaw-zone.txt"), "openclaw")
        .expect("write openclaw marker");
    fs::write(opencode_workspace.join("opencode-zone.txt"), "opencode")
        .expect("write opencode marker");
    fs::write(
        claudecode_workspace.join("claudecode-zone.txt"),
        "claudecode",
    )
    .expect("write claudecode marker");

    persist_runner_association(&runtime, RunnerKind::Openclaw, openclaw_workspace.clone());

    let (runtime_default_status, runtime_default_payload) =
        call_json(&harness.app, Method::GET, "/api/runtime", None).await;
    assert_eq!(runtime_default_status, StatusCode::OK);
    assert_eq!(
        runtime_default_payload["selected_runner"],
        RunnerKind::Openclaw.id()
    );
    assert_eq!(
        runtime_default_payload["workspace"].as_str(),
        Some(openclaw_workspace.to_string_lossy().as_ref())
    );

    let (runtime_opencode_status, runtime_opencode_payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/runtime?runner=opencode",
        None,
    )
    .await;
    assert_eq!(runtime_opencode_status, StatusCode::OK);
    assert_eq!(
        runtime_opencode_payload["selected_runner"],
        RunnerKind::Opencode.id()
    );
    assert_eq!(
        runtime_opencode_payload["workspace"].as_str(),
        Some(opencode_workspace.to_string_lossy().as_ref())
    );

    let (runtime_claude_status, runtime_claude_payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/runtime?runner=claudecode",
        None,
    )
    .await;
    assert_eq!(runtime_claude_status, StatusCode::OK);
    assert_eq!(
        runtime_claude_payload["selected_runner"],
        RunnerKind::Claudecode.id()
    );
    assert_eq!(
        runtime_claude_payload["workspace"].as_str(),
        Some(claudecode_workspace.to_string_lossy().as_ref())
    );

    let (default_files_status, default_files_payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/files/list?zone=workspace",
        None,
    )
    .await;
    assert_eq!(default_files_status, StatusCode::OK);
    assert!(default_files_payload
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|item| item["path"] == "openclaw-zone.txt"));
    assert!(!default_files_payload
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|item| item["path"] == "opencode-zone.txt"));

    let (opencode_files_status, opencode_files_payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/files/list?zone=workspace&runner=opencode",
        None,
    )
    .await;
    assert_eq!(opencode_files_status, StatusCode::OK);
    assert!(opencode_files_payload
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|item| item["path"] == "opencode-zone.txt"));
    assert!(!opencode_files_payload
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|item| item["path"] == "openclaw-zone.txt"));

    let (claude_files_status, claude_files_payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/files/list?zone=workspace&runner=claudecode",
        None,
    )
    .await;
    assert_eq!(claude_files_status, StatusCode::OK);
    assert!(claude_files_payload
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|item| item["path"] == "claudecode-zone.txt"));

    let (invalid_runner_status, invalid_runner_payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/files/list?zone=workspace&runner=unknown",
        None,
    )
    .await;
    assert_eq!(invalid_runner_status, StatusCode::BAD_REQUEST);
    assert!(invalid_runner_payload["error"]
        .as_str()
        .unwrap_or_default()
        .contains("runner must be one of"));
}

#[tokio::test]
async fn approvals_support_runner_filter() {
    let harness = UiHarness::new("ui-approvals-runner-filter");
    let runtime = harness.runtime_state();
    let approvals = ApprovalStore::new(&runtime.config.approvals_file);

    let _ = approvals
        .create_pending(
            &make_pending_action_with_runner(
                "openclaw",
                runtime.config.shared_zone_dir.join("openclaw.txt"),
                "openclaw",
            ),
            &approval_decision(),
            "openclaw approval",
        )
        .expect("create openclaw approval");
    let _ = approvals
        .create_pending(
            &make_pending_action_with_runner(
                "claudecode",
                runtime.config.shared_zone_dir.join("claudecode.txt"),
                "claudecode",
            ),
            &approval_decision(),
            "claudecode approval",
        )
        .expect("create claudecode approval");

    let (status, payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/approvals?runner=claudecode",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = payload.as_array().expect("approvals array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["action"]["metadata"]["runner_id"], "claudecode");
}

#[tokio::test]
async fn receipts_support_runner_filter() {
    let harness = UiHarness::new("ui-receipts-runner-filter");
    let runtime = harness.runtime_state();
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);

    receipts
        .append(&make_receipt_with_runner("openclaw-1", "openclaw"))
        .expect("append openclaw receipt");
    receipts
        .append(&make_receipt_with_runner("opencode-1", "opencode"))
        .expect("append opencode receipt");

    let (status, payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/receipts?runner=opencode&limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["total"], 1);
    assert_eq!(
        payload["items"][0]["action"]["metadata"]["runner_id"],
        "opencode"
    );
}

#[tokio::test]
async fn ui_logs_support_runner_filter() {
    let harness = UiHarness::new("ui-logs-runner-filter");

    let _ = call_json(
        &harness.app,
        Method::POST,
        "/api/ui/logs/event",
        Some(json!({
            "level": "info",
            "source": "runner",
            "message": "claude event",
            "details": {"runner_id": "claudecode"}
        })),
    )
    .await;
    let _ = call_json(
        &harness.app,
        Method::POST,
        "/api/ui/logs/event",
        Some(json!({
            "level": "info",
            "source": "runner",
            "message": "openclaw event",
            "details": {"runner_id": "openclaw"}
        })),
    )
    .await;

    let (status, payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/ui/logs?runner=claudecode&limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["total"], 1);
    assert_eq!(payload["items"][0]["details"]["runner_id"], "claudecode");
}

#[tokio::test]
async fn ui_logs_endpoint_appends_and_filters_entries() {
    let harness = UiHarness::new("ui-logs-endpoint");

    let (append_code, append_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/ui/logs/event",
        Some(json!({
            "level": "warning",
            "source": "update-check",
            "message": "Update check failed: test",
            "details": {
                "code": "gh_api_22"
            }
        })),
    )
    .await;
    assert_eq!(append_code, StatusCode::OK);
    assert_eq!(append_payload["status"], "logged");

    let (list_code, list_payload) =
        call_json(&harness.app, Method::GET, "/api/ui/logs?limit=10", None).await;
    assert_eq!(list_code, StatusCode::OK);
    assert!(list_payload["items"].is_array());
    assert_eq!(list_payload["limit"], 10);
    assert!(list_payload["total"].as_u64().unwrap_or_default() >= 1);
    assert_eq!(list_payload["items"][0]["source"], "update-check");

    let (filtered_code, filtered_payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/ui/logs?source=update-check&level=warning&limit=10",
        None,
    )
    .await;
    assert_eq!(filtered_code, StatusCode::OK);
    assert!(filtered_payload["items"].is_array());
    assert!(filtered_payload["total"].as_u64().unwrap_or_default() >= 1);
}

#[tokio::test]
async fn approvals_endpoints_handle_single_and_bulk_actions() {
    let harness = UiHarness::new("ui-approvals");
    let runtime = harness.runtime_state();
    let approvals = ApprovalStore::new(&runtime.config.approvals_file);

    let first = approvals
        .create_pending(
            &make_pending_action("first", runtime.config.shared_zone_dir.join("one.txt")),
            &approval_decision(),
            "first pending",
        )
        .expect("create first approval");
    let _second = approvals
        .create_pending(
            &make_pending_action("second", runtime.config.shared_zone_dir.join("two.txt")),
            &approval_decision(),
            "second pending",
        )
        .expect("create second approval");

    let (_, pending_before) = call_json(&harness.app, Method::GET, "/api/approvals", None).await;
    assert_eq!(pending_before.as_array().unwrap_or(&Vec::new()).len(), 2);
    let first_pending = pending_before
        .as_array()
        .and_then(|items| items.first())
        .expect("first pending approval");
    assert!(first_pending["why"]
        .as_str()
        .unwrap_or_default()
        .contains("first pending"));

    let approve_path = format!("/api/approvals/{}/approve", first.id);
    let (approve_code, approve_payload) =
        call_json(&harness.app, Method::POST, &approve_path, None).await;
    assert_eq!(approve_code, StatusCode::OK);
    assert_eq!(approve_payload["status"], "approved");

    // Repeated resolution should be idempotent (no 400 for already-approved).
    let (approve_again_code, approve_again_payload) =
        call_json(&harness.app, Method::POST, &approve_path, None).await;
    assert_eq!(approve_again_code, StatusCode::OK);
    assert_eq!(approve_again_payload["status"], "approved");

    let (_, pending_after_single) =
        call_json(&harness.app, Method::GET, "/api/approvals", None).await;
    assert_eq!(
        pending_after_single.as_array().unwrap_or(&Vec::new()).len(),
        1
    );

    let (deny_all_code, deny_all_payload) =
        call_json(&harness.app, Method::POST, "/api/approvals/deny-all", None).await;
    assert_eq!(deny_all_code, StatusCode::OK);
    assert_eq!(
        deny_all_payload["updated"]
            .as_array()
            .unwrap_or(&Vec::new())
            .len(),
        1
    );

    let (_, pending_after_deny_all) =
        call_json(&harness.app, Method::GET, "/api/approvals", None).await;
    assert_eq!(
        pending_after_deny_all
            .as_array()
            .unwrap_or(&Vec::new())
            .len(),
        0
    );

    approvals
        .create_pending(
            &make_pending_action("third", runtime.config.shared_zone_dir.join("three.txt")),
            &approval_decision(),
            "third pending",
        )
        .expect("create third approval");
    approvals
        .create_pending(
            &make_pending_action("fourth", runtime.config.shared_zone_dir.join("four.txt")),
            &approval_decision(),
            "fourth pending",
        )
        .expect("create fourth approval");

    let (approve_all_code, approve_all_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/approvals/approve-all",
        None,
    )
    .await;
    assert_eq!(approve_all_code, StatusCode::OK);
    assert_eq!(
        approve_all_payload["updated"]
            .as_array()
            .unwrap_or(&Vec::new())
            .len(),
        2
    );
    assert_eq!(
        approve_all_payload["failed"]
            .as_array()
            .unwrap_or(&Vec::new())
            .len(),
        0
    );

    let (_, pending_after_approve_all) =
        call_json(&harness.app, Method::GET, "/api/approvals", None).await;
    assert_eq!(
        pending_after_approve_all
            .as_array()
            .unwrap_or(&Vec::new())
            .len(),
        0
    );
}

#[tokio::test]
async fn approval_wait_endpoint_reports_timeout_then_resolution() {
    let harness = UiHarness::new("ui-approval-wait");
    let runtime = harness.runtime_state();
    let approvals = ApprovalStore::new(&runtime.config.approvals_file);

    let pending = approvals
        .create_pending(
            &make_pending_action("wait", runtime.config.shared_zone_dir.join("wait.txt")),
            &approval_decision(),
            "wait pending",
        )
        .expect("create pending approval");

    let wait_uri = format!("/api/approvals/{}/wait?timeout=1&poll_ms=100", pending.id);
    let (timeout_code, timeout_payload) =
        call_json(&harness.app, Method::GET, &wait_uri, None).await;
    assert_eq!(timeout_code, StatusCode::OK);
    assert_eq!(timeout_payload["approval_id"], pending.id);
    assert_eq!(timeout_payload["verdict"], "pending");
    assert_eq!(timeout_payload["reason_code"], "approval_required_export");
    assert_eq!(timeout_payload["resolved"], false);
    assert_eq!(timeout_payload["timeout"], true);
    assert!(timeout_payload["open_in_webui"]
        .as_str()
        .unwrap_or_default()
        .ends_with(&pending.id));

    let approve_path = format!("/api/approvals/{}/approve", pending.id);
    let (approve_code, _) = call_json(&harness.app, Method::POST, &approve_path, None).await;
    assert_eq!(approve_code, StatusCode::OK);

    let (resolved_code, resolved_payload) =
        call_json(&harness.app, Method::GET, &wait_uri, None).await;
    assert_eq!(resolved_code, StatusCode::OK);
    assert_eq!(resolved_payload["approval_id"], pending.id);
    assert_eq!(resolved_payload["verdict"], "approved");
    assert_eq!(resolved_payload["resolved"], true);
    assert_eq!(resolved_payload["timeout"], false);
}

#[tokio::test]
async fn approval_wait_endpoint_uses_runtime_default_timeout_setting() {
    let harness = UiHarness::new("ui-approval-wait-default-timeout");
    let runtime = harness.runtime_state();
    let approvals = ApprovalStore::new(&runtime.config.approvals_file);

    let (update_code, update_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/config/update",
        Some(json!({
            "approval_wait_timeout_secs": 1
        })),
    )
    .await;
    assert_eq!(update_code, StatusCode::OK);
    assert_eq!(update_payload["config"]["approval_wait_timeout_secs"], 1);

    let pending = approvals
        .create_pending(
            &make_pending_action(
                "wait-default",
                runtime.config.shared_zone_dir.join("wait-default.txt"),
            ),
            &approval_decision(),
            "wait pending default timeout",
        )
        .expect("create pending approval");

    let wait_uri = format!("/api/approvals/{}/wait?poll_ms=100", pending.id);
    let (timeout_code, timeout_payload) =
        call_json(&harness.app, Method::GET, &wait_uri, None).await;
    assert_eq!(timeout_code, StatusCode::OK);
    assert_eq!(timeout_payload["approval_id"], pending.id);
    assert_eq!(timeout_payload["verdict"], "pending");
    assert_eq!(timeout_payload["resolved"], false);
    assert_eq!(timeout_payload["timeout"], true);
}

#[tokio::test]
async fn config_update_rejects_invalid_approval_wait_timeout() {
    let harness = UiHarness::new("ui-config-invalid-wait-timeout");

    let (status, payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/config/update",
        Some(json!({
            "approval_wait_timeout_secs": 0
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(payload["error"]
        .as_str()
        .unwrap_or_default()
        .contains("approval_wait_timeout_secs"));
}

#[tokio::test]
async fn config_get_includes_generated_openclaw_bridge_settings() {
    let harness = UiHarness::new("ui-config-openclaw-bridge-get");

    let (status, payload) = call_json(&harness.app, Method::GET, "/api/config", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        payload["openclaw_bridge"]["config"]["poll_interval_seconds"],
        8
    );
    assert_eq!(
        payload["openclaw_bridge"]["config"]["decision_ttl_seconds"],
        7200
    );
    assert_eq!(payload["openclaw_bridge"]["config"]["short_id_length"], 6);
    assert_eq!(
        payload["openclaw_bridge"]["config"]["inbound_bind"],
        "127.0.0.1:4661"
    );
    assert_eq!(
        payload["openclaw_bridge"]["config"]["openclaw_bin"],
        "openclaw"
    );
    assert_eq!(
        payload["openclaw_bridge"]["config"]["agent_ruler_bin"],
        "agent-ruler"
    );
    assert!(payload["openclaw_bridge"]["config"]["ruler_url"]
        .as_str()
        .unwrap_or_default()
        .starts_with("http://"));

    let config_path = PathBuf::from(
        payload["openclaw_bridge"]["config_path"]
            .as_str()
            .expect("bridge config path"),
    );
    assert!(config_path.exists(), "generated bridge config should exist");

    for key in ["claudecode_bridge", "opencode_bridge"] {
        assert_eq!(payload[key]["config"]["enabled"], false);
        assert_eq!(payload[key]["config"]["answer_streaming_enabled"], true);
        assert_eq!(payload[key]["config"]["poll_interval_seconds"], 8);
        assert_eq!(payload[key]["config"]["decision_ttl_seconds"], 7200);
        assert_eq!(payload[key]["config"]["short_id_length"], 6);
        assert_eq!(payload[key]["config"]["bot_token_configured"], false);
        assert_eq!(payload[key]["config"]["bot_token_masked"], "");
        let bridge_path = PathBuf::from(
            payload[key]["config_path"]
                .as_str()
                .expect("runner bridge config path"),
        );
        assert!(
            bridge_path.exists(),
            "generated runner bridge config should exist for {key}"
        );
    }
}

#[tokio::test]
async fn config_update_persists_openclaw_bridge_settings() {
    let harness = UiHarness::new("ui-config-openclaw-bridge-update");

    let (status, payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/config/update",
        Some(json!({
            "openclaw_bridge": {
                "poll_interval_seconds": 12,
                "decision_ttl_seconds": 1800,
                "short_id_length": 8,
                "inbound_bind": "127.0.0.1:4777",
                "state_file": "custom-state.json",
                "openclaw_bin": "openclaw-custom",
                "agent_ruler_bin": "agent-ruler-custom"
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        payload["openclaw_bridge"]["config"]["poll_interval_seconds"],
        12
    );
    assert_eq!(
        payload["openclaw_bridge"]["config"]["decision_ttl_seconds"],
        1800
    );
    assert_eq!(payload["openclaw_bridge"]["config"]["short_id_length"], 8);
    assert_eq!(
        payload["openclaw_bridge"]["config"]["inbound_bind"],
        "127.0.0.1:4777"
    );
    assert_eq!(
        payload["openclaw_bridge"]["config"]["state_file"],
        "custom-state.json"
    );
    assert_eq!(
        payload["openclaw_bridge"]["config"]["openclaw_bin"],
        "openclaw-custom"
    );
    assert_eq!(
        payload["openclaw_bridge"]["config"]["agent_ruler_bin"],
        "agent-ruler-custom"
    );

    let config_path = PathBuf::from(
        payload["openclaw_bridge"]["config_path"]
            .as_str()
            .expect("bridge config path"),
    );
    let raw = fs::read_to_string(&config_path).expect("read generated bridge config");
    let parsed: Value = serde_json::from_str(&raw).expect("parse generated bridge config");
    assert_eq!(parsed["poll_interval_seconds"], 12);
    assert_eq!(parsed["decision_ttl_seconds"], 1800);
    assert_eq!(parsed["short_id_length"], 8);
    assert_eq!(parsed["inbound_bind"], "127.0.0.1:4777");
    assert_eq!(parsed["state_file"], "custom-state.json");
    assert_eq!(parsed["openclaw_bin"], "openclaw-custom");
    assert_eq!(parsed["agent_ruler_bin"], "agent-ruler-custom");
}

#[tokio::test]
async fn config_update_persists_runner_telegram_bridge_settings_and_masks_token() {
    let harness = UiHarness::new("ui-config-runner-telegram-bridge-update");

    let (status, payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/config/update",
        Some(json!({
            "claudecode_bridge": {
                "enabled": true,
                "answer_streaming_enabled": false,
                "poll_interval_seconds": 11,
                "decision_ttl_seconds": 1900,
                "short_id_length": 7,
                "state_file": "claudecode-telegram-state-custom.json",
                "bot_token": "123456:telegram-secret-token",
                "chat_ids": ["-1009876543210", "-1001122334455"],
                "allow_from": ["10001", "10002"]
            },
            "opencode_bridge": {
                "enabled": true,
                "answer_streaming_enabled": true,
                "poll_interval_seconds": 12,
                "decision_ttl_seconds": 2000,
                "short_id_length": 8,
                "state_file": "opencode-telegram-state-custom.json",
                "bot_token": "987654:telegram-secret-token-opencode",
                "chat_ids": ["-1009988776655"],
                "allow_from": ["20001"]
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["claudecode_bridge"]["config"]["enabled"], true);
    assert_eq!(
        payload["claudecode_bridge"]["config"]["answer_streaming_enabled"],
        false
    );
    assert_eq!(
        payload["claudecode_bridge"]["config"]["poll_interval_seconds"],
        11
    );
    assert_eq!(
        payload["claudecode_bridge"]["config"]["decision_ttl_seconds"],
        1900
    );
    assert_eq!(payload["claudecode_bridge"]["config"]["short_id_length"], 7);
    assert_eq!(
        payload["claudecode_bridge"]["config"]["state_file"],
        "claudecode-telegram-state-custom.json"
    );
    assert_eq!(
        payload["claudecode_bridge"]["config"]["chat_ids"],
        json!([])
    );
    assert_eq!(
        payload["claudecode_bridge"]["config"]["allow_from"][0],
        "10001"
    );
    assert_eq!(
        payload["claudecode_bridge"]["config"]["bot_token_configured"],
        true
    );
    assert_eq!(
        payload["claudecode_bridge"]["config"]["bot_token_masked"],
        "***oken"
    );

    assert_eq!(payload["opencode_bridge"]["config"]["enabled"], true);
    assert_eq!(
        payload["opencode_bridge"]["config"]["answer_streaming_enabled"],
        true
    );
    assert_eq!(
        payload["opencode_bridge"]["config"]["poll_interval_seconds"],
        12
    );
    assert_eq!(
        payload["opencode_bridge"]["config"]["decision_ttl_seconds"],
        2000
    );
    assert_eq!(payload["opencode_bridge"]["config"]["short_id_length"], 8);
    assert_eq!(
        payload["opencode_bridge"]["config"]["state_file"],
        "opencode-telegram-state-custom.json"
    );
    assert_eq!(payload["opencode_bridge"]["config"]["chat_ids"], json!([]));
    assert_eq!(
        payload["opencode_bridge"]["config"]["allow_from"][0],
        "20001"
    );
    assert_eq!(
        payload["opencode_bridge"]["config"]["bot_token_configured"],
        true
    );
    assert_eq!(
        payload["opencode_bridge"]["config"]["bot_token_masked"],
        "***code"
    );

    let claudecode_path = PathBuf::from(
        payload["claudecode_bridge"]["config_path"]
            .as_str()
            .expect("claudecode bridge config path"),
    );
    let claudecode_raw =
        fs::read_to_string(&claudecode_path).expect("read generated claudecode bridge config");
    let claudecode_parsed: Value =
        serde_json::from_str(&claudecode_raw).expect("parse generated claudecode bridge config");
    assert_eq!(claudecode_parsed["enabled"], true);
    assert_eq!(claudecode_parsed["answer_streaming_enabled"], false);
    assert_eq!(claudecode_parsed["poll_interval_seconds"], 11);
    assert_eq!(claudecode_parsed["decision_ttl_seconds"], 1900);
    assert_eq!(claudecode_parsed["short_id_length"], 7);
    assert_eq!(
        claudecode_parsed["state_file"],
        "claudecode-telegram-state-custom.json"
    );
    assert_eq!(
        claudecode_parsed["bot_token"],
        "123456:telegram-secret-token"
    );
    assert_eq!(claudecode_parsed["chat_ids"], json!([]));

    let opencode_path = PathBuf::from(
        payload["opencode_bridge"]["config_path"]
            .as_str()
            .expect("opencode bridge config path"),
    );
    let opencode_raw =
        fs::read_to_string(&opencode_path).expect("read generated opencode bridge config");
    let opencode_parsed: Value =
        serde_json::from_str(&opencode_raw).expect("parse generated opencode bridge config");
    assert_eq!(opencode_parsed["enabled"], true);
    assert_eq!(opencode_parsed["answer_streaming_enabled"], true);
    assert_eq!(opencode_parsed["poll_interval_seconds"], 12);
    assert_eq!(opencode_parsed["decision_ttl_seconds"], 2000);
    assert_eq!(opencode_parsed["short_id_length"], 8);
    assert_eq!(
        opencode_parsed["state_file"],
        "opencode-telegram-state-custom.json"
    );
    assert_eq!(
        opencode_parsed["bot_token"],
        "987654:telegram-secret-token-opencode"
    );
    assert_eq!(opencode_parsed["chat_ids"], json!([]));
}

#[tokio::test]
async fn config_update_rejects_invalid_runner_telegram_bot_token() {
    let harness = UiHarness::new("ui-config-runner-telegram-invalid-token");

    let (status, payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/config/update",
        Some(json!({
            "claudecode_bridge": {
                "bot_token": "panda@panda-VMware:~$ agent-ruler run -- claude error: preflight failed"
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        payload["error"]
            .as_str()
            .unwrap_or_default()
            .contains("bot_token must match Telegram format"),
        "error should surface invalid token guidance"
    );
}

#[tokio::test]
async fn config_update_syncs_selected_runner_bridge_ownership() {
    let harness = UiHarness::new("ui-config-runner-bridge-ownership-sync");
    let runtime = harness.runtime_state();
    let opencode_workspace = runtime
        .config
        .runtime_root
        .join("user_data")
        .join("runners")
        .join("opencode")
        .join("workspace");
    persist_runner_association(&runtime, RunnerKind::Opencode, opencode_workspace);

    let logs_dir = runtime.config.runtime_root.join("user_data").join("logs");
    fs::create_dir_all(&logs_dir).expect("create logs dir");
    let claudecode_pid = logs_dir.join("claudecode-telegram-channel-bridge.pid");
    let opencode_pid = logs_dir.join("opencode-telegram-channel-bridge.pid");
    fs::write(&claudecode_pid, "not-a-pid\n").expect("seed stale claudecode pid");
    fs::write(&opencode_pid, "not-a-pid\n").expect("seed stale opencode pid");

    let (status, _payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/config/update",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !claudecode_pid.exists(),
        "non-selected runner bridge pid should be cleared"
    );
    assert!(
        !opencode_pid.exists(),
        "selected runner bridge pid should be reconciled on config sync"
    );
}

#[tokio::test]
async fn status_feed_is_redacted_and_includes_resolved_states() {
    let harness = UiHarness::new("ui-status-feed");
    let runtime = harness.runtime_state();
    let approvals = ApprovalStore::new(&runtime.config.approvals_file);

    let approved = approvals
        .create_pending(
            &make_pending_action(
                "approved",
                runtime.config.shared_zone_dir.join("approved.txt"),
            ),
            &approval_decision(),
            "approved pending",
        )
        .expect("create first approval");
    let pending = approvals
        .create_pending(
            &make_pending_action(
                "pending",
                runtime.config.shared_zone_dir.join("pending.txt"),
            ),
            &approval_decision(),
            "pending approval",
        )
        .expect("create second approval");

    let approve_path = format!("/api/approvals/{}/approve", approved.id);
    let (approve_code, _) = call_json(&harness.app, Method::POST, &approve_path, None).await;
    assert_eq!(approve_code, StatusCode::OK);

    let (feed_code, feed_payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/status/feed?limit=10&include_resolved=true",
        None,
    )
    .await;
    assert_eq!(feed_code, StatusCode::OK);

    let events = feed_payload.as_array().expect("status feed array");
    assert!(events
        .iter()
        .any(|event| { event["approval_id"] == approved.id && event["verdict"] == "approved" }));
    assert!(events
        .iter()
        .any(|event| { event["approval_id"] == pending.id && event["verdict"] == "pending" }));

    let workspace_path = runtime.config.workspace.to_string_lossy().to_string();
    for event in events {
        let blob = serde_json::to_string(event).expect("serialize redacted event");
        assert!(
            !blob.contains(&workspace_path),
            "redacted event leaked workspace path: {blob}"
        );
        assert!(event["reason_code"]
            .as_str()
            .unwrap_or_default()
            .contains("approval_required"));
        assert!(event["target_classification"].is_string());
        assert!(event["open_in_webui"]
            .as_str()
            .unwrap_or_default()
            .starts_with("/approvals/"));
    }
}

#[tokio::test]
async fn export_stage_and_delivery_flow_work() {
    let harness = UiHarness::new("ui-export-flow");
    let runtime = harness.runtime_state();

    fs::write(
        runtime.config.workspace.join("report.txt"),
        "release-notes-v1\n",
    )
    .expect("write workspace report");

    let (preview_code, preview) = call_json(
        &harness.app,
        Method::POST,
        "/api/export/preview",
        Some(json!({
            "src": "report.txt",
            "dst": "report.txt"
        })),
    )
    .await;
    assert_eq!(preview_code, StatusCode::OK);
    assert!(preview["diff_preview"]
        .as_str()
        .unwrap_or_default()
        .contains("release-notes-v1"));

    let (request_code, request) = call_json(
        &harness.app,
        Method::POST,
        "/api/export/request",
        Some(json!({
            "src": "report.txt",
            "dst": "report.txt"
        })),
    )
    .await;
    assert_eq!(request_code, StatusCode::ACCEPTED);
    assert_eq!(request["status"], "pending_approval");

    let approval_id = request["approval_id"]
        .as_str()
        .expect("approval id in export response");
    let stage_id = request["stage_id"]
        .as_str()
        .expect("stage id in export response")
        .to_string();

    let approve_path = format!("/api/approvals/{approval_id}/approve");
    let (approve_code, _) = call_json(&harness.app, Method::POST, &approve_path, None).await;
    assert_eq!(approve_code, StatusCode::OK);

    let exported = runtime.config.shared_zone_dir.join("report.txt");
    assert!(exported.exists(), "expected export to be committed");
    let exported_content = fs::read_to_string(&exported).expect("read exported report");
    assert!(exported_content.contains("release-notes-v1"));

    let delivery_target = harness.runtime.path().join("delivered").join("report.txt");

    let (deliver_preview_code, deliver_preview) = call_json(
        &harness.app,
        Method::POST,
        "/api/export/deliver/preview",
        Some(json!({
            "stage_ref": stage_id,
            "dst": delivery_target.to_string_lossy()
        })),
    )
    .await;
    assert_eq!(deliver_preview_code, StatusCode::OK);
    assert!(deliver_preview["diff_preview"]
        .as_str()
        .unwrap_or_default()
        .contains("release-notes-v1"));

    let (deliver_code, deliver) = call_json(
        &harness.app,
        Method::POST,
        "/api/export/deliver/request",
        Some(json!({
            "stage_ref": "report.txt",
            "dst": delivery_target.to_string_lossy()
        })),
    )
    .await;
    assert_eq!(deliver_code, StatusCode::ACCEPTED);
    assert_eq!(deliver["status"], "pending_approval");

    let delivery_approval_id = deliver["approval_id"]
        .as_str()
        .expect("approval id in delivery response");
    let delivery_approve_path = format!("/api/approvals/{delivery_approval_id}/approve");
    let (delivery_approve_code, _) =
        call_json(&harness.app, Method::POST, &delivery_approve_path, None).await;
    assert_eq!(delivery_approve_code, StatusCode::OK);

    assert!(delivery_target.exists(), "expected delivered output");
    let delivered = fs::read_to_string(&delivery_target).expect("read delivered report");
    assert!(delivered.contains("release-notes-v1"));

    let receipts_raw = fs::read_to_string(runtime.config.receipts_file).expect("read receipts");
    assert!(receipts_raw.contains("\"reason\":\"approval_required_export\""));
    assert!(receipts_raw.contains("export staged after approval"));
    assert!(receipts_raw.contains("Delivered to"));
}

#[tokio::test]
async fn delivery_request_defaults_to_runtime_user_destination_when_dst_omitted() {
    let harness = UiHarness::new("ui-delivery-default-dst");
    let runtime = harness.runtime_state();

    fs::write(
        runtime.config.shared_zone_dir.join("report.txt"),
        "release-notes-default-dst\n",
    )
    .expect("write shared-zone report");

    let (deliver_code, deliver) = call_json(
        &harness.app,
        Method::POST,
        "/api/export/deliver/request",
        Some(json!({
            "stage_ref": "report.txt",
            "auto_approve": true,
            "auto_approve_origin": "control_panel_user"
        })),
    )
    .await;
    assert_eq!(deliver_code, StatusCode::OK);
    assert_eq!(deliver["status"], "delivered");

    let delivery_target = runtime.config.default_delivery_dir.join("report.txt");
    assert!(
        delivery_target.exists(),
        "expected delivery to land in runtime default user destination"
    );
    let delivered = fs::read_to_string(&delivery_target).expect("read delivered report");
    assert!(delivered.contains("release-notes-default-dst"));
}

#[tokio::test]
async fn delivery_request_rejects_stage_reference_outside_shared_zone() {
    let harness = UiHarness::new("ui-delivery-stage-ref-shared-zone-only");
    let runtime = harness.runtime_state();

    fs::write(
        runtime.config.workspace.join("outside-shared-zone.txt"),
        "outside shared zone\n",
    )
    .expect("write workspace file");

    let outside_ref = runtime.config.workspace.join("outside-shared-zone.txt");
    let (deliver_code, deliver_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/export/deliver/request",
        Some(json!({
            "stage_ref": outside_ref.to_string_lossy(),
            "auto_approve": true,
            "auto_approve_origin": "control_panel_user"
        })),
    )
    .await;

    assert_eq!(deliver_code, StatusCode::BAD_REQUEST);
    assert!(deliver_payload["error"]
        .as_str()
        .unwrap_or_default()
        .contains("stage reference must stay within shared zone"));
}

#[tokio::test]
async fn claudecode_run_stage_and_deliver_uses_managed_workspace_and_default_destination() {
    let harness = UiHarness::new("ui-claude-run-stage-deliver");
    let runtime = harness.runtime_state();
    let claudecode_workspace = runtime
        .config
        .runtime_root
        .join("user_data")
        .join("runners")
        .join("claudecode")
        .join("workspace");
    persist_runner_association(
        &runtime,
        RunnerKind::Claudecode,
        claudecode_workspace.clone(),
    );

    let runtime = harness.runtime_state();
    let shim = write_runner_shim(
        &claudecode_workspace,
        "claude",
        r#"printf 'This is a simple test file.\nCreated on 2026-03-10.\n' > test.txt"#,
    );
    let mut runtime = runtime;
    runtime.policy.rules.execution.deny_workspace_exec = false;
    runtime.policy.rules.execution.deny_tmp_exec = false;
    runtime.policy_hash = runtime.policy.policy_hash().expect("policy hash");
    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let workspace_root = workspace_root_for_runner_id(&runtime, Some("claudecode"))
        .expect("resolve Claude workspace");
    let engine = PolicyEngine::new(runtime.policy.clone(), workspace_root.clone());

    let run = run_confined(
        &[
            shim.to_string_lossy().to_string(),
            "-p".to_string(),
            "create test file".to_string(),
        ],
        &runtime,
        &engine,
        &approvals,
        &receipts,
    );
    let run = match run {
        Ok(value) => value,
        Err(err) => {
            if is_confinement_env_error(&err.to_string()) {
                eprintln!("skipping claude stage+deliver runner assertion due host limits: {err}");
                return;
            }
            panic!("run claude shim: {err}");
        }
    };
    assert_eq!(run.exit_code, 0);

    let managed_file = claudecode_workspace.join("test.txt");
    assert!(
        managed_file.exists(),
        "expected Claude runner file in managed workspace"
    );
    assert!(
        !runtime.config.workspace.join("test.txt").exists(),
        "runner should not write into the default runtime workspace"
    );

    let (stage_code, stage_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/export/request",
        Some(json!({
            "src": "test.txt",
            "runner": "claudecode",
            "auto_approve": true,
            "auto_approve_origin": "control_panel_user"
        })),
    )
    .await;
    assert_eq!(stage_code, StatusCode::OK);
    assert_eq!(stage_payload["status"], "staged");

    let (deliver_code, deliver_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/export/deliver/request",
        Some(json!({
            "stage_ref": "test.txt",
            "runner": "claudecode",
            "auto_approve": true,
            "auto_approve_origin": "control_panel_user"
        })),
    )
    .await;
    assert_eq!(deliver_code, StatusCode::OK);
    assert_eq!(deliver_payload["status"], "delivered");

    let delivery_target = runtime.config.default_delivery_dir.join("test.txt");
    assert!(
        delivery_target.exists(),
        "expected delivered file in runtime default destination"
    );
    let delivered = fs::read_to_string(&delivery_target).expect("read delivered file");
    assert!(delivered.contains("This is a simple test file."));
}

#[tokio::test]
async fn export_stage_rejects_destination_outside_shared_zone() {
    let harness = UiHarness::new("ui-export-stage-shared-zone-only");
    let runtime = harness.runtime_state();

    fs::write(
        runtime.config.workspace.join("report.txt"),
        "release-notes-v2\n",
    )
    .expect("write workspace report");

    let outside_dst = harness.project.path().join("outside-stage.txt");
    let (preview_code, preview_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/export/preview",
        Some(json!({
            "src": "report.txt",
            "dst": outside_dst.to_string_lossy()
        })),
    )
    .await;
    assert_eq!(preview_code, StatusCode::BAD_REQUEST);
    assert!(preview_payload["error"]
        .as_str()
        .unwrap_or_default()
        .contains("stage destination must stay within shared zone"));

    let (request_code, request_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/export/request",
        Some(json!({
            "src": "report.txt",
            "dst": outside_dst.to_string_lossy()
        })),
    )
    .await;
    assert_eq!(request_code, StatusCode::BAD_REQUEST);
    assert!(request_payload["error"]
        .as_str()
        .unwrap_or_default()
        .contains("stage destination must stay within shared zone"));

    let inside_dst = runtime.config.shared_zone_dir.join("inside-stage.txt");
    let (inside_preview_code, _) = call_json(
        &harness.app,
        Method::POST,
        "/api/export/preview",
        Some(json!({
            "src": "report.txt",
            "dst": inside_dst.to_string_lossy()
        })),
    )
    .await;
    assert_eq!(inside_preview_code, StatusCode::OK);
}

#[tokio::test]
async fn import_preview_and_request_work() {
    let harness = UiHarness::new("ui-import-flow");
    let runtime = harness.runtime_state();

    let input = harness.project.path().join("import-me.txt");
    fs::write(&input, "import-content\n").expect("write import src");

    let (preview_code, preview) = call_json(
        &harness.app,
        Method::POST,
        "/api/import/preview",
        Some(json!({
            "src": input.to_string_lossy(),
            "dst": "imported.txt"
        })),
    )
    .await;
    assert_eq!(preview_code, StatusCode::OK);
    assert!(preview["diff_preview"]
        .as_str()
        .unwrap_or_default()
        .contains("import-content"));

    let (request_code, request) = call_json(
        &harness.app,
        Method::POST,
        "/api/import/request",
        Some(json!({
            "src": input.to_string_lossy(),
            "dst": "imported.txt"
        })),
    )
    .await;
    assert_eq!(request_code, StatusCode::OK);
    assert_eq!(request["status"], "completed");

    let imported = runtime.config.workspace.join("imported.txt");
    assert!(imported.exists(), "expected imported output");
    let imported_body = fs::read_to_string(&imported).expect("read imported output");
    assert!(imported_body.contains("import-content"));
}

#[tokio::test]
async fn reset_exec_and_run_script_endpoints_work() {
    let harness = UiHarness::new("ui-reset-run");
    let runtime = harness.runtime_state();

    let stale_exec_artifact = runtime.config.exec_layer_dir.join("stale.bin");
    fs::write(&stale_exec_artifact, "stale").expect("seed exec artifact");

    let (reset_code, reset_payload) =
        call_json(&harness.app, Method::POST, "/api/reset-exec", None).await;
    assert_eq!(reset_code, StatusCode::OK);
    assert_eq!(reset_payload["status"], "reset");
    assert!(runtime.config.exec_layer_dir.exists());
    assert!(
        !stale_exec_artifact.exists(),
        "reset should clear old exec artifacts"
    );

    // CLI-parity command endpoint should keep the same deterministic block semantics as `agent-ruler run`.
    let (blocked_code, blocked_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/run/command",
        Some(json!({
            "cmd": ["rm", "/etc/passwd"]
        })),
    )
    .await;
    assert_eq!(blocked_code, StatusCode::BAD_REQUEST);
    assert_eq!(blocked_payload["status"], "failed");
    assert!(blocked_payload["error"]
        .as_str()
        .unwrap_or_default()
        .contains("blocked by policy"));

    let receipts_raw = fs::read_to_string(runtime.config.receipts_file).expect("read receipts");
    assert!(receipts_raw.contains("\"reason\":\"deny_system_critical\""));

    let (redacted_receipts_code, redacted_receipts_payload) =
        call_json(&harness.app, Method::GET, "/api/receipts?limit=1", None).await;
    assert_eq!(redacted_receipts_code, StatusCode::OK);
    assert!(!redacted_receipts_payload["items"][0]["decision"]["detail"]
        .as_str()
        .unwrap_or_default()
        .is_empty());
    assert_eq!(
        redacted_receipts_payload["items"][0]["action"]["process"]["command"],
        ""
    );

    let (detailed_receipts_code, detailed_receipts_payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/receipts?limit=1&include_details=true",
        None,
    )
    .await;
    assert_eq!(detailed_receipts_code, StatusCode::OK);
    let detailed_receipt_detail = detailed_receipts_payload["items"][0]["decision"]["detail"]
        .as_str()
        .unwrap_or_default();
    assert!(
        !detailed_receipt_detail.is_empty(),
        "expected detail text when include_details=1"
    );
    assert_eq!(
        detailed_receipts_payload["items"][0]["action"]["process"]["command"],
        "rm"
    );

    // Success path may depend on host user-namespace allowances; skip if host rejects bwrap.
    let (run_code, run_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/run/script",
        Some(json!({
            "script": "echo ui-ok > from-ui.txt"
        })),
    )
    .await;

    if run_code == StatusCode::OK {
        assert_eq!(run_payload["status"], "completed");
        assert!(
            run_payload["stdout"].is_string(),
            "run-script success should include stdout field"
        );
        assert!(
            run_payload["stderr"].is_string(),
            "run-script success should include stderr field"
        );
        let output_file = runtime.config.workspace.join("from-ui.txt");
        assert!(
            output_file.exists(),
            "run endpoint should write inside workspace"
        );
        return;
    }

    let err = run_payload["error"].as_str().unwrap_or_default();
    if is_confinement_env_error(err) {
        eprintln!("skipping run success assertion due host confinement limits: {err}");
        return;
    }

    panic!("unexpected run-script failure: status={run_code} payload={run_payload}");
}

#[tokio::test]
async fn openclaw_tool_preflight_endpoint_logs_and_blocks_system_write() {
    let harness = UiHarness::new("ui-openclaw-tool-preflight");
    let runtime = harness.runtime_state();

    let (deny_code, deny_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/openclaw/tool/preflight",
        Some(json!({
            "tool_name": "write",
            "params": {
                "path": "/etc/systemd/system/myservice.service",
                "content": "[Service]\nExecStart=/bin/bash"
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-preflight-test"
            }
        })),
    )
    .await;
    assert_eq!(deny_code, StatusCode::OK);
    assert_eq!(deny_payload["status"], "denied");
    assert_eq!(deny_payload["blocked"], true);
    assert_eq!(deny_payload["reason"], "deny_system_critical");

    let allowed_path = runtime.config.workspace.join("safe-write.txt");
    let (allow_code, allow_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/openclaw/tool/preflight",
        Some(json!({
            "tool_name": "write",
            "params": {
                "path": allowed_path.to_string_lossy(),
                "content": "hello"
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-preflight-test"
            }
        })),
    )
    .await;
    assert_eq!(allow_code, StatusCode::OK);
    assert_eq!(allow_payload["status"], "allow");
    assert_eq!(allow_payload["blocked"], false);
    assert_eq!(allow_payload["reason"], "allowed_by_policy");

    let receipts_raw = fs::read_to_string(runtime.config.receipts_file).expect("read receipts");
    assert!(
        receipts_raw.contains("\"operation\":\"openclaw_tool_write\""),
        "expected openclaw write preflight operation in receipts"
    );
    assert!(
        receipts_raw.contains("\"reason\":\"deny_system_critical\""),
        "expected denied system-critical reason in receipts"
    );
    assert!(
        receipts_raw.contains("\"reason\":\"allowed_by_policy\""),
        "expected allow reason in receipts"
    );
}

#[tokio::test]
async fn runner_tool_preflight_generic_endpoint_supports_claudecode() {
    let harness = UiHarness::new("ui-runner-tool-preflight-claude");
    let runtime = harness.runtime_state();

    let (deny_code, deny_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/runners/claudecode/tool/preflight",
        Some(json!({
            "tool_name": "write",
            "params": {
                "path": "/etc/systemd/system/blocked.service",
                "content": "x"
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-runner-claude"
            }
        })),
    )
    .await;
    assert_eq!(deny_code, StatusCode::OK);
    assert_eq!(deny_payload["status"], "denied");
    assert_eq!(deny_payload["blocked"], true);
    assert_eq!(deny_payload["reason"], "deny_system_critical");

    let receipts_raw = fs::read_to_string(runtime.config.receipts_file).expect("read receipts");
    assert!(
        receipts_raw.contains("\"operation\":\"claudecode_tool_write\""),
        "expected claudecode write preflight operation in receipts"
    );
    assert!(
        receipts_raw.contains("\"runner_id\":\"claudecode\""),
        "expected claudecode runner id in preflight receipt"
    );
}

#[tokio::test]
async fn runner_tool_preflight_alias_endpoint_supports_opencode() {
    let harness = UiHarness::new("ui-runner-tool-preflight-opencode");
    let runtime = harness.runtime_state();

    let (deny_code, deny_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/opencode/tool/preflight",
        Some(json!({
            "tool_name": "write",
            "params": {
                "path": "/etc/systemd/system/blocked-opencode.service",
                "content": "x"
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-runner-opencode"
            }
        })),
    )
    .await;
    assert_eq!(deny_code, StatusCode::OK);
    assert_eq!(deny_payload["status"], "denied");
    assert_eq!(deny_payload["blocked"], true);
    assert_eq!(deny_payload["reason"], "deny_system_critical");

    let receipts_raw = fs::read_to_string(runtime.config.receipts_file).expect("read receipts");
    assert!(
        receipts_raw.contains("\"operation\":\"opencode_tool_write\""),
        "expected opencode write preflight operation in receipts"
    );
    assert!(
        receipts_raw.contains("\"runner_id\":\"opencode\""),
        "expected opencode runner id in preflight receipt"
    );
}

#[tokio::test]
async fn runner_tool_preflight_resolves_relative_paths_against_selected_runner_workspace() {
    let harness = UiHarness::new("ui-runner-tool-preflight-relative-path");
    let runtime = harness.runtime_state();
    let opencode_workspace = runtime
        .config
        .runtime_root
        .join("user_data")
        .join("runners")
        .join("opencode")
        .join("workspace");
    fs::create_dir_all(&opencode_workspace).expect("create opencode workspace");
    persist_runner_association(&runtime, RunnerKind::Opencode, opencode_workspace.clone());

    let (code, payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/opencode/tool/preflight",
        Some(json!({
            "tool_name": "write",
            "params": {
                "path": "runner-aware.txt",
                "content": "x"
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-relative-opencode"
            }
        })),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(payload["status"], "allow");
    assert_eq!(payload["blocked"], false);

    let receipts_raw = fs::read_to_string(runtime.config.receipts_file).expect("read receipts");
    assert!(
        receipts_raw.contains(
            opencode_workspace
                .join("runner-aware.txt")
                .to_string_lossy()
                .as_ref()
        ),
        "expected relative runner tool path to resolve inside opencode managed workspace"
    );
    assert!(
        !receipts_raw.contains(runtime.config.workspace.join("runner-aware.txt").to_string_lossy().as_ref()),
        "runner tool preflight should not resolve relative opencode paths against the default workspace"
    );
}

#[tokio::test]
async fn runner_tool_preflight_reuses_active_approval_within_runner_session() {
    let harness = UiHarness::new("ui-runner-tool-preflight-approval-reuse");

    let command_text = "rm -rf /opt/shared/retry-approved.txt";
    let (pending_code, pending_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/opencode/tool/preflight",
        Some(json!({
            "tool_name": "exec",
            "params": {
                "command": command_text
            },
            "context": {
                "agent_id": "agent-a",
                "session_key": "session-a"
            }
        })),
    )
    .await;
    assert_eq!(pending_code, StatusCode::OK);
    assert_eq!(pending_payload["status"], "pending_approval");
    assert_eq!(pending_payload["blocked"], true);

    let approval_id = pending_payload["approval_id"]
        .as_str()
        .expect("approval id for pending runner preflight");
    let approve_path = format!("/api/approvals/{approval_id}/approve");
    let (approve_code, _) = call_json(&harness.app, Method::POST, &approve_path, None).await;
    assert_eq!(approve_code, StatusCode::OK);

    let (retry_code, retry_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/opencode/tool/preflight",
        Some(json!({
            "tool_name": "exec",
            "params": {
                "command": command_text
            },
            "context": {
                "agent_id": "agent-b",
                "session_key": "session-a"
            }
        })),
    )
    .await;
    assert_eq!(retry_code, StatusCode::OK);
    assert_eq!(retry_payload["status"], "allow");
    assert_eq!(retry_payload["blocked"], false);
    assert_eq!(retry_payload["reason"], "allowed_by_policy");
    assert!(retry_payload["detail"]
        .as_str()
        .unwrap_or_default()
        .contains("active approval scope"));
}

#[tokio::test]
async fn runner_tool_preflight_reuses_active_approval_within_runner_session_claudecode() {
    let harness = UiHarness::new("ui-runner-tool-preflight-approval-reuse-claude");

    let command_text = "rm -rf /opt/shared/retry-approved-claude.txt";
    let (pending_code, pending_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/claudecode/tool/preflight",
        Some(json!({
            "tool_name": "exec",
            "params": {
                "command": command_text
            },
            "context": {
                "agent_id": "agent-a",
                "session_key": "session-a"
            }
        })),
    )
    .await;
    assert_eq!(pending_code, StatusCode::OK);
    assert_eq!(pending_payload["status"], "pending_approval");
    assert_eq!(pending_payload["blocked"], true);

    let approval_id = pending_payload["approval_id"]
        .as_str()
        .expect("approval id for pending runner preflight");
    let approve_path = format!("/api/approvals/{approval_id}/approve");
    let (approve_code, _) = call_json(&harness.app, Method::POST, &approve_path, None).await;
    assert_eq!(approve_code, StatusCode::OK);

    let (retry_code, retry_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/claudecode/tool/preflight",
        Some(json!({
            "tool_name": "exec",
            "params": {
                "command": command_text
            },
            "context": {
                "agent_id": "agent-b",
                "session_key": "session-a"
            }
        })),
    )
    .await;
    assert_eq!(retry_code, StatusCode::OK);
    assert_eq!(retry_payload["status"], "allow");
    assert_eq!(retry_payload["blocked"], false);
    assert_eq!(retry_payload["reason"], "allowed_by_policy");
    assert!(retry_payload["detail"]
        .as_str()
        .unwrap_or_default()
        .contains("active approval scope"));
}

#[tokio::test]
async fn openclaw_tool_preflight_normalizes_tool_name_aliases() {
    let harness = UiHarness::new("ui-openclaw-tool-preflight-alias");

    let (deny_code, deny_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/openclaw/tool/preflight",
        Some(json!({
            "tool_name": "filesystem.write_file",
            "params": {
                "path": "/etc/systemd/system/alias.service",
                "content": "x"
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-preflight-alias"
            }
        })),
    )
    .await;

    assert_eq!(deny_code, StatusCode::OK);
    assert_eq!(deny_payload["status"], "denied");
    assert_eq!(deny_payload["blocked"], true);
    assert_eq!(deny_payload["reason"], "deny_system_critical");
}

/// Test that exec commands with destructive file operations (rm) are subject to
/// filesystem zone policies. This prevents bypassing write/delete restrictions
/// by using shell commands like `rm /path/to/file`.
#[tokio::test]
async fn openclaw_tool_preflight_blocks_destructive_exec_in_protected_zones() {
    let harness = UiHarness::new("ui-openclaw-exec-rm");
    let runtime = harness.runtime_state();

    // Test 1: rm on system-critical path should be blocked
    let (deny_code, deny_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/openclaw/tool/preflight",
        Some(json!({
            "tool_name": "exec",
            "params": {
                "command": "rm -f /etc/passwd"
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-exec-rm-test"
            }
        })),
    )
    .await;
    assert_eq!(deny_code, StatusCode::OK);
    assert_eq!(deny_payload["status"], "denied");
    assert_eq!(deny_payload["blocked"], true);
    assert_eq!(deny_payload["reason"], "deny_system_critical");

    // Test 2: rm on shared zone path should require approval
    let (deny_shared_code, deny_shared_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/openclaw/tool/preflight",
        Some(json!({
            "tool_name": "exec",
            "params": {
                "command": "rm -rf /opt/shared/important.txt"
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-exec-rm-test"
            }
        })),
    )
    .await;
    assert_eq!(deny_shared_code, StatusCode::OK);
    // Should require approval because shared zone requires approval for writes/deletes
    assert_eq!(deny_shared_payload["blocked"], true);
    assert_eq!(deny_shared_payload["status"], "pending_approval");

    // Test 3: rm within workspace should be allowed
    let workspace_path = runtime.config.workspace.join("test-file.txt");
    let (allow_code, allow_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/openclaw/tool/preflight",
        Some(json!({
            "tool_name": "exec",
            "params": {
                "command": format!("rm {}", workspace_path.to_string_lossy())
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-exec-rm-test"
            }
        })),
    )
    .await;
    assert_eq!(allow_code, StatusCode::OK);
    assert_eq!(allow_payload["status"], "allow");
    assert_eq!(allow_payload["blocked"], false);

    // Test 4: Non-destructive exec commands should work as before
    let (normal_code, normal_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/openclaw/tool/preflight",
        Some(json!({
            "tool_name": "exec",
            "params": {
                "command": "ls -la /usr/bin"
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-exec-normal-test"
            }
        })),
    )
    .await;
    assert_eq!(normal_code, StatusCode::OK);
    // Non-destructive commands should be ignored (not blocked) by the destructive detection
    assert_eq!(normal_payload["status"], "allow");
}

#[tokio::test]
async fn openclaw_tool_preflight_blocks_shell_redirection_writes_to_protected_paths() {
    let harness = UiHarness::new("ui-openclaw-redirection");
    let runtime = harness.runtime_state();

    let (deny_code, deny_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/openclaw/tool/preflight",
        Some(json!({
            "tool_name": "exec",
            "params": {
                "command": "echo malicious > /etc/systemd/system/malicious.service"
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-exec-redirection-test"
            }
        })),
    )
    .await;
    assert_eq!(deny_code, StatusCode::OK);
    assert_eq!(deny_payload["status"], "denied");
    assert_eq!(deny_payload["blocked"], true);
    assert_eq!(deny_payload["reason"], "deny_system_critical");

    let workspace_target = runtime.config.workspace.join("redirection-safe.txt");
    let (allow_code, allow_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/openclaw/tool/preflight",
        Some(json!({
            "tool_name": "exec",
            "params": {
                "command": format!("echo allowed > {}", workspace_target.to_string_lossy())
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-exec-redirection-test"
            }
        })),
    )
    .await;
    assert_eq!(allow_code, StatusCode::OK);
    assert_eq!(allow_payload["status"], "allow");
    assert_eq!(allow_payload["blocked"], false);
}

#[tokio::test]
async fn openclaw_tool_preflight_blocks_interpreter_stream_exec() {
    let harness = UiHarness::new("ui-openclaw-stream-exec");

    let (code, payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/openclaw/tool/preflight",
        Some(json!({
            "tool_name": "exec",
            "params": {
                "command": "bash <(echo 'echo injected')"
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-stream-exec-test"
            }
        })),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(payload["status"], "denied");
    assert_eq!(payload["blocked"], true);
    assert_eq!(payload["reason"], "deny_interpreter_stream_exec");
}

#[tokio::test]
async fn openclaw_tool_preflight_blocks_agent_ruler_cli_exec() {
    let harness = UiHarness::new("ui-openclaw-agent-ruler-cli");

    let (code, payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/openclaw/tool/preflight",
        Some(json!({
            "tool_name": "exec",
            "params": {
                "command": "agent-ruler status --json"
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-agent-ruler-cli-test"
            }
        })),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(payload["status"], "denied");
    assert_eq!(payload["blocked"], true);
    assert_eq!(payload["reason"], "deny_system_critical");
    assert!(payload["detail"]
        .as_str()
        .unwrap_or_default()
        .contains("operator-only"));
}

#[tokio::test]
async fn openclaw_tool_preflight_blocks_agent_ruler_internal_paths() {
    let harness = UiHarness::new("ui-openclaw-internal-paths");
    let runtime = harness.runtime_state();

    let policy_path = runtime.config.state_dir.join("policy.yaml");
    let (state_code, state_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/openclaw/tool/preflight",
        Some(json!({
            "tool_name": "read",
            "params": {
                "path": policy_path.to_string_lossy()
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-internal-state-test"
            }
        })),
    )
    .await;
    assert_eq!(state_code, StatusCode::OK);
    assert_eq!(state_payload["status"], "denied");
    assert_eq!(state_payload["reason"], "deny_system_critical");

    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/ui.rs");
    let (src_code, src_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/openclaw/tool/preflight",
        Some(json!({
            "tool_name": "read",
            "params": {
                "path": source_path.to_string_lossy()
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-internal-source-test"
            }
        })),
    )
    .await;
    assert_eq!(src_code, StatusCode::OK);
    assert_eq!(src_payload["status"], "denied");
    assert_eq!(src_payload["reason"], "deny_system_critical");
}

#[tokio::test]
async fn openclaw_tool_preflight_blocks_direct_delivery_destination_copy() {
    let harness = UiHarness::new("ui-openclaw-delivery-bypass");
    let runtime = harness.runtime_state();

    let delivery_target = runtime
        .config
        .default_delivery_dir
        .join("bypass-attempt.txt");

    let (code, payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/openclaw/tool/preflight",
        Some(json!({
            "tool_name": "exec",
            "params": {
                "command": format!("cp workspace-note.txt {}", delivery_target.to_string_lossy())
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-delivery-bypass-test"
            }
        })),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(payload["status"], "denied");
    assert_eq!(payload["blocked"], true);
    assert_eq!(payload["reason"], "deny_user_data_write");
    assert!(payload["detail"]
        .as_str()
        .unwrap_or_default()
        .contains("stage + deliver flow"));
}

#[tokio::test]
async fn openclaw_tool_preflight_expands_tilde_for_secret_paths() {
    let harness = UiHarness::new("ui-openclaw-tilde-secrets");

    let (code, payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/openclaw/tool/preflight",
        Some(json!({
            "tool_name": "read",
            "params": {
                "path": "~/.ssh/id_rsa"
            },
            "context": {
                "agent_id": "main",
                "session_key": "session-tilde-secrets-test"
            }
        })),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(payload["status"], "denied");
    assert_eq!(payload["reason"], "deny_secrets");
}

#[tokio::test]
async fn reset_runtime_endpoint_supports_keep_config_toggle() {
    let harness = UiHarness::new("ui-reset-runtime");
    let runtime = harness.runtime_state();

    let original_bind = runtime.config.ui_bind.clone();
    let original_profile = runtime.policy.profile.clone();

    let (_, update_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/config/update",
        Some(json!({
            "ui_bind": "127.0.0.1:4999"
        })),
    )
    .await;
    assert_eq!(update_payload["status"], "updated");

    let (keep_code, keep_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/reset-runtime",
        Some(json!({ "keep_config": true })),
    )
    .await;
    assert_eq!(keep_code, StatusCode::OK);
    assert_eq!(keep_payload["status"], "reset");
    assert_eq!(
        keep_payload["config_impact"],
        "preserved_existing_config_and_policy"
    );

    let (_, runtime_after_keep) = call_json(&harness.app, Method::GET, "/api/runtime", None).await;
    assert_eq!(runtime_after_keep["ui_bind"], "127.0.0.1:4999");

    let (default_code, default_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/reset-runtime",
        Some(json!({ "keep_config": false })),
    )
    .await;
    assert_eq!(default_code, StatusCode::OK);
    assert_eq!(
        default_payload["config_impact"],
        "restored_default_config_and_policy"
    );

    let (_, runtime_after_default) =
        call_json(&harness.app, Method::GET, "/api/runtime", None).await;
    assert_eq!(runtime_after_default["ui_bind"], original_bind);

    let (_, policy_after_default) = call_json(&harness.app, Method::GET, "/api/policy", None).await;
    assert_eq!(policy_after_default["profile"], original_profile);
}

#[tokio::test]
async fn user_auto_approve_mode_skips_pending_queue_for_export() {
    let harness = UiHarness::new("ui-auto-approve");
    let runtime = harness.runtime_state();

    fs::write(runtime.config.workspace.join("note.txt"), "hello\n").expect("write workspace file");

    let (stage_code, stage_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/export/request",
        Some(json!({
            "src": "note.txt",
            "dst": "note.txt",
            "auto_approve": true,
            "auto_approve_origin": "control_panel_user"
        })),
    )
    .await;
    assert_eq!(stage_code, StatusCode::OK);
    assert_eq!(stage_payload["status"], "staged");

    let (_, pending) = call_json(&harness.app, Method::GET, "/api/approvals", None).await;
    assert_eq!(pending.as_array().unwrap_or(&Vec::new()).len(), 0);
}

#[tokio::test]
async fn auto_approve_requires_control_panel_origin() {
    let harness = UiHarness::new("ui-auto-approve-origin");
    let runtime = harness.runtime_state();

    fs::write(runtime.config.workspace.join("note.txt"), "hello\n").expect("write workspace file");

    let (stage_code, stage_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/export/request",
        Some(json!({
            "src": "note.txt",
            "dst": "note.txt",
            "auto_approve": true
        })),
    )
    .await;
    assert_eq!(stage_code, StatusCode::ACCEPTED);
    assert_eq!(stage_payload["status"], "pending_approval");

    let (_, pending) = call_json(&harness.app, Method::GET, "/api/approvals", None).await;
    assert_eq!(pending.as_array().unwrap_or(&Vec::new()).len(), 1);
}

#[tokio::test]
async fn files_list_endpoint_returns_workspace_entries() {
    let harness = UiHarness::new("ui-files-list");
    let runtime = harness.runtime_state();

    fs::create_dir_all(runtime.config.workspace.join("nested")).expect("create nested dir");
    fs::write(
        runtime.config.workspace.join("nested/report.txt"),
        "payload\n",
    )
    .expect("write workspace file");

    let (code, payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/files/list?zone=workspace&limit=50",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK);

    let entries = payload.as_array().expect("files list array");
    assert!(entries
        .iter()
        .any(|item| item["path"] == "nested/report.txt"));
}

#[tokio::test]
async fn files_list_prefix_supports_subtree_and_blocks_parent_traversal() {
    let harness = UiHarness::new("ui-files-prefix");
    let runtime = harness.runtime_state();

    fs::create_dir_all(runtime.config.workspace.join("nested/sub")).expect("create nested dirs");
    fs::write(
        runtime.config.workspace.join("nested/sub/report.txt"),
        "payload\n",
    )
    .expect("write workspace file");

    let (subtree_code, subtree_payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/files/list?zone=workspace&prefix=nested&limit=50",
        None,
    )
    .await;
    assert_eq!(subtree_code, StatusCode::OK);
    let entries = subtree_payload
        .as_array()
        .expect("subtree files list array");
    assert!(entries
        .iter()
        .any(|item| item["path"] == "nested/sub/report.txt"));

    let (invalid_code, invalid_payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/files/list?zone=workspace&prefix=../etc&limit=50",
        None,
    )
    .await;
    assert_eq!(invalid_code, StatusCode::BAD_REQUEST);
    assert!(invalid_payload["error"]
        .as_str()
        .unwrap_or_default()
        .contains("prefix cannot include parent directory traversals"));
}

#[tokio::test]
async fn files_list_limit_keeps_top_level_workspace_entries_visible() {
    let harness = UiHarness::new("ui-files-limit-priority");
    let runtime = harness.runtime_state();

    fs::write(runtime.config.workspace.join("0-root.md"), "root\n").expect("write root file");
    fs::create_dir_all(runtime.config.workspace.join("zzz/deep")).expect("create deep dirs");
    for idx in 0..120usize {
        fs::write(
            runtime
                .config
                .workspace
                .join("zzz/deep")
                .join(format!("nested-{idx:03}.txt")),
            "payload\n",
        )
        .expect("write deep file");
    }

    let (code, payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/files/list?zone=workspace&limit=1",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK);

    let entries = payload.as_array().expect("files list array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["path"], "0-root.md");
}

#[tokio::test]
async fn files_list_deep_parent_traversal_rejected() {
    let harness = UiHarness::new("ui-files-prefix");
    let (code, payload) = call_json(
        &harness.app,
        Method::GET,
        "/api/files/list?zone=workspace&prefix=../../etc&limit=50",
        None,
    )
    .await;

    assert_eq!(code, StatusCode::BAD_REQUEST);
    assert!(payload["error"]
        .as_str()
        .unwrap_or_default()
        .contains("prefix cannot include parent directory traversals"));
}

#[tokio::test]
async fn docs_route_redirects_to_help_site() {
    let harness = UiHarness::new("ui-docs-redirect");

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/docs")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("dispatch request");

    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert_eq!(location, "/help/");
}

#[tokio::test]
async fn runtime_paths_endpoint_updates_shared_zone_and_default_destination() {
    let harness = UiHarness::new("ui-runtime-paths");

    let (update_code, update_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/runtime/paths",
        Some(json!({
            "shared_zone_path": "shared-zone-alt",
            "shared_zone_absolute": false,
            "default_user_destination_path": "exports-alt",
            "default_user_destination_absolute": false
        })),
    )
    .await;
    assert_eq!(update_code, StatusCode::OK);
    assert_eq!(update_payload["status"], "updated");

    let (runtime_code, runtime_payload) =
        call_json(&harness.app, Method::GET, "/api/runtime", None).await;
    assert_eq!(runtime_code, StatusCode::OK);

    let shared_zone = runtime_payload["shared_zone"].as_str().unwrap_or_default();
    let default_dst = runtime_payload["default_user_destination_dir"]
        .as_str()
        .unwrap_or_default();

    assert!(shared_zone.ends_with("shared-zone-alt"));
    assert!(default_dst.ends_with("exports-alt"));
    assert!(PathBuf::from(shared_zone).exists());
    assert!(PathBuf::from(default_dst).exists());
}

#[tokio::test]
async fn export_request_without_dst_uses_default_stage_filename() {
    let harness = UiHarness::new("ui-export-default-dst");
    let runtime = harness.runtime_state();

    fs::write(runtime.config.workspace.join("release.txt"), "v1\n").expect("write source file");

    let (stage_code, stage_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/export/request",
        Some(json!({
            "src": "release.txt",
            "auto_approve": true,
            "auto_approve_origin": "control_panel_user"
        })),
    )
    .await;

    assert_eq!(stage_code, StatusCode::OK);
    assert_eq!(stage_payload["status"], "staged");
    assert!(runtime.config.shared_zone_dir.join("release.txt").exists());
}

#[tokio::test]
async fn help_site_is_served_from_daemon() {
    let project = TestRuntimeDir::new("ui-help-static-project");
    let runtime = TestRuntimeDir::new("ui-help-static-runtime");
    init_layout(project.path(), Some(runtime.path()), None, true).expect("init runtime layout");

    let runtime_help_dist = project.path().join("docs-site/docs/.vitepress/dist");
    fs::create_dir_all(&runtime_help_dist).expect("create runtime docs dist");
    fs::write(
        runtime_help_dist.join("index.html"),
        "<html><body>Agent Ruler Documentation</body></html>",
    )
    .expect("write runtime docs index");

    let app = build_router(WebState {
        ruler_root: project.path().to_path_buf(),
        runtime_dir: Some(runtime.path().to_path_buf()),
    });

    let (status, body) = call_text(&app, Method::GET, "/help/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Agent Ruler Documentation"));
}

#[tokio::test]
async fn help_site_prefers_runtime_docs_bundle_when_present() {
    let project = TestRuntimeDir::new("ui-help-runtime-docs-project");
    let runtime = TestRuntimeDir::new("ui-help-runtime-docs-runtime");
    init_layout(project.path(), Some(runtime.path()), None, true).expect("init runtime layout");

    let runtime_help_dist = project.path().join("docs-site/docs/.vitepress/dist");
    fs::create_dir_all(&runtime_help_dist).expect("create runtime docs dist");
    fs::write(
        runtime_help_dist.join("index.html"),
        "<html><body>Runtime Help Bundle</body></html>",
    )
    .expect("write runtime docs index");

    let app = build_router(WebState {
        ruler_root: project.path().to_path_buf(),
        runtime_dir: Some(runtime.path().to_path_buf()),
    });

    let (status, body) = call_text(&app, Method::GET, "/help/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Runtime Help Bundle"));
}

/// Test: GET /api/capabilities returns a safe capabilities contract
///
/// This test verifies that the capabilities endpoint:
/// 1. Returns HTTP 200 OK
/// 2. Contains required fields (api_version, name, agent_safe_endpoints)
/// 3. Does NOT expose forbidden information:
///    - No filesystem paths
///    - No policy contents or toggles
///    - No receipt/approval internals
///    - No secrets
#[tokio::test]
async fn capabilities_endpoint_returns_safe_contract() {
    let harness = UiHarness::new("ui-capabilities");

    let (status, payload) = call_json(&harness.app, Method::GET, "/api/capabilities", None).await;

    assert_eq!(status, StatusCode::OK);

    // Verify required fields exist
    assert!(payload.get("api_version").is_some(), "api_version required");
    assert!(payload.get("name").is_some(), "name required");
    assert!(
        payload.get("agent_safe_endpoints").is_some(),
        "agent_safe_endpoints required"
    );
    assert!(
        payload.get("operator_only_endpoints").is_some(),
        "operator_only_endpoints required"
    );
    assert!(
        payload.get("excluded_data_classes").is_some(),
        "excluded_data_classes required"
    );
    assert!(
        payload.get("redaction_guarantees").is_some(),
        "redaction_guarantees required"
    );
    assert!(
        payload.get("tool_mapping").is_some(),
        "tool_mapping required"
    );

    // Verify the name is correct
    assert_eq!(payload["name"], "agent-ruler");

    // Verify agent_safe_endpoints has ALL documented endpoints
    let endpoints = &payload["agent_safe_endpoints"];
    let agent_safe_keys: std::collections::HashSet<&str> = endpoints
        .as_object()
        .map(|obj| obj.keys().map(|k| k.as_str()).collect())
        .unwrap_or_default();

    // Required agent-safe endpoints from documentation
    let required_agent_safe = [
        "status_feed",     // GET /api/status/feed
        "approval_wait",   // GET /api/approvals/:id/wait
        "tool_preflight",  // POST /api/openclaw/tool/preflight
        "export_request",  // POST /api/export/request
        "deliver_request", // POST /api/export/deliver/request
        "import_request",  // POST /api/import/request
    ];

    for endpoint in &required_agent_safe {
        assert!(
            agent_safe_keys.contains(endpoint),
            "agent_safe_endpoints must contain '{endpoint}'"
        );
    }

    assert_eq!(
        endpoints["tool_preflight"]["path"],
        "/api/runners/:id/tool/preflight"
    );

    // Verify each endpoint has required fields
    for (key, endpoint) in endpoints.as_object().unwrap_or(&serde_json::Map::new()) {
        assert!(
            endpoint.get("method").is_some(),
            "endpoint {key} must have method"
        );
        assert!(
            endpoint.get("path").is_some(),
            "endpoint {key} must have path"
        );
        assert!(
            endpoint.get("description").is_some(),
            "endpoint {key} must have description"
        );
    }

    // Verify operator_only_endpoints contains expected entries
    let operator_only = payload["operator_only_endpoints"]
        .as_array()
        .expect("operator_only_endpoints is array");
    let operator_only_str: Vec<&str> = operator_only.iter().filter_map(|v| v.as_str()).collect();

    // Required operator-only endpoints
    let required_operator_only = [
        "/api/status",
        "/api/runtime",
        "/api/policy",
        "/api/policy/toggles",
        "/api/approvals",
        "/api/approvals/:id/approve",
        "/api/approvals/:id/deny",
        "/api/approvals/approve-all",
        "/api/approvals/deny-all",
        "/api/reset-exec",
        "/api/reset-runtime",
        "/api/run/command",
    ];

    for endpoint in &required_operator_only {
        assert!(
            operator_only_str.contains(endpoint),
            "operator_only_endpoints must contain '{endpoint}'"
        );
    }

    // Verify agent_safe_endpoints does NOT contain operator-only endpoints
    for endpoint in &operator_only_str {
        // Check that the path is not in agent_safe_endpoints
        for (_, agent_endpoint) in endpoints.as_object().unwrap_or(&serde_json::Map::new()) {
            if let Some(path) = agent_endpoint.get("path").and_then(|p| p.as_str()) {
                assert!(
                    path != *endpoint,
                    "agent_safe_endpoints should not contain operator-only path: {endpoint}"
                );
            }
        }
    }

    // Verify excluded_data_classes contains expected entries
    let excluded = payload["excluded_data_classes"]
        .as_array()
        .expect("excluded_data_classes is array");
    let excluded_str: Vec<&str> = excluded.iter().filter_map(|v| v.as_str()).collect();

    assert!(
        excluded_str.contains(&"runtime_filesystem_paths"),
        "runtime_filesystem_paths must be excluded"
    );
    assert!(
        excluded_str.contains(&"policy_contents"),
        "policy_contents must be excluded"
    );
    assert!(
        excluded_str.contains(&"policy_toggles"),
        "policy_toggles must be excluded"
    );
    assert!(
        excluded_str.contains(&"secrets"),
        "secrets must be excluded"
    );
    assert!(
        excluded_str.contains(&"receipt_internals"),
        "receipt_internals must be excluded"
    );
    assert!(
        excluded_str.contains(&"approval_queue_internals"),
        "approval_queue_internals must be excluded"
    );

    // Verify the response does NOT contain any filesystem paths
    let payload_str = serde_json::to_string(&payload).expect("serialize payload");

    // Check that common path patterns are not present
    assert!(
        !payload_str.contains("/home/"),
        "Response should not contain /home/ paths"
    );
    assert!(
        !payload_str.contains("/.local/"),
        "Response should not contain /.local/ paths"
    );
    assert!(
        !payload_str.contains("policy.yaml"),
        "Response should not contain policy.yaml"
    );
    assert!(
        !payload_str.contains("approvals.json"),
        "Response should not contain approvals.json"
    );
    assert!(
        !payload_str.contains("receipts.jsonl"),
        "Response should not contain receipts.jsonl"
    );
    assert!(
        !payload_str.contains("state_dir"),
        "Response should not contain state_dir"
    );
    assert!(
        !payload_str.contains("runtime_root"),
        "Response should not contain runtime_root"
    );

    // Verify tool_mapping contains expected entries
    let tool_mapping = &payload["tool_mapping"];
    assert!(
        tool_mapping.get("agent_ruler_status_feed").is_some(),
        "tool_mapping must contain agent_ruler_status_feed"
    );
    assert!(
        tool_mapping.get("agent_ruler_wait_for_approval").is_some(),
        "tool_mapping must contain agent_ruler_wait_for_approval"
    );
    assert!(
        tool_mapping
            .get("agent_ruler_request_export_stage")
            .is_some(),
        "tool_mapping must contain agent_ruler_request_export_stage"
    );
    assert!(
        tool_mapping.get("agent_ruler_request_delivery").is_some(),
        "tool_mapping must contain agent_ruler_request_delivery"
    );
    assert!(
        tool_mapping.get("agent_ruler_request_import").is_some(),
        "tool_mapping must contain agent_ruler_request_import"
    );
    assert!(
        tool_mapping.get("before_tool_call").is_some(),
        "tool_mapping must contain before_tool_call"
    );
    assert_eq!(
        tool_mapping["before_tool_call"],
        "/api/runners/:id/tool/preflight"
    );
    assert_eq!(
        tool_mapping["before_tool_call_openclaw"],
        "/api/openclaw/tool/preflight"
    );
}

#[tokio::test]
async fn policy_toggles_endpoint_enforces_profile_locks() {
    let harness = UiHarness::new("ui-policy-toggles-locks");

    let (status, payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/policy/toggles",
        Some(json!({
            "profile": "strict"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["profile"], "strict");

    let (locked_status, locked_payload) = call_json(
        &harness.app,
        Method::POST,
        "/api/policy/toggles",
        Some(json!({
            "execution_deny_workspace_exec": false
        })),
    )
    .await;
    assert_eq!(locked_status, StatusCode::BAD_REQUEST);
    assert!(locked_payload["error"]
        .as_str()
        .unwrap_or_default()
        .contains("locks advanced"));
}
