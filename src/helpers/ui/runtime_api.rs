use std::collections::BTreeSet;
use std::fs;
use std::net::SocketAddr;
use std::path::{Component, Path};
use std::process::{Command, Stdio};
use std::time::Duration;

use axum::extract::{Path as AxPath, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use walkdir::WalkDir;

use crate::approvals::ApprovalStore;
use crate::claudecode_bridge::{
    ensure_generated_config as ensure_generated_claudecode_bridge_config,
    generated_config_path as claudecode_bridge_config_path,
    update_generated_config as update_generated_claudecode_bridge_config,
};
use crate::config::{save_config, RuntimeState};
use crate::helpers::channels::telegram_bridge_config::TelegramBridgeConfigPatch;
use crate::helpers::runners::introspection::{cached_runners_view, runner_view};
use crate::helpers::runtime::{resolve_ui_path_update, workspace_root_for_runner_id};
use crate::helpers::ui::payloads::{
    FileListItem, FileListQuery, ReceiptPage, ReceiptQuery, RuntimePathsPayload, RuntimePayload,
    StatusPayload, UiLogEventPayload, UiLogPage, UiLogQuery,
};
use crate::openclaw_bridge::{
    ensure_generated_config as ensure_generated_openclaw_bridge_config,
    generated_config_path as openclaw_bridge_config_path,
    update_generated_config as update_generated_openclaw_bridge_config, OpenClawBridgeConfigPatch,
};
use crate::opencode_bridge::{
    ensure_generated_config as ensure_generated_opencode_bridge_config,
    generated_config_path as opencode_bridge_config_path,
    update_generated_config as update_generated_opencode_bridge_config,
};
use crate::receipts::ReceiptStore;
use crate::runner::redacted_command_for_receipts;
use crate::runners::RunnerKind;
use crate::sessions::{SessionChannel, SessionListQuery, SessionStatus, SessionStore, SessionView};
use crate::staged_exports::{StagedExportState, StagedExportStore};
use crate::ui::{error_response, load_runtime_from_state, WebState};
use crate::ui_logs::UiLogStore;

const UI_EVENT_LOG_FILE_NAME: &str = "control-panel-events.jsonl";
const CLAUDECODE_TELEGRAM_CHANNEL_BRIDGE_PID_FILE_NAME: &str =
    "claudecode-telegram-channel-bridge.pid";
const OPENCODE_TELEGRAM_CHANNEL_BRIDGE_PID_FILE_NAME: &str = "opencode-telegram-channel-bridge.pid";
const CLAUDECODE_TELEGRAM_CHANNEL_BRIDGE_LOG_FILE_NAME: &str =
    "claudecode-telegram-channel-bridge.log";
const OPENCODE_TELEGRAM_CHANNEL_BRIDGE_LOG_FILE_NAME: &str = "opencode-telegram-channel-bridge.log";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunnerTelegramBridgeKind {
    Claudecode,
    Opencode,
}

impl RunnerTelegramBridgeKind {
    fn id(self) -> &'static str {
        match self {
            RunnerTelegramBridgeKind::Claudecode => "claudecode",
            RunnerTelegramBridgeKind::Opencode => "opencode",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            RunnerTelegramBridgeKind::Claudecode => "Claude Code",
            RunnerTelegramBridgeKind::Opencode => "OpenCode",
        }
    }

    fn pid_file_name(self) -> &'static str {
        match self {
            RunnerTelegramBridgeKind::Claudecode => {
                CLAUDECODE_TELEGRAM_CHANNEL_BRIDGE_PID_FILE_NAME
            }
            RunnerTelegramBridgeKind::Opencode => OPENCODE_TELEGRAM_CHANNEL_BRIDGE_PID_FILE_NAME,
        }
    }

    fn log_file_name(self) -> &'static str {
        match self {
            RunnerTelegramBridgeKind::Claudecode => {
                CLAUDECODE_TELEGRAM_CHANNEL_BRIDGE_LOG_FILE_NAME
            }
            RunnerTelegramBridgeKind::Opencode => OPENCODE_TELEGRAM_CHANNEL_BRIDGE_LOG_FILE_NAME,
        }
    }
}

fn runner_bridge_script_path(
    runtime: &RuntimeState,
    runner: RunnerTelegramBridgeKind,
) -> std::path::PathBuf {
    runtime
        .config
        .ruler_root
        .join("bridge")
        .join(runner.id())
        .join("channels")
        .join("telegram")
        .join("channel_bridge.py")
}

fn runner_bridge_pid_file(
    runtime: &RuntimeState,
    runner: RunnerTelegramBridgeKind,
) -> std::path::PathBuf {
    runtime
        .config
        .runtime_root
        .join("user_data")
        .join("logs")
        .join(runner.pid_file_name())
}

