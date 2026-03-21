use crate::config::RuleDisposition;
use crate::model::{ApprovalRecord, DiffSummary, Receipt};
use crate::ui_logs::UiLogEntry;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Debug, Serialize)]
pub struct StatusPayload {
    pub app_version: String,
    pub profile: String,
    pub policy_version: String,
    pub policy_hash: String,
    pub pending_approvals: usize,
    pub receipts_count: usize,
    pub staged_count: usize,
    pub delivered_count: usize,
    pub runtime_root: String,
    pub workspace: String,
    pub shared_zone: String,
    pub state_dir: String,
    pub default_delivery_dir: String,
    pub default_user_destination_dir: String,
    pub ui_bind: String,
    pub allow_degraded_confinement: bool,
    pub ui_show_debug_tools: bool,
    pub approval_wait_timeout_secs: u64,
    pub selected_runner: Option<String>,
    pub telegram_bridge_active_runner: Option<String>,
    pub telegram_bridge_active_runners: Vec<String>,
    pub telegram_bridge_in_sync: bool,
}

#[derive(Debug, Serialize)]
pub struct RuntimePayload {
    pub app_version: String,
    pub ruler_root: String,
    pub runtime_root: String,
    pub workspace: String,
    pub shared_zone: String,
    pub state_dir: String,
    pub policy_file: String,
    pub receipts_file: String,
    pub approvals_file: String,
    pub staged_exports_file: String,
    pub default_delivery_dir: String,
    pub default_user_destination_dir: String,
    pub ui_bind: String,
    pub exec_layer_dir: String,
    pub quarantine_dir: String,
    pub ui_show_debug_tools: bool,
    pub approval_wait_timeout_secs: u64,
    pub selected_runner: Option<String>,
    pub telegram_bridge_active_runner: Option<String>,
    pub telegram_bridge_active_runners: Vec<String>,
    pub telegram_bridge_in_sync: bool,
}

