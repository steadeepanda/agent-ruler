//! Shared Telegram bridge config normalization helpers.
//!
//! Extracted from `src/telegram_bridge.rs` to keep runner-specific bridge
//! modules (`claudecode_bridge.rs`, `opencode_bridge.rs`) small and explicit.
//! The helper centralizes deterministic config generation and masking behavior.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::config::RuntimeState;
use crate::runners::RunnerKind;

const DEFAULT_ENABLED: bool = false;
const DEFAULT_POLL_INTERVAL_SECONDS: u64 = 8;
const DEFAULT_DECISION_TTL_SECONDS: u64 = 7200;
const DEFAULT_SHORT_ID_LENGTH: u64 = 6;
const DEFAULT_UI_PORT: u16 = 4622;

/// Runner-scoped defaults for generated Telegram bridge config.
#[derive(Debug, Clone, Copy)]
pub struct TelegramBridgeConfigDefaults {
    pub generated_config_file_name: &'static str,
    pub default_state_file_name: &'static str,
    pub runner_kind: RunnerKind,
}

/// Normalized Telegram bridge config for runtime startup.
#[derive(Debug, Clone)]
pub struct TelegramBridgeConfig {
    pub runner_kind: String,
    pub enabled: bool,
    pub answer_streaming_enabled: bool,
    pub ruler_url: String,
    pub public_base_url: String,
    pub poll_interval_seconds: u64,
    pub decision_ttl_seconds: u64,
    pub short_id_length: u64,
    pub state_file: String,
    pub runtime_dir: String,
    pub bot_token: String,
    pub chat_ids: Vec<String>,
    pub allow_from: Vec<String>,
}

/// Redacted Telegram bridge config view for UI responses.
#[derive(Debug, Clone, Serialize)]
pub struct TelegramBridgeConfigView {
    pub runner_kind: String,
    pub enabled: bool,
    pub answer_streaming_enabled: bool,
    pub ruler_url: String,
    pub public_base_url: String,
    pub poll_interval_seconds: u64,
    pub decision_ttl_seconds: u64,
    pub short_id_length: u64,
    pub state_file: String,
    pub runtime_dir: String,
    pub bot_token_configured: bool,
    pub bot_token_masked: String,
    pub chat_ids: Vec<String>,
    pub allow_from: Vec<String>,
}

/// Patch payload for operator updates to Telegram bridge config.
#[derive(Debug, Clone, Deserialize)]
pub struct TelegramBridgeConfigPatch {
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

impl TelegramBridgeConfig {
    pub fn token_configured(&self) -> bool {
        !self.bot_token.trim().is_empty()
    }

