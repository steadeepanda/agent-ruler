//! Runtime-generated OpenClaw approval bridge configuration helpers.
//!
//! This module owns the generated bridge config under the runtime directory so
//! both CLI and UI read/write one canonical file.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::RuntimeState;

pub const GENERATED_CONFIG_FILE_NAME: &str = "openclaw-channel-bridge.generated.json";

const DEFAULT_POLL_INTERVAL_SECONDS: u64 = 8;
const DEFAULT_DECISION_TTL_SECONDS: u64 = 7200;
const DEFAULT_SHORT_ID_LENGTH: u64 = 6;
const DEFAULT_INBOUND_BIND: &str = "127.0.0.1:4661";
const DEFAULT_OPENCLAW_BIN: &str = "openclaw";
const DEFAULT_AGENT_RULER_BIN: &str = "agent-ruler";
const DEFAULT_UI_PORT: u16 = 4622;

#[derive(Debug, Clone, Serialize)]
pub struct OpenClawBridgeConfigView {
    pub ruler_url: String,
    pub public_base_url: String,
    pub poll_interval_seconds: u64,
    pub decision_ttl_seconds: u64,
    pub short_id_length: u64,
    pub inbound_bind: String,
    pub state_file: String,
    pub openclaw_bin: String,
    pub agent_ruler_bin: String,
    pub runtime_dir: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenClawBridgeConfigPatch {
    pub poll_interval_seconds: Option<u64>,
    pub decision_ttl_seconds: Option<u64>,
    pub short_id_length: Option<u64>,
    pub inbound_bind: Option<String>,
    pub state_file: Option<String>,
    pub openclaw_bin: Option<String>,
    pub agent_ruler_bin: Option<String>,
}

pub fn generated_config_path(runtime: &RuntimeState) -> PathBuf {
    runtime
        .config
        .runtime_root
        .join("user_data")
        .join("bridge")
        .join(GENERATED_CONFIG_FILE_NAME)
}

pub fn ensure_generated_config(runtime: &RuntimeState) -> Result<OpenClawBridgeConfigView> {
    let path = generated_config_path(runtime);
    let mut root = read_config_object(&path)?;
    let normalized = normalize_config(runtime, &mut root);
    write_config_object(&path, &root)?;
    Ok(normalized)
}

pub fn update_generated_config(
    runtime: &RuntimeState,
    patch: &OpenClawBridgeConfigPatch,
) -> Result<OpenClawBridgeConfigView> {
    let path = generated_config_path(runtime);
    let mut root = read_config_object(&path)?;
    let _ = normalize_config(runtime, &mut root);

    if let Some(value) = patch.poll_interval_seconds {
        root.insert("poll_interval_seconds".to_string(), Value::from(value));
    }
    if let Some(value) = patch.decision_ttl_seconds {
        root.insert("decision_ttl_seconds".to_string(), Value::from(value));
    }
    if let Some(value) = patch.short_id_length {
        root.insert("short_id_length".to_string(), Value::from(value));
    }
    if let Some(value) = patch.inbound_bind.as_ref() {
        root.insert(
            "inbound_bind".to_string(),
            Value::String(require_non_empty("inbound_bind", value)?),
        );
    }
    if let Some(value) = patch.state_file.as_ref() {
        root.insert(
            "state_file".to_string(),
            Value::String(require_non_empty("state_file", value)?),
        );
    }
    if let Some(value) = patch.openclaw_bin.as_ref() {
        root.insert(
            "openclaw_bin".to_string(),
            Value::String(require_non_empty("openclaw_bin", value)?),
        );
    }
    if let Some(value) = patch.agent_ruler_bin.as_ref() {
        root.insert(
            "agent_ruler_bin".to_string(),
            Value::String(require_non_empty("agent_ruler_bin", value)?),
        );
    }

    let normalized = normalize_config(runtime, &mut root);
    write_config_object(&path, &root)?;
    Ok(normalized)
}

fn normalize_config(
    runtime: &RuntimeState,
    root: &mut Map<String, Value>,
) -> OpenClawBridgeConfigView {
    let (ruler_url, public_base_url) = bridge_base_urls(&runtime.config.ui_bind);
    root.insert("ruler_url".to_string(), Value::String(ruler_url.clone()));
    root.insert(
        "public_base_url".to_string(),
        Value::String(public_base_url.clone()),
    );

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
    let inbound_bind = normalize_string(root, "inbound_bind", DEFAULT_INBOUND_BIND);
    let state_file = normalize_string(
        root,
        "state_file",
        &runtime
            .config
            .runtime_root
            .join("user_data")
            .join("bridge")
            .join("openclaw-state.json")
            .to_string_lossy(),
    );
    let openclaw_bin = normalize_string(root, "openclaw_bin", DEFAULT_OPENCLAW_BIN);
    let agent_ruler_bin = normalize_string(root, "agent_ruler_bin", DEFAULT_AGENT_RULER_BIN);
    let runtime_dir = normalize_string(
        root,
        "runtime_dir",
        &runtime.config.runtime_root.to_string_lossy(),
    );

    OpenClawBridgeConfigView {
        ruler_url,
        public_base_url,
        poll_interval_seconds,
        decision_ttl_seconds,
        short_id_length,
        inbound_bind,
        state_file,
        openclaw_bin,
        agent_ruler_bin,
        runtime_dir,
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
            "generated bridge config root must be a JSON object: {}",
            path.display()
        ));
    };
    Ok(root.clone())
}

fn write_config_object(path: &Path, root: &Map<String, Value>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let payload = serde_json::to_string_pretty(root).context("serialize bridge config")?;
    fs::write(path, payload).with_context(|| format!("write {}", path.display()))
}

fn require_non_empty(name: &str, raw: &str) -> Result<String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(anyhow!("{name} cannot be empty"));
    }
    Ok(value.to_string())
}

fn normalize_string(root: &mut Map<String, Value>, key: &str, fallback: &str) -> String {
    let value = root
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .unwrap_or(fallback)
        .to_string();
    root.insert(key.to_string(), Value::String(value.clone()));
    value
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

#[cfg(test)]
mod tests {
    use super::bridge_base_urls_with_tailscale;

    #[test]
    fn bridge_urls_use_tailscale_public_url_when_available() {
        let (ruler_url, public_base_url) =
            bridge_base_urls_with_tailscale(4622, Some("100.64.12.34".to_string()));
        assert_eq!(ruler_url, "http://127.0.0.1:4622");
        assert_eq!(public_base_url, "http://100.64.12.34:4622");
    }

    #[test]
    fn bridge_urls_fallback_to_local_when_tailscale_is_missing() {
        let (ruler_url, public_base_url) = bridge_base_urls_with_tailscale(4622, None);
        assert_eq!(ruler_url, "http://127.0.0.1:4622");
        assert_eq!(public_base_url, "http://127.0.0.1:4622");
    }

    #[test]
    fn bridge_urls_keep_custom_port() {
        let (ruler_url, public_base_url) =
            bridge_base_urls_with_tailscale(4750, Some("100.64.20.21".to_string()));
        assert_eq!(ruler_url, "http://127.0.0.1:4750");
        assert_eq!(public_base_url, "http://100.64.20.21:4750");
    }
}
