//! OpenClaw managed-config mutation helpers.
//!
//! Extracted from `src/runners/openclaw.rs` to keep setup flow orchestration
//! separate from JSON mutation details. If this file changes, check
//! `OpenClawAdapter::write_runner_config` in `src/runners/openclaw.rs`.

use std::path::Path;

use serde_json::{Map, Value};

use crate::config::RuntimeState;

/// Merge the import-safe top-level sections into managed config.
///
/// This helper intentionally does a shallow key overwrite only for explicitly
/// allowed root keys so host-only settings outside that allowlist cannot leak
/// into managed runtime config.
pub(crate) fn merge_imported_sections(
    config: &mut Map<String, Value>,
    imported: Option<&Value>,
    importable_root_keys: &[&str],
) {
    let Some(imported_obj) = imported.and_then(Value::as_object) else {
        return;
    };
    for key in importable_root_keys {
        if let Some(value) = imported_obj.get(*key) {
            config.insert((*key).to_string(), value.clone());
        }
    }
}

/// Force OpenClaw workspace to Agent Ruler managed workspace.
///
/// Setup must always pin workspace explicitly so relative paths in tool calls
/// resolve against project-local runtime state rather than host defaults.
pub(crate) fn set_workspace(config: &mut Map<String, Value>, workspace: &Path) {
    let agents = ensure_object(config, "agents");
    let defaults = ensure_object(agents, "defaults");
    defaults.insert(
        "workspace".to_string(),
        Value::String(workspace.to_string_lossy().to_string()),
    );
}

/// Enforce local gateway mode in managed config.
///
/// Agent Ruler never configures managed OpenClaw homes for remote gateway mode.
pub(crate) fn set_gateway_mode_local(config: &mut Map<String, Value>) {
    let gateway = ensure_object(config, "gateway");
    gateway.insert("mode".to_string(), Value::String("local".to_string()));
}

/// Normalize and enforce managed Telegram streaming for OpenClaw.
///
/// Managed Agent Ruler Telegram UX expects assistant replies to surface live and
/// in-order before approval cards. Keep Telegram draft streaming enabled and
/// block-streaming disabled for managed homes whenever Telegram is configured
/// and enabled.
pub(crate) fn normalize_telegram_streaming_flag(config: &mut Map<String, Value>) -> bool {
    let Some(channels) = config.get_mut("channels").and_then(Value::as_object_mut) else {
        return false;
    };
    let Some(telegram) = channels.get_mut("telegram").and_then(Value::as_object_mut) else {
        return false;
    };

    if matches!(telegram.get("enabled"), Some(Value::Bool(false))) {
        return false;
    }

    let mut changed = false;
    if let Some(raw_streaming) = telegram.get("streaming").cloned() {
        match raw_streaming {
            Value::String(raw) => {
                let coerced = coerce_string_bool(&raw);
                if let Some(flag) = coerced {
                    telegram.insert("streaming".to_string(), Value::Bool(flag));
                    changed = true;
                }
            }
            Value::Bool(_) => {}
            _ => {
                changed = true;
            }
        }
    };
    if telegram.get("streaming").and_then(Value::as_bool) != Some(true) {
        telegram.insert("streaming".to_string(), Value::Bool(true));
        changed = true;
    }
    if let Some(raw_block_streaming) = telegram.get("blockStreaming").cloned() {
        match raw_block_streaming {
            Value::String(raw) => {
                let coerced = coerce_string_bool(&raw);
                if let Some(flag) = coerced {
                    telegram.insert("blockStreaming".to_string(), Value::Bool(flag));
                    changed = true;
                }
            }
            Value::Bool(_) => {}
            _ => {
                changed = true;
            }
        }
    }
    if telegram.get("blockStreaming").and_then(Value::as_bool) != Some(false) {
        telegram.insert("blockStreaming".to_string(), Value::Bool(false));
        changed = true;
    }

    let agents = ensure_object(config, "agents");
    let defaults = ensure_object(agents, "defaults");
    if defaults
        .get("blockStreamingDefault")
        .and_then(Value::as_str)
        != Some("off")
    {
        defaults.insert(
            "blockStreamingDefault".to_string(),
            Value::String("off".to_string()),
        );
        changed = true;
    }

    changed
}