    pub fn to_view(&self) -> TelegramBridgeConfigView {
        TelegramBridgeConfigView {
            runner_kind: self.runner_kind.clone(),
            enabled: self.enabled,
            answer_streaming_enabled: self.answer_streaming_enabled,
            ruler_url: self.ruler_url.clone(),
            public_base_url: self.public_base_url.clone(),
            poll_interval_seconds: self.poll_interval_seconds,
            decision_ttl_seconds: self.decision_ttl_seconds,
            short_id_length: self.short_id_length,
            state_file: self.state_file.clone(),
            runtime_dir: self.runtime_dir.clone(),
            bot_token_configured: self.token_configured(),
            bot_token_masked: mask_token(&self.bot_token),
            chat_ids: self.chat_ids.clone(),
            allow_from: self.allow_from.clone(),
        }
    }
}

/// Resolve generated Telegram config path for a specific runner bridge.
pub fn generated_config_path(
    runtime: &RuntimeState,
    defaults: TelegramBridgeConfigDefaults,
) -> PathBuf {
    runtime
        .config
        .runtime_root
        .join("user_data")
        .join("bridge")
        .join(defaults.generated_config_file_name)
}

/// Ensure generated Telegram config exists and is normalized.
pub fn ensure_generated_config(
    runtime: &RuntimeState,
    defaults: TelegramBridgeConfigDefaults,
) -> Result<TelegramBridgeConfig> {
    let path = generated_config_path(runtime, defaults);
    let mut root = read_config_object(&path)?;
    let normalized = normalize_config(runtime, defaults, &mut root);
    write_config_object(&path, &root)?;
    Ok(normalized)
}

/// Apply operator patch to generated Telegram config.
pub fn update_generated_config(
    runtime: &RuntimeState,
    defaults: TelegramBridgeConfigDefaults,
    patch: &TelegramBridgeConfigPatch,
) -> Result<TelegramBridgeConfig> {
    let path = generated_config_path(runtime, defaults);
    let mut root = read_config_object(&path)?;
    let _ = normalize_config(runtime, defaults, &mut root);

    if let Some(value) = patch.enabled {
        root.insert("enabled".to_string(), Value::Bool(value));
    }
    if let Some(value) = patch.answer_streaming_enabled {
        root.insert("answer_streaming_enabled".to_string(), Value::Bool(value));
    }
    if let Some(value) = patch.poll_interval_seconds {
        root.insert("poll_interval_seconds".to_string(), Value::from(value));
    }
    if let Some(value) = patch.decision_ttl_seconds {
        root.insert("decision_ttl_seconds".to_string(), Value::from(value));
    }
    if let Some(value) = patch.short_id_length {
        root.insert("short_id_length".to_string(), Value::from(value));
    }
    if let Some(value) = patch.state_file.as_ref() {
        root.insert(
            "state_file".to_string(),
            Value::String(require_non_empty("state_file", value)?),
        );
    }
    if let Some(value) = patch.bot_token.as_ref() {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            validate_bot_token(trimmed)?;
        }
        root.insert("bot_token".to_string(), Value::String(trimmed.to_string()));
    }
    if let Some(values) = patch.chat_ids.as_ref() {
        validate_chat_targets(values)?;
        root.insert("chat_ids".to_string(), normalize_string_array_value(values));
    }
    if let Some(values) = patch.allow_from.as_ref() {
        root.insert(
            "allow_from".to_string(),
            normalize_string_array_value(values),
        );
    }

    let normalized = normalize_config(runtime, defaults, &mut root);
    write_config_object(&path, &root)?;
    Ok(normalized)
}

fn normalize_config(
    runtime: &RuntimeState,
    defaults: TelegramBridgeConfigDefaults,
    root: &mut Map<String, Value>,
) -> TelegramBridgeConfig {
    root.insert(
        "runner_kind".to_string(),
        Value::String(defaults.runner_kind.id().to_string()),
    );
    let (ruler_url, public_base_url) = bridge_base_urls(&runtime.config.ui_bind);
    root.insert("ruler_url".to_string(), Value::String(ruler_url.clone()));
    root.insert(
        "public_base_url".to_string(),
        Value::String(public_base_url.clone()),
    );

    let enabled = normalize_bool(root, "enabled", DEFAULT_ENABLED);
    let answer_streaming_enabled = normalize_bool(root, "answer_streaming_enabled", true);
    let poll_interval_seconds = normalize_u64(
        root,
        "poll_interval_seconds",
        DEFAULT_POLL_INTERVAL_SECONDS,
        1,
        300,
    );
    let decision_ttl_seconds = normalize_u64(
        root,
        "decision_ttl_seconds",
        DEFAULT_DECISION_TTL_SECONDS,
        60,
        604_800,
    );
    let short_id_length = normalize_u64(root, "short_id_length", DEFAULT_SHORT_ID_LENGTH, 4, 10);
    let state_file = normalize_string(
        root,
        "state_file",
        &runtime
            .config
            .runtime_root
            .join("user_data")
            .join("bridge")
            .join(defaults.default_state_file_name)
            .to_string_lossy(),
    );
    let runtime_dir = normalize_string(
        root,
        "runtime_dir",
        &runtime.config.runtime_root.to_string_lossy(),
    );
    let bot_token = normalize_bot_token(root, "bot_token");
    let chat_ids = normalize_string_array(root, "chat_ids");
    let allow_from = normalize_string_array(root, "allow_from");

    TelegramBridgeConfig {
        runner_kind: defaults.runner_kind.id().to_string(),
        enabled,
        answer_streaming_enabled,
        ruler_url,
        public_base_url,
        poll_interval_seconds,
        decision_ttl_seconds,
        short_id_length,
        state_file,
        runtime_dir,
        bot_token,
        chat_ids,
        allow_from,
    }
}