fn runner_bridge_log_file(
    runtime: &RuntimeState,
    runner: RunnerTelegramBridgeKind,
) -> std::path::PathBuf {
    runtime
        .config
        .runtime_root
        .join("user_data")
        .join("logs")
        .join(runner.log_file_name())
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
            return false;
        }
    }
    true
}

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(path)
}

fn file_tail(path: &Path, max_lines: usize) -> String {
    let Ok(raw) = fs::read_to_string(path) else {
        return String::new();
    };
    let lines = raw
        .lines()
        .rev()
        .take(max_lines)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    lines.join("\n").trim().to_string()
}

fn pid_cmdline(pid: u32) -> Option<String> {
    let raw = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    if raw.is_empty() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&raw)
            .replace('\0', " ")
            .trim()
            .to_string(),
    )
}

fn pid_matches_runner_bridge(pid: u32, runner: RunnerTelegramBridgeKind) -> bool {
    let Some(cmdline) = pid_cmdline(pid) else {
        return false;
    };
    cmdline.contains(&format!(
        "bridge/{}/channels/telegram/channel_bridge.py",
        runner.id()
    ))
}

fn pid_matches_any_runner_bridge(pid: u32) -> bool {
    pid_matches_runner_bridge(pid, RunnerTelegramBridgeKind::Claudecode)
        || pid_matches_runner_bridge(pid, RunnerTelegramBridgeKind::Opencode)
}

fn parse_lock_owner_pid(detail: &str) -> Option<u32> {
    let marker = "pid=";
    let start = detail.find(marker)? + marker.len();
    let remainder = &detail[start..];
    let digits = remainder
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u32>().ok()
}

fn runtime_runner_bridge_pids(
    runtime: &RuntimeState,
    runner: RunnerTelegramBridgeKind,
) -> Vec<u32> {
    let mut matches = BTreeSet::new();
    let runtime_root = runtime.config.runtime_root.to_string_lossy().to_string();
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(raw) = name.to_str() else {
            continue;
        };
        if !raw.as_bytes().iter().all(u8::is_ascii_digit) {
            continue;
        }
        let Ok(pid) = raw.parse::<u32>() else {
            continue;
        };
        if pid == 0 {
            continue;
        }
        if !pid_matches_runner_bridge(pid, runner) {
            continue;
        }
        let Some(cmdline) = pid_cmdline(pid) else {
            continue;
        };
        if !runtime_root.is_empty() && !cmdline.contains(&runtime_root) {
            continue;
        }
        matches.insert(pid);
    }
    matches.into_iter().collect()
}