/// Apply the OpenClaw tools adapter configuration into managed `openclaw.json`.
///
/// Invariants:
/// - Writes only managed/runtime-local config.
/// - Ensures plugin path + plugin entry + primary agent tool allowlist stay in sync.
pub(crate) fn apply_tools_adapter_config(
    runtime: &RuntimeState,
    config: &mut Map<String, Value>,
    plugin_id: &str,
) {
    let plugin_path = {
        let preferred = runtime
            .config
            .ruler_root
            .join("bridge")
            .join("openclaw")
            .join("openclaw-agent-ruler-tools");
        let legacy_scoped = runtime
            .config
            .ruler_root
            .join("bridge")
            .join("openclaw")
            .join("tools-adapter");
        let legacy_root = runtime
            .config
            .ruler_root
            .join("bridge")
            .join("openclaw-agent-ruler-tools");
        if preferred.exists() || (!legacy_scoped.exists() && !legacy_root.exists()) {
            preferred
        } else if legacy_scoped.exists() {
            legacy_scoped
        } else {
            legacy_root
        }
        .to_string_lossy()
        .to_string()
    };

    let plugins = ensure_object(config, "plugins");
    let load = ensure_object(plugins, "load");
    let load_paths = ensure_array(load, "paths");
    prune_stale_tools_adapter_paths(load_paths, &plugin_path);
    ensure_string_in_array(load_paths, &plugin_path);

    let entries = ensure_object(plugins, "entries");
    let plugin_entry = ensure_object(entries, plugin_id);
    plugin_entry.insert("enabled".to_string(), Value::Bool(true));
    let plugin_cfg = ensure_object(plugin_entry, "config");
    let base_url = runner_api_base_url(&runtime.config.ui_bind);
    plugin_cfg.insert("baseUrl".to_string(), Value::String(base_url));
    plugin_cfg.insert(
        "approvalWaitTimeoutSecs".to_string(),
        Value::from(runtime.config.approval_wait_timeout_secs.clamp(1, 300)),
    );

    // OpenClaw agent lists can be absent in fresh homes; setup must still produce
    // a deterministic primary agent that can call the adapter's optional tools.
    let primary_agent = ensure_primary_agent_entry(config);
    if primary_agent
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        primary_agent.insert("id".to_string(), Value::String("main".to_string()));
    }
    let tools = ensure_object(primary_agent, "tools");
    let allow = ensure_array(tools, "allow");
    ensure_string_in_array(allow, plugin_id);
}

fn prune_stale_tools_adapter_paths(load_paths: &mut Vec<Value>, canonical_plugin_path: &str) {
    load_paths.retain(|entry| {
        let Some(path) = entry.as_str() else {
            return true;
        };
        if path == canonical_plugin_path {
            return true;
        }
        !looks_like_agent_ruler_openclaw_plugin_path(path)
    });
}

fn looks_like_agent_ruler_openclaw_plugin_path(path: &str) -> bool {
    path.ends_with("/bridge/openclaw/openclaw-agent-ruler-tools")
        || path.ends_with("/bridge/openclaw/tools-adapter")
        || path.ends_with("/bridge/openclaw-agent-ruler-tools")
}

/// Resolve the in-runner Agent Ruler API base URL for OpenClaw tools adapter.
///
/// This intentionally always targets loopback for lowest-latency preflight and
/// wait/resume calls. UI server startup ensures a loopback listener is available
/// even when the public UI bind is a concrete interface (for example Tailscale).
pub(crate) fn runner_api_base_url(ui_bind: &str) -> String {
    let port = ui_bind
        .trim()
        .rsplit_once(':')
        .and_then(|(_, raw)| raw.parse::<u16>().ok())
        .unwrap_or(4622);
    format!("http://127.0.0.1:{port}")
}