fn read_config_object(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }

    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let parsed: Value =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    let Some(root) = parsed.as_object() else {
        return Err(anyhow!(
            "generated telegram bridge config root must be a JSON object: {}",
            path.display()
        ));
    };
    Ok(root.clone())
}

fn write_config_object(path: &Path, root: &Map<String, Value>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let payload = serde_json::to_string_pretty(root).context("serialize telegram bridge config")?;
    fs::write(path, payload).with_context(|| format!("write {}", path.display()))
}

fn require_non_empty(name: &str, raw: &str) -> Result<String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(anyhow!("{name} cannot be empty"));
    }
    Ok(value.to_string())
}

fn normalize_bool(root: &mut Map<String, Value>, key: &str, fallback: bool) -> bool {
    let value = root.get(key).and_then(Value::as_bool).unwrap_or(fallback);
    root.insert(key.to_string(), Value::Bool(value));
    value
}

fn normalize_string(root: &mut Map<String, Value>, key: &str, fallback: &str) -> String {
    let value = root
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or(fallback)
        .to_string();
    root.insert(key.to_string(), Value::String(value.clone()));
    value
}

fn normalize_string_array_value(values: &[String]) -> Value {
    Value::Array(
        unique_trimmed(values)
            .into_iter()
            .map(Value::String)
            .collect::<Vec<_>>(),
    )
}

fn normalize_string_array(root: &mut Map<String, Value>, key: &str) -> Vec<String> {
    let current = root
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let normalized = unique_trimmed(
        &current
            .iter()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>(),
    );
    root.insert(
        key.to_string(),
        Value::Array(
            normalized
                .iter()
                .cloned()
                .map(Value::String)
                .collect::<Vec<_>>(),
        ),
    );
    normalized
}

fn unique_trimmed(values: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        if out.iter().any(|existing| existing == trimmed) {
            continue;
        }
        out.push(trimmed.to_string());
    }
    out
}

fn validate_chat_targets(values: &[String]) -> Result<()> {
    for raw in values {
        let value = raw.trim();
        if value.is_empty() {
            continue;
        }
        let Some((chat_id, thread_id)) = value.split_once('#') else {
            continue;
        };
        if chat_id.trim().is_empty() {
            return Err(anyhow!(
                "invalid chat_ids entry `{value}`: missing chat id before `#`"
            ));
        }
        let thread_id = thread_id.trim();
        if thread_id.is_empty() {
            return Err(anyhow!(
                "invalid chat_ids entry `{value}`: missing thread id after `#`"
            ));
        }
        let parsed = thread_id.parse::<u64>().map_err(|_| {
            anyhow!("invalid chat_ids entry `{value}`: thread id must be a positive integer")
        })?;
        if parsed == 0 {
            return Err(anyhow!(
                "invalid chat_ids entry `{value}`: thread id must be greater than zero"
            ));
        }
    }
    Ok(())
}

fn normalize_bot_token(root: &mut Map<String, Value>, key: &str) -> String {
    let raw = normalize_string(root, key, "");
    if raw.is_empty() {
        return raw;
    }
    if validate_bot_token(&raw).is_ok() {
        return raw;
    }
    root.insert(key.to_string(), Value::String(String::new()));
    String::new()
}

fn validate_bot_token(token: &str) -> Result<()> {
    if !looks_like_telegram_bot_token(token) {
        return Err(anyhow!(
            "bot_token must match Telegram format `<bot-id>:<token>`"
        ));
    }
    Ok(())
}

fn looks_like_telegram_bot_token(token: &str) -> bool {
    if token.chars().any(char::is_whitespace) {
        return false;
    }
    let Some((bot_id, secret)) = token.split_once(':') else {
        return false;
    };
    if bot_id.is_empty() || secret.is_empty() {
        return false;
    }
    if !bot_id.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }
    if secret.len() < 20 {
        return false;
    }
    secret
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn normalize_u64(
    root: &mut Map<String, Value>,
    key: &str,
    fallback: u64,
    min: u64,
    max: u64,
) -> u64 {
    let parsed = root.get(key).and_then(parse_u64).unwrap_or(fallback);
    let clamped = parsed.clamp(min, max);
    root.insert(key.to_string(), Value::from(clamped));
    clamped
}