fn terminate_pid(pid: u32) -> anyhow::Result<()> {
    let pid_str = pid.to_string();
    let _ = Command::new("kill").args(["-TERM", &pid_str]).status();
    for _ in 0..20 {
        if !process_exists(pid) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = Command::new("kill").args(["-KILL", &pid_str]).status();
    for _ in 0..10 {
        if !process_exists(pid) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(anyhow::anyhow!(
        "process {pid} is still alive after TERM/KILL attempts"
    ))
}

fn stop_runner_bridge_process(
    runtime: &RuntimeState,
    runner: RunnerTelegramBridgeKind,
) -> anyhow::Result<()> {
    let pid_file = runner_bridge_pid_file(runtime, runner);
    let mut pids_to_stop = BTreeSet::new();
    let raw = match fs::read_to_string(&pid_file) {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(anyhow::anyhow!("read {}: {}", pid_file.display(), err)),
    };

    if let Ok(pid) = raw.trim().parse::<u32>() {
        if pid > 0 && process_exists(pid) && pid_matches_runner_bridge(pid, runner) {
            pids_to_stop.insert(pid);
        }
    }
    for pid in runtime_runner_bridge_pids(runtime, runner) {
        pids_to_stop.insert(pid);
    }

    if pids_to_stop.is_empty() {
        let _ = remove_file_if_exists(&pid_file);
        return Ok(());
    }

    let mut failures: Vec<String> = Vec::new();
    for pid in pids_to_stop {
        if let Err(err) = terminate_pid(pid) {
            failures.push(format!("{pid}: {err}"));
        }
    }
    let _ = remove_file_if_exists(&pid_file);
    if !failures.is_empty() {
        return Err(anyhow::anyhow!(
            "unable to stop {} bridge process(es): {}",
            runner.display_name(),
            failures.join("; ")
        ));
    }
    Ok(())
}

fn start_runner_bridge_process(
    runtime: &RuntimeState,
    runner: RunnerTelegramBridgeKind,
    config_path: &Path,
) -> anyhow::Result<()> {
    let script_path = runner_bridge_script_path(runtime, runner);
    if !script_path.exists() {
        // Runtime tests create temp roots without embedded bridge assets; skip
        // process sync there and keep config update behavior intact.
        return Ok(());
    }

    let logs_dir = runtime.config.runtime_root.join("user_data").join("logs");
    fs::create_dir_all(&logs_dir)?;
    let log_path = runner_bridge_log_file(runtime, runner);
    let stdout = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let stderr = stdout.try_clone()?;
    let mut command = Command::new("python3");
    command
        .arg(script_path)
        .arg("--config")
        .arg(config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    if let Ok(current_exe) = std::env::current_exe() {
        command.env("AGENT_RULER_BIN", current_exe);
    }
    let mut child = command.spawn()?;

    let pid_file = runner_bridge_pid_file(runtime, runner);
    fs::write(&pid_file, format!("{}\n", child.id()))?;

    std::thread::sleep(Duration::from_millis(300));
    if let Some(status) = child.try_wait()? {
        let _ = remove_file_if_exists(&pid_file);
        let tail = file_tail(&log_path, 20);
        let detail = if tail.is_empty() {
            format!(
                "managed {} bridge exited during startup (status {}); check {}",
                runner.display_name(),
                status,
                log_path.display()
            )
        } else {
            format!(
                "managed {} bridge exited during startup (status {}); check {}. Recent log lines:\n{}",
                runner.display_name(),
                status,
                log_path.display(),
                tail
            )
        };
        return Err(anyhow::anyhow!("{}", detail));
    }
    Ok(())
}

fn sync_runner_bridge_process(
    runtime: &RuntimeState,
    runner: RunnerTelegramBridgeKind,
    config: &crate::helpers::channels::telegram_bridge_config::TelegramBridgeConfig,
    config_path: &Path,
    force_restart: bool,
) -> anyhow::Result<()> {
    if force_restart {
        stop_runner_bridge_process(runtime, runner)?;
    }
    if !config.enabled {
        stop_runner_bridge_process(runtime, runner)?;
        return Ok(());
    }
    if !config.token_configured() {
        stop_runner_bridge_process(runtime, runner)?;
        return Ok(());
    }

    if !force_restart {
        let pid_file = runner_bridge_pid_file(runtime, runner);
        if let Ok(raw) = fs::read_to_string(&pid_file) {
            if let Ok(pid) = raw.trim().parse::<u32>() {
                if process_exists(pid) && pid_matches_runner_bridge(pid, runner) {
                    return Ok(());
                }
            }
        }
    }

    if let Err(err) = start_runner_bridge_process(runtime, runner, config_path) {
        let detail = err.to_string();
        if detail.contains("another Agent Ruler Telegram bridge already owns this bot token") {
            if let Some(owner_pid) = parse_lock_owner_pid(&detail) {
                let runtime_root = runtime.config.runtime_root.to_string_lossy().to_string();
                if process_exists(owner_pid)
                    && pid_matches_any_runner_bridge(owner_pid)
                    && pid_cmdline(owner_pid)
                        .map(|cmdline| runtime_root.is_empty() || cmdline.contains(&runtime_root))
                        .unwrap_or(false)
                {
                    let _ = terminate_pid(owner_pid);
                }
            }
            let _ = stop_runner_bridge_process(runtime, RunnerTelegramBridgeKind::Claudecode);
            let _ = stop_runner_bridge_process(runtime, RunnerTelegramBridgeKind::Opencode);
            return start_runner_bridge_process(runtime, runner, config_path);
        }
        return Err(err);
    }
    Ok(())
}

fn selected_runner_bridge_kind(runtime: &RuntimeState) -> Option<RunnerTelegramBridgeKind> {
    match runtime.config.runner.as_ref().map(|runner| runner.kind) {
        Some(RunnerKind::Claudecode) => Some(RunnerTelegramBridgeKind::Claudecode),
        Some(RunnerKind::Opencode) => Some(RunnerTelegramBridgeKind::Opencode),
        _ => None,
    }
}

fn runner_bridge_running(runtime: &RuntimeState, runner: RunnerTelegramBridgeKind) -> bool {
    let pid_file = runner_bridge_pid_file(runtime, runner);
    if let Ok(raw) = fs::read_to_string(&pid_file) {
        if let Ok(pid) = raw.trim().parse::<u32>() {
            if pid > 0 && process_exists(pid) && pid_matches_runner_bridge(pid, runner) {
                return true;
            }
        }
    }
    !runtime_runner_bridge_pids(runtime, runner).is_empty()
}

fn active_runner_bridge_kinds(runtime: &RuntimeState) -> Vec<RunnerTelegramBridgeKind> {
    let mut active = Vec::new();
    for runner in [
        RunnerTelegramBridgeKind::Claudecode,
        RunnerTelegramBridgeKind::Opencode,
    ] {
        if runner_bridge_running(runtime, runner) {
            active.push(runner);
        }
    }
    active
}

fn telegram_bridge_status(runtime: &RuntimeState) -> (Option<String>, Vec<String>, bool) {
    let active = active_runner_bridge_kinds(runtime);
    let selected = selected_runner_bridge_kind(runtime);
    let active_ids = active
        .iter()
        .map(|runner| runner.id().to_string())
        .collect::<Vec<_>>();
    let active_primary = if active.len() == 1 {
        Some(active[0].id().to_string())
    } else {
        None
    };
    let in_sync = match selected {
        Some(selected_runner) => {
            // Selected runner with no active bridge is an idle/explicitly-stopped
            // state; treat it as in-sync. We only flag drift when a different
            // runner bridge is active (or more than one bridge is active).
            active.is_empty() || (active.len() == 1 && active[0] == selected_runner)
        }
        None => active.is_empty(),
    };
    (active_primary, active_ids, in_sync)
}

/// Keep managed Telegram bridge process ownership aligned with selected runner.
///
/// Exactly one conversational Telegram bridge is expected to own inbound updates
/// for a given runtime/bot token. On runner switch, we stop non-selected bridge
/// processes to prevent stale routing and start/sync the selected one.
pub fn sync_selected_runner_telegram_bridges(
    runtime: &RuntimeState,
    force_restart_claudecode: bool,
    force_restart_opencode: bool,
) -> anyhow::Result<()> {
    let claudecode_config = ensure_generated_claudecode_bridge_config(runtime)?;
    let claudecode_path = claudecode_bridge_config_path(runtime);
    let opencode_config = ensure_generated_opencode_bridge_config(runtime)?;
    let opencode_path = opencode_bridge_config_path(runtime);
    let selected = selected_runner_bridge_kind(runtime);

    // Stop non-selected runner bridge first to avoid Telegram token lock
    // contention.
    //
    // Keep selected runner bridge active when configured (enabled + token).
    // This ensures Telegram bridge status reflects actual conversational
    // availability right after setup/config updates.
    match selected {
        Some(RunnerTelegramBridgeKind::Claudecode) => {
            stop_runner_bridge_process(runtime, RunnerTelegramBridgeKind::Opencode)?;
            sync_runner_bridge_process(
                runtime,
                RunnerTelegramBridgeKind::Claudecode,
                &claudecode_config,
                &claudecode_path,
                force_restart_claudecode,
            )?;
        }
        Some(RunnerTelegramBridgeKind::Opencode) => {
            stop_runner_bridge_process(runtime, RunnerTelegramBridgeKind::Claudecode)?;
            sync_runner_bridge_process(
                runtime,
                RunnerTelegramBridgeKind::Opencode,
                &opencode_config,
                &opencode_path,
                force_restart_opencode,
            )?;
        }
        None => {
            stop_runner_bridge_process(runtime, RunnerTelegramBridgeKind::Claudecode)?;
            stop_runner_bridge_process(runtime, RunnerTelegramBridgeKind::Opencode)?;
        }
    }

    let selected = selected_runner_bridge_kind(runtime).map(|runner| runner.id().to_string());
    let (active_runner, active_runners, in_sync) = telegram_bridge_status(runtime);
    append_control_panel_log(
        runtime,
        if in_sync { "info" } else { "warning" },
        "runner-bridge-sync",
        if in_sync {
            "Runner Telegram bridge state is in sync"
        } else {
            "Runner Telegram bridge state is out of sync"
        },
        Some(serde_json::json!({
            "selected_runner": selected,
            "active_runner": active_runner,
            "active_runners": active_runners,
            "in_sync": in_sync,
        })),
    );

    Ok(())
}

#[derive(Debug, Default, Deserialize)]
pub struct RuntimeQuery {
    pub runner: Option<String>,
}

#[derive(Debug, Clone)]
struct RuntimeZoneRoots {
    workspace: std::path::PathBuf,
    shared: std::path::PathBuf,
    delivery: std::path::PathBuf,
    selected_runner: Option<RunnerKind>,
}

pub async fn api_status(
    State(state): State<WebState>,
    Query(query): Query<RuntimeQuery>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };
    let zones = match runtime_zone_roots(&runtime, query.runner.as_deref()) {
        Ok(zones) => zones,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err),
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
    let (bridge_active_runner, bridge_active_runners, bridge_in_sync) =
        telegram_bridge_status(&runtime);

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
        workspace: zones.workspace.to_string_lossy().to_string(),
        shared_zone: zones.shared.to_string_lossy().to_string(),
        state_dir: runtime.config.state_dir.to_string_lossy().to_string(),
        default_delivery_dir: zones.delivery.to_string_lossy().to_string(),
        default_user_destination_dir: zones.delivery.to_string_lossy().to_string(),
        ui_bind: runtime.config.ui_bind.clone(),
        allow_degraded_confinement: runtime.config.allow_degraded_confinement,
        ui_show_debug_tools: runtime.config.ui_show_debug_tools,
        approval_wait_timeout_secs: runtime.config.approval_wait_timeout_secs,
        selected_runner: zones.selected_runner.map(|runner| runner.id().to_string()),
        telegram_bridge_active_runner: bridge_active_runner,
        telegram_bridge_active_runners: bridge_active_runners,
        telegram_bridge_in_sync: bridge_in_sync,
    };
    (StatusCode::OK, Json(payload)).into_response()
}

pub async fn api_runtime(
    State(state): State<WebState>,
    Query(query): Query<RuntimeQuery>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };
    let zones = match runtime_zone_roots(&runtime, query.runner.as_deref()) {
        Ok(zones) => zones,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err),
    };
    let (bridge_active_runner, bridge_active_runners, bridge_in_sync) =
        telegram_bridge_status(&runtime);

    let payload = RuntimePayload {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        ruler_root: runtime.config.ruler_root.to_string_lossy().to_string(),
        runtime_root: runtime.config.runtime_root.to_string_lossy().to_string(),
        workspace: zones.workspace.to_string_lossy().to_string(),
        shared_zone: zones.shared.to_string_lossy().to_string(),
        state_dir: runtime.config.state_dir.to_string_lossy().to_string(),
        policy_file: runtime.config.policy_file.to_string_lossy().to_string(),
        receipts_file: runtime.config.receipts_file.to_string_lossy().to_string(),
        approvals_file: runtime.config.approvals_file.to_string_lossy().to_string(),
        staged_exports_file: runtime
            .config
            .staged_exports_file
            .to_string_lossy()
            .to_string(),
        default_delivery_dir: zones.delivery.to_string_lossy().to_string(),
        default_user_destination_dir: zones.delivery.to_string_lossy().to_string(),
        ui_bind: runtime.config.ui_bind.clone(),
        exec_layer_dir: runtime.config.exec_layer_dir.to_string_lossy().to_string(),
        quarantine_dir: runtime.config.quarantine_dir.to_string_lossy().to_string(),
        ui_show_debug_tools: runtime.config.ui_show_debug_tools,
        approval_wait_timeout_secs: runtime.config.approval_wait_timeout_secs,
        selected_runner: zones.selected_runner.map(|runner| runner.id().to_string()),
        telegram_bridge_active_runner: bridge_active_runner,
        telegram_bridge_active_runners: bridge_active_runners,
        telegram_bridge_in_sync: bridge_in_sync,
    };

    (StatusCode::OK, Json(payload)).into_response()
}