pub(crate) fn selected_model_provider(config: &Map<String, Value>) -> Option<String> {
    let model_value = config
        .get("agents")
        .and_then(Value::as_object)
        .and_then(|agents| agents.get("defaults"))
        .and_then(Value::as_object)
        .and_then(|defaults| defaults.get("model"))?;

    match model_value {
        Value::String(raw) => provider_from_model_ref(raw),
        Value::Object(map) => {
            let explicit = map
                .get("provider")
                .or_else(|| map.get("modelProvider"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            if explicit.is_some() {
                return explicit;
            }

            if let Some(primary) = map.get("primary") {
                match primary {
                    Value::String(raw) => provider_from_model_ref(raw),
                    Value::Object(obj) => obj
                        .get("provider")
                        .or_else(|| obj.get("modelProvider"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned),
                    _ => None,
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

fn provider_from_model_ref(raw_model: &str) -> Option<String> {
    let model = raw_model.trim();
    if model.is_empty() {
        return None;
    }
    let (provider, _) = model.split_once('/')?;
    let provider = provider.trim();
    if provider.is_empty() {
        None
    } else {
        Some(provider.to_string())
    }
}

pub(crate) fn ensure_object<'a>(
    map: &'a mut Map<String, Value>,
    key: &str,
) -> &'a mut Map<String, Value> {
    let value = map
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value
        .as_object_mut()
        .expect("value should be object after normalization")
}

fn ensure_array<'a>(map: &'a mut Map<String, Value>, key: &str) -> &'a mut Vec<Value> {
    let value = map
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !value.is_array() {
        *value = Value::Array(Vec::new());
    }
    value
        .as_array_mut()
        .expect("value should be array after normalization")
}

fn ensure_primary_agent_entry(config: &mut Map<String, Value>) -> &mut Map<String, Value> {
    let agents = ensure_object(config, "agents");
    let list_value = agents
        .entry("list".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !list_value.is_array() {
        *list_value = Value::Array(Vec::new());
    }
    let list = list_value
        .as_array_mut()
        .expect("agents.list should be array after normalization");
    if list.is_empty() {
        list.push(Value::Object(Map::new()));
    }
    if !list[0].is_object() {
        list[0] = Value::Object(Map::new());
    }
    list[0]
        .as_object_mut()
        .expect("agents.list[0] should be object after normalization")
}

fn ensure_string_in_array(values: &mut Vec<Value>, target: &str) {
    if values
        .iter()
        .any(|entry| entry.as_str().map(|value| value == target).unwrap_or(false))
    {
        return;
    }
    values.push(Value::String(target.to_string()));
}

fn coerce_string_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "on" | "true" | "1" | "yes" => Some(true),
        "off" | "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        looks_like_agent_ruler_openclaw_plugin_path, normalize_telegram_streaming_flag,
        prune_stale_tools_adapter_paths, runner_api_base_url,
    };

    #[test]
    fn runner_api_base_url_uses_loopback_for_tailscale_ip_bind() {
        assert_eq!(
            runner_api_base_url("100.64.12.34:4622"),
            "http://127.0.0.1:4622"
        );
    }

    #[test]
    fn runner_api_base_url_uses_loopback_for_wildcard_bind() {
        assert_eq!(runner_api_base_url("0.0.0.0:4750"), "http://127.0.0.1:4750");
    }

    #[test]
    fn normalize_telegram_streaming_enables_live_streaming_when_false() {
        let mut cfg = json!({
            "channels": {
                "telegram": {
                    "enabled": true,
                    "streaming": false,
                    "blockStreaming": true
                }
            }
        })
        .as_object()
        .cloned()
        .expect("config should be object");
        assert!(normalize_telegram_streaming_flag(&mut cfg));
        let parsed = serde_json::Value::Object(cfg);
        assert_eq!(
            parsed
                .pointer("/channels/telegram/streaming")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            parsed
                .pointer("/channels/telegram/blockStreaming")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            parsed
                .pointer("/agents/defaults/blockStreamingDefault")
                .and_then(serde_json::Value::as_str),
            Some("off")
        );
    }

    #[test]
    fn normalize_telegram_streaming_preserves_disabled_channel() {
        let mut cfg = json!({
            "channels": {
                "telegram": {
                    "enabled": false,
                    "streaming": false
                }
            }
        })
        .as_object()
        .cloned()
        .expect("config should be object");
        assert!(!normalize_telegram_streaming_flag(&mut cfg));
        let parsed = serde_json::Value::Object(cfg);
        assert_eq!(
            parsed
                .pointer("/channels/telegram/streaming")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn prune_stale_tools_adapter_paths_removes_only_old_agent_ruler_variants() {
        let canonical = "/repo/bridge/openclaw/openclaw-agent-ruler-tools";
        let mut load_paths = vec![
            json!("/old/install/bridge/openclaw/openclaw-agent-ruler-tools"),
            json!("/runtime/workspace/bridge/openclaw/openclaw-agent-ruler-tools"),
            json!("/custom/plugins/other-plugin"),
            json!(canonical),
        ];

        prune_stale_tools_adapter_paths(&mut load_paths, canonical);

        assert_eq!(load_paths.len(), 2);
        assert!(load_paths
            .iter()
            .any(|entry| entry.as_str() == Some(canonical)));
        assert!(load_paths
            .iter()
            .any(|entry| entry.as_str() == Some("/custom/plugins/other-plugin")));
    }

    #[test]
    fn detects_agent_ruler_openclaw_plugin_path_variants() {
        assert!(looks_like_agent_ruler_openclaw_plugin_path(
            "/tmp/workspace/bridge/openclaw/openclaw-agent-ruler-tools"
        ));
        assert!(looks_like_agent_ruler_openclaw_plugin_path(
            "/tmp/workspace/bridge/openclaw/tools-adapter"
        ));
        assert!(!looks_like_agent_ruler_openclaw_plugin_path(
            "/tmp/workspace/plugins/other-plugin"
        ));
    }
}