fn parse_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(raw) => raw.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn bridge_base_urls(bind: &str) -> (String, String) {
    let port = parse_bind_port(bind.trim());
    bridge_base_urls_with_tailscale(port, detect_tailscale_ipv4())
}

fn bridge_base_urls_with_tailscale(port: u16, tailscale_ip: Option<String>) -> (String, String) {
    let local_bind = format!("127.0.0.1:{port}");
    let public_bind = tailscale_ip
        .map(|ip| format!("{ip}:{port}"))
        .unwrap_or_else(|| local_bind.clone());
    (
        format!("http://{local_bind}"),
        format!("http://{public_bind}"),
    )
}

fn parse_bind_port(bind: &str) -> u16 {
    bind.rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
        .unwrap_or(DEFAULT_UI_PORT)
}

fn detect_tailscale_ipv4() -> Option<String> {
    let output = Command::new("tailscale").args(["ip", "-4"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

fn mask_token(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let visible = trimmed.chars().rev().take(4).collect::<String>();
    let suffix = visible.chars().rev().collect::<String>();
    format!("***{suffix}")
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, Value};

    use super::looks_like_telegram_bot_token;
    use super::mask_token;
    use super::normalize_bot_token;
    use super::unique_trimmed;
    use super::validate_chat_targets;

    #[test]
    fn mask_token_handles_empty_values() {
        assert_eq!(mask_token(""), "");
        assert_eq!(mask_token("   "), "");
    }

    #[test]
    fn mask_token_preserves_last_four_chars() {
        assert_eq!(mask_token("abcdef1234"), "***1234");
        assert_eq!(mask_token("123"), "***123");
    }

    #[test]
    fn unique_trimmed_deduplicates_values() {
        let values = vec![
            " 1001 ".to_string(),
            "".to_string(),
            "1002".to_string(),
            "1001".to_string(),
        ];
        assert_eq!(unique_trimmed(&values), vec!["1001", "1002"]);
    }

    #[test]
    fn validate_chat_targets_accepts_plain_and_thread_targets() {
        let values = vec!["-100123456789".to_string(), "-100123456789#42".to_string()];
        assert!(validate_chat_targets(&values).is_ok());
    }

    #[test]
    fn validate_chat_targets_rejects_invalid_thread_target() {
        let values = vec!["-100123456789#bad".to_string()];
        let err = validate_chat_targets(&values).expect_err("invalid thread id should fail");
        assert!(err
            .to_string()
            .contains("thread id must be a positive integer"));
    }

    #[test]
    fn bot_token_validation_accepts_telegram_shape() {
        assert!(looks_like_telegram_bot_token(
            "123456789:AAF85-WDxcpVVQM4tjDsYhLIK2HnYcSo4QQ"
        ));
    }

    #[test]
    fn bot_token_validation_rejects_shell_text_or_whitespace() {
        assert!(!looks_like_telegram_bot_token(
            "panda@host:~$ agent-ruler run -- claude"
        ));
        assert!(!looks_like_telegram_bot_token(
            "123456789:token with spaces"
        ));
    }

    #[test]
    fn normalize_bot_token_clears_corrupted_preflight_error_blob() {
        let mut root = Map::new();
        root.insert(
            "bot_token".to_string(),
            Value::String("panda@panda-VMware:~/Documents/Agent Ruler$ agent-ruler run -- claude error: Claude Code preflight API probe failed at http://100.89.186.26:4622 for /api/claudecode/tool/preflight: connect to preflight api probe target [::1]:4622".to_string()),
        );

        let normalized = normalize_bot_token(&mut root, "bot_token");
        assert!(normalized.is_empty(), "invalid bot token should be cleared");
        assert_eq!(
            root.get("bot_token").and_then(Value::as_str),
            Some(""),
            "stored bot token should be sanitized to empty string"
        );
    }
}