#[derive(Debug, Default, Deserialize)]
pub struct RunnersQuery {
    #[serde(default)]
    pub force: bool,
}

pub async fn api_runners(
    State(state): State<WebState>,
    Query(query): Query<RunnersQuery>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let (items, cached) = cached_runners_view(&runtime, query.force);
    (
        StatusCode::OK,
        Json(serde_json::json!({ "items": items, "cached": cached })),
    )
        .into_response()
}

pub async fn api_runner_get(
    State(state): State<WebState>,
    AxPath(id): AxPath<String>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let Some(kind) = RunnerKind::from_id(&id) else {
        return error_response(StatusCode::NOT_FOUND, format!("runner `{id}` not found"));
    };

    let item = runner_view(&runtime, kind);
    (StatusCode::OK, Json(item)).into_response()
}

#[derive(Debug, Default, Deserialize)]
pub struct SessionsQuery {
    pub runner: Option<String>,
    pub channel: Option<String>,
    pub status: Option<String>,
    pub activity: Option<String>,
    pub q: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramSessionResolvePayload {
    pub runner_kind: String,
    pub chat_id: String,
    pub thread_id: i64,
    pub message_anchor_id: Option<i64>,
    pub title: Option<String>,
    pub bind_session_id: Option<String>,
    pub bind_runner_session_key: Option<String>,
    #[serde(default)]
    pub prefer_existing_runner_session: bool,
}

#[derive(Debug, Deserialize)]
pub struct SessionRunnerKeyPayload {
    pub runner_session_key: String,
}

pub async fn api_sessions(
    State(state): State<WebState>,
    Query(query): Query<SessionsQuery>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let runner_kind = match normalize_runner_query(query.runner.as_deref()) {
        Ok(value) => value,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err),
    };
    let channel = match normalize_session_channel_query(query.channel.as_deref()) {
        Ok(value) => value,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err),
    };
    let status = match normalize_session_status_query(query.status.as_deref()) {
        Ok(value) => value,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err),
    };
    let recent_only = match normalize_session_activity_query(query.activity.as_deref()) {
        Ok(value) => value,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err),
    };
    let cursor = match parse_session_cursor(query.cursor.as_deref()) {
        Ok(value) => value,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err),
    };

    let store = SessionStore::new(SessionStore::default_path(&runtime.config.state_dir));
    let result = match store.page(&SessionListQuery {
        runner_kind,
        channel,
        status,
        recent_only,
        search: query.q.clone(),
        limit: query.limit.unwrap_or(10),
        cursor,
    }) {
        Ok(value) => value,
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };

    let items = result
        .items
        .iter()
        .map(SessionView::from)
        .collect::<Vec<_>>();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "items": items,
            "total": result.total,
            "limit": result.limit,
            "cursor": result.cursor.to_string(),
            "has_more": result.has_more,
            "next_cursor": result.next_cursor.map(|value| value.to_string()),
        })),
    )
        .into_response()
}