#[derive(Debug, Deserialize)]
pub struct ReceiptQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub date: Option<String>,
    pub q: Option<String>,
    pub verdict: Option<String>,
    pub action: Option<String>,
    pub runner: Option<String>,
    pub include_details: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ExportPayload {
    pub src: String,
    pub dst: Option<String>,
    pub runner: Option<String>,
    pub bypass: Option<bool>,
    pub bypass_ack: Option<bool>,
    pub auto_approve: Option<bool>,
    pub auto_approve_origin: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeliverPayload {
    pub stage_ref: String,
    pub dst: Option<String>,
    pub runner: Option<String>,
    pub move_artifact: Option<bool>,
    pub bypass: Option<bool>,
    pub bypass_ack: Option<bool>,
    pub auto_approve: Option<bool>,
    pub auto_approve_origin: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImportPayload {
    pub src: String,
    pub dst: Option<String>,
    pub runner: Option<String>,
    pub bypass: Option<bool>,
    pub bypass_ack: Option<bool>,
    pub auto_approve: Option<bool>,
    pub auto_approve_origin: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TogglePayload {
    pub profile: Option<String>,
    pub create_custom_profile: Option<bool>,
    pub network_default_deny: Option<bool>,
    pub network_allowlist_hosts: Option<Vec<String>>,
    pub network_require_approval_for_post: Option<bool>,
    pub network_denylist_hosts: Option<Vec<String>>,
    pub network_invert_allowlist: Option<bool>,
    pub network_invert_denylist: Option<bool>,
    pub elevation_enabled: Option<bool>,
    pub elevation_require_operator_auth: Option<bool>,
    pub elevation_use_allowlist: Option<bool>,
    pub elevation_allowed_packages: Option<Vec<String>>,
    pub elevation_denied_packages: Option<Vec<String>>,
    pub require_shared_approval: Option<bool>,
    pub filesystem_workspace: Option<RuleDisposition>,
    pub filesystem_user_data: Option<RuleDisposition>,
    pub filesystem_shared: Option<RuleDisposition>,
    pub filesystem_secrets: Option<RuleDisposition>,
    pub execution_deny_workspace_exec: Option<bool>,
    pub execution_deny_tmp_exec: Option<bool>,
    pub execution_quarantine_on_download_exec_chain: Option<bool>,
    pub execution_allowed_exec_prefixes: Option<Vec<String>>,
    pub persistence_deny_autostart: Option<bool>,
    pub persistence_approval_paths: Option<Vec<String>>,
    pub persistence_deny_paths: Option<Vec<String>>,
    pub safeguards_mass_delete_threshold: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct RuntimePathsPayload {
    pub shared_zone_path: Option<String>,
    pub shared_zone_absolute: Option<bool>,
    pub default_user_destination_path: Option<String>,
    pub default_user_destination_absolute: Option<bool>,
    pub ui_bind: Option<String>,
    pub ui_show_debug_tools: Option<bool>,
    pub allow_degraded_confinement: Option<bool>,
    pub approval_wait_timeout_secs: Option<u64>,
    pub openclaw_bridge: Option<OpenClawBridgePayload>,
    pub claudecode_bridge: Option<TelegramBridgePayload>,
    pub opencode_bridge: Option<TelegramBridgePayload>,
}

#[derive(Debug, Deserialize)]
pub struct OpenClawBridgePayload {
    pub poll_interval_seconds: Option<u64>,
    pub decision_ttl_seconds: Option<u64>,
    pub short_id_length: Option<u64>,
    pub inbound_bind: Option<String>,
    pub state_file: Option<String>,
    pub openclaw_bin: Option<String>,
    pub agent_ruler_bin: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramBridgePayload {
    pub enabled: Option<bool>,
    pub answer_streaming_enabled: Option<bool>,
    pub poll_interval_seconds: Option<u64>,
    pub decision_ttl_seconds: Option<u64>,
    pub short_id_length: Option<u64>,
    pub state_file: Option<String>,
    pub bot_token: Option<String>,
    pub chat_ids: Option<Vec<String>>,
    pub allow_from: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct ResetRuntimePayload {
    pub keep_config: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct RunScriptPayload {
    pub script: String,
}

#[derive(Debug, Deserialize)]
pub struct RunCommandPayload {
    pub cmd: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateApplyPayload {
    pub version: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct OpenClawToolContextPayload {
    pub agent_id: Option<String>,
    pub session_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenClawToolPreflightPayload {
    pub tool_name: String,
    #[serde(default)]
    pub params: JsonValue,
    #[serde(default)]
    pub context: OpenClawToolContextPayload,
}

#[derive(Debug, Serialize)]
pub struct BulkApprovalResult {
    pub updated: Vec<String>,
    pub failed: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ApprovalView {
    #[serde(flatten)]
    pub approval: ApprovalRecord,
    pub why: String,
    pub resolved_src: Option<String>,
    pub resolved_dst: Option<String>,
    pub diff_summary: Option<DiffSummary>,
}

#[derive(Debug, Serialize)]
pub struct ReceiptPage {
    pub items: Vec<Receipt>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
    pub has_more: bool,
}

#[derive(Debug, Deserialize)]
pub struct UiLogQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub q: Option<String>,
    pub level: Option<String>,
    pub source: Option<String>,
    pub runner: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ApprovalQuery {
    pub runner: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UiLogPage {
    pub items: Vec<UiLogEntry>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
    pub has_more: bool,
}

#[derive(Debug, Deserialize)]
pub struct UiLogEventPayload {
    pub level: String,
    pub source: String,
    pub message: String,
    pub details: Option<JsonValue>,
}

#[derive(Debug, Deserialize)]
pub struct FileListQuery {
    pub zone: String,
    pub runner: Option<String>,
    pub q: Option<String>,
    pub limit: Option<usize>,
    pub prefix: Option<String>,
    pub dirs_only: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ApprovalWaitQuery {
    pub timeout: Option<u64>,
    pub poll_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct StatusFeedQuery {
    pub limit: Option<usize>,
    pub include_resolved: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct FileListItem {
    pub path: String,
    pub kind: String,
    pub bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct PolicyProfile {
    pub id: String,
    pub label: String,
    pub description: String,
    pub details: Vec<String>,
    pub allow_rule_customization: bool,
    pub allow_elevation_customization: bool,
    pub allow_network_customization: bool,
    pub allow_domain_customization: bool,
    pub can_create_custom_profile: bool,
}

#[derive(Debug, Serialize)]
pub struct RedactedStatusEvent {
    pub approval_id: String,
    pub verdict: String,
    pub reason_code: String,
    pub category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_hint: Option<String>,
    pub target_classification: String,
    pub guidance: String,
    pub open_in_webui: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct ApprovalWaitResponse {
    #[serde(flatten)]
    pub event: RedactedStatusEvent,
    pub resolved: bool,
    pub timeout: bool,
}