pub async fn api_session_get(
    State(state): State<WebState>,
    AxPath(id): AxPath<String>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let store = SessionStore::new(SessionStore::default_path(&runtime.config.state_dir));
    let Some(session) = (match store.get(&id) {
        Ok(value) => value,
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    }) else {
        return error_response(StatusCode::NOT_FOUND, format!("session `{id}` not found"));
    };

    (StatusCode::OK, Json(SessionView::from(&session))).into_response()
}

pub async fn api_session_telegram_resolve(
    State(state): State<WebState>,
    Json(payload): Json<TelegramSessionResolvePayload>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let Some(runner_kind) = RunnerKind::from_id(&payload.runner_kind) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "runner_kind must be one of: {}",
                [
                    RunnerKind::Openclaw,
                    RunnerKind::Claudecode,
                    RunnerKind::Opencode
                ]
                .into_iter()
                .map(RunnerKind::id)
                .collect::<Vec<_>>()
                .join("|")
            ),
        );
    };

    let store = SessionStore::new(SessionStore::default_path(&runtime.config.state_dir));
    let result = match store.resolve_telegram_thread(
        runner_kind,
        &payload.chat_id,
        payload.thread_id,
        payload.message_anchor_id,
        payload.title.as_deref(),
        payload.bind_session_id.as_deref(),
        payload.bind_runner_session_key.as_deref(),
        payload.prefer_existing_runner_session,
    ) {
        Ok(value) => value,
        Err(err) => {
            let message = err.to_string();
            let status = if message.contains("already bound to runner") {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            return error_response(status, message);
        }
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "created": result.created,
            "session": SessionView::from(&result.session),
        })),
    )
        .into_response()
}

pub async fn api_session_runner_key_set(
    State(state): State<WebState>,
    AxPath(id): AxPath<String>,
    Json(payload): Json<SessionRunnerKeyPayload>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let store = SessionStore::new(SessionStore::default_path(&runtime.config.state_dir));
    let session = match store.bind_runner_session_key(&id, &payload.runner_session_key) {
        Ok(value) => value,
        Err(err) => {
            let message = err.to_string();
            let status = if message.contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            return error_response(status, message);
        }
    };

    (StatusCode::OK, Json(SessionView::from(&session))).into_response()
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
    let bridge_config = match ensure_generated_openclaw_bridge_config(&runtime) {
        Ok(config) => config,
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };
    let bridge_config_path = openclaw_bridge_config_path(&runtime);
    let claudecode_bridge_config = match ensure_generated_claudecode_bridge_config(&runtime) {
        Ok(config) => config,
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };
    let claudecode_bridge_path = claudecode_bridge_config_path(&runtime);
    let opencode_bridge_config = match ensure_generated_opencode_bridge_config(&runtime) {
        Ok(config) => config,
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };
    let opencode_bridge_path = opencode_bridge_config_path(&runtime);

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
            "claudecode_bridge": {
                "config": claudecode_bridge_config.to_view(),
                "config_path": claudecode_bridge_path,
                "note": "Generated bridge config for Claude Code channel adapters. Token is masked in API responses."
            },
            "opencode_bridge": {
                "config": opencode_bridge_config.to_view(),
                "config_path": opencode_bridge_path,
                "note": "Generated bridge config for OpenCode channel adapters. Token is masked in API responses."
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
        match update_generated_openclaw_bridge_config(&runtime, &patch) {
            Ok(config) => config,
            Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
        }
    } else {
        match ensure_generated_openclaw_bridge_config(&runtime) {
            Ok(config) => config,
            Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
        }
    };
    let bridge_config_path = openclaw_bridge_config_path(&runtime);
    let claudecode_bridge_config = if let Some(bridge_payload) = payload.claudecode_bridge.as_ref()
    {
        let patch = TelegramBridgeConfigPatch {
            enabled: bridge_payload.enabled,
            answer_streaming_enabled: bridge_payload.answer_streaming_enabled,
            poll_interval_seconds: bridge_payload.poll_interval_seconds,
            decision_ttl_seconds: bridge_payload.decision_ttl_seconds,
            short_id_length: bridge_payload.short_id_length,
            state_file: bridge_payload.state_file.clone(),
            bot_token: bridge_payload.bot_token.clone(),
            // Runner bridges learn chat/thread bindings dynamically from inbound
            // Telegram commands; static chat_ids are intentionally cleared.
            chat_ids: Some(Vec::new()),
            allow_from: bridge_payload.allow_from.clone(),
        };
        match update_generated_claudecode_bridge_config(&runtime, &patch) {
            Ok(config) => config,
            Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
        }
    } else {
        match ensure_generated_claudecode_bridge_config(&runtime) {
            Ok(config) => config,
            Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
        }
    };
    let claudecode_bridge_path = claudecode_bridge_config_path(&runtime);
    let opencode_bridge_config = if let Some(bridge_payload) = payload.opencode_bridge.as_ref() {
        let patch = TelegramBridgeConfigPatch {
            enabled: bridge_payload.enabled,
            answer_streaming_enabled: bridge_payload.answer_streaming_enabled,
            poll_interval_seconds: bridge_payload.poll_interval_seconds,
            decision_ttl_seconds: bridge_payload.decision_ttl_seconds,
            short_id_length: bridge_payload.short_id_length,
            state_file: bridge_payload.state_file.clone(),
            bot_token: bridge_payload.bot_token.clone(),
            // Runner bridges learn chat/thread bindings dynamically from inbound
            // Telegram commands; static chat_ids are intentionally cleared.
            chat_ids: Some(Vec::new()),
            allow_from: bridge_payload.allow_from.clone(),
        };
        match update_generated_opencode_bridge_config(&runtime, &patch) {
            Ok(config) => config,
            Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
        }
    } else {
        match ensure_generated_opencode_bridge_config(&runtime) {
            Ok(config) => config,
            Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
        }
    };
    let opencode_bridge_path = opencode_bridge_config_path(&runtime);
    if let Err(err) = sync_selected_runner_telegram_bridges(
        &runtime,
        payload.claudecode_bridge.is_some(),
        payload.opencode_bridge.is_some(),
    ) {
        eprintln!("runner bridge diagnostics: unable to sync bridge process ownership: {err}");
    }

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
            "claudecode_bridge": {
                "config": claudecode_bridge_config.to_view(),
                "config_path": claudecode_bridge_path,
            },
            "opencode_bridge": {
                "config": opencode_bridge_config.to_view(),
                "config_path": opencode_bridge_path,
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

    if let Some(runner_filter) = normalize_runner_filter(query.runner.as_deref()) {
        items.retain(|item| runner_id_from_receipt(item).as_deref() == Some(&runner_filter));
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
    let include_details = query.include_details.unwrap_or(false);
    for item in &mut page_items {
        let original_command = if item.action.process.command.trim().is_empty() {
            item.action
                .metadata
                .get("argv")
                .cloned()
                .unwrap_or_default()
        } else {
            item.action.process.command.clone()
        };

        let command_summary = redact_receipt_command_line(&original_command);
        item.action.metadata.remove("argv");
        item.action
            .metadata
            .insert("_command_summary".to_string(), command_summary);

        if include_details {
            item.action.process.command = original_command;
        } else {
            item.action.process.command.clear();
            item.diff_summary = None;
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

    if let Some(runner_filter) = normalize_runner_filter(query.runner.as_deref()) {
        items.retain(|item| runner_id_from_ui_log(item).as_deref() == Some(&runner_filter));
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
    let zones = match runtime_zone_roots(&runtime, query.runner.as_deref()) {
        Ok(zones) => zones,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err),
    };

    let root = match query.zone.as_str() {
        "workspace" => zones.workspace,
        "shared" => zones.shared,
        "deliver" => zones.delivery,
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

    // Prioritize direct children so top-level files remain visible even when
    // deep trees contain many entries.
    let read_dir = match fs::read_dir(&scan_root) {
        Ok(entries) => entries,
        Err(_) => return (StatusCode::OK, Json(Vec::<FileListItem>::new())).into_response(),
    };
    for entry in read_dir.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if let Some(item) = build_file_list_item(&path, &root, &needle, dirs_only, &metadata) {
            items.push(item);
        }
    }

    if items.len() < limit {
        for entry in WalkDir::new(&scan_root)
            .min_depth(2)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let path = entry.path();
            let metadata = match entry.metadata() {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            if let Some(item) = build_file_list_item(path, &root, &needle, dirs_only, &metadata) {
                items.push(item);
            }
            if items.len() >= limit {
                break;
            }
        }
    }

    items.sort_by(|a, b| a.path.cmp(&b.path));
    if items.len() > limit {
        items.truncate(limit);
    }
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

fn build_file_list_item(
    path: &Path,
    root: &Path,
    needle: &str,
    dirs_only: bool,
    metadata: &std::fs::Metadata,
) -> Option<FileListItem> {
    let relative = path.strip_prefix(root).ok()?;
    let rel_text = relative.to_string_lossy().to_string();
    if !needle.is_empty() && !rel_text.to_ascii_lowercase().contains(needle) {
        return None;
    }
    if dirs_only && !metadata.is_dir() {
        return None;
    }

    let kind = if metadata.is_dir() {
        "dir"
    } else if metadata.is_file() {
        "file"
    } else {
        "other"
    };

    Some(FileListItem {
        path: rel_text,
        kind: kind.to_string(),
        bytes: if metadata.is_file() {
            metadata.len()
        } else {
            0
        },
    })
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

fn parse_session_cursor(value: Option<&str>) -> Result<usize, String> {
    let trimmed = value.map(str::trim).unwrap_or_default();
    if trimmed.is_empty() {
        return Ok(0);
    }
    trimmed
        .parse::<usize>()
        .map_err(|_| "cursor must be a non-negative integer".to_string())
}

fn normalize_session_channel_query(value: Option<&str>) -> Result<Option<SessionChannel>, String> {
    let trimmed = value.map(str::trim).unwrap_or_default();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("all") {
        return Ok(None);
    }
    SessionChannel::from_id(trimmed).map(Some).ok_or_else(|| {
        format!(
            "channel must be one of: {}",
            [
                SessionChannel::Telegram,
                SessionChannel::Tui,
                SessionChannel::Web,
                SessionChannel::Api
            ]
            .into_iter()
            .map(SessionChannel::id)
            .collect::<Vec<_>>()
            .join("|")
        )
    })
}

fn normalize_session_status_query(value: Option<&str>) -> Result<Option<SessionStatus>, String> {
    let trimmed = value.map(str::trim).unwrap_or_default();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("all") {
        return Ok(None);
    }
    SessionStatus::from_id(trimmed).map(Some).ok_or_else(|| {
        format!(
            "status must be one of: {}",
            [SessionStatus::Active, SessionStatus::Archived]
                .into_iter()
                .map(SessionStatus::id)
                .collect::<Vec<_>>()
                .join("|")
        )
    })
}

fn normalize_session_activity_query(value: Option<&str>) -> Result<bool, String> {
    let trimmed = value.map(str::trim).unwrap_or_default();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("all") {
        return Ok(false);
    }
    if trimmed.eq_ignore_ascii_case("recent") {
        return Ok(true);
    }
    Err("activity must be `all` or `recent`".to_string())
}

fn normalize_runner_filter(value: Option<&str>) -> Option<String> {
    let trimmed = value.map(str::trim).unwrap_or_default();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("all") {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

fn runner_id_from_receipt(receipt: &crate::model::Receipt) -> Option<String> {
    receipt
        .action
        .metadata
        .get("runner_id")
        .map(|value| value.trim().to_ascii_lowercase())
}

fn runner_id_from_ui_log(item: &crate::ui_logs::UiLogEntry) -> Option<String> {
    let details = item.details.as_ref()?;
    let runner = details.get("runner_id")?.as_str()?;
    let trimmed = runner.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

fn redact_receipt_command_line(raw: &str) -> String {
    let tokens = raw
        .split_whitespace()
        .filter(|token| !token.trim().is_empty())
        .map(|token| token.to_string())
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return String::new();
    }
    redacted_command_for_receipts(&tokens)
}

fn runtime_zone_roots(
    runtime: &RuntimeState,
    requested_runner: Option<&str>,
) -> Result<RuntimeZoneRoots, String> {
    let selected_runner = normalize_runner_query(requested_runner)?
        .or_else(|| runtime.config.runner.as_ref().map(|runner| runner.kind));
    let workspace =
        workspace_root_for_runner_id(runtime, requested_runner).map_err(|err| err.to_string())?;

    Ok(RuntimeZoneRoots {
        workspace,
        shared: runtime.config.shared_zone_dir.clone(),
        delivery: runtime.config.default_delivery_dir.clone(),
        selected_runner,
    })
}

fn normalize_runner_query(value: Option<&str>) -> Result<Option<RunnerKind>, String> {
    let trimmed = value.map(str::trim).unwrap_or_default();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("all")
        || trimmed.eq_ignore_ascii_case("current")
        || trimmed.eq_ignore_ascii_case("selected")
    {
        return Ok(None);
    }

    match RunnerKind::from_id(trimmed) {
        Some(kind) => Ok(Some(kind)),
        None => Err(format!(
            "runner must be one of: {}",
            [
                RunnerKind::Openclaw,
                RunnerKind::Claudecode,
                RunnerKind::Opencode
            ]
            .into_iter()
            .map(RunnerKind::id)
            .collect::<Vec<_>>()
            .join("|")
        )),
    }
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
