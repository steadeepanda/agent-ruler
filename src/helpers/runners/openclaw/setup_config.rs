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

/// Disable the session-memory hook for non-Anthropic provider defaults.
///
/// This preserves startup behavior for providers where the hook is not
/// compatible or creates noisy failures during gateway boot.
pub(crate) fn disable_session_memory_hook_for_non_anthropic(config: &mut Map<String, Value>) {
    let Some(provider) = selected_model_provider(config) else {
        return;
    };
    if provider.eq_ignore_ascii_case("anthropic") {
        return;
    }

    let hooks = ensure_object(config, "hooks");
    let internal = ensure_object(hooks, "internal");
    let entries = ensure_object(internal, "entries");
    let session_memory = ensure_object(entries, "session-memory");
    session_memory.insert("enabled".to_string(), Value::Bool(false));
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

fn selected_model_provider(config: &Map<String, Value>) -> Option<String> {
    let raw_model = config
        .get("agents")
        .and_then(Value::as_object)
        .and_then(|agents| agents.get("defaults"))
        .and_then(Value::as_object)
        .and_then(|defaults| defaults.get("model"))
        .and_then(|model| match model {
            Value::String(value) => Some(value.as_str()),
            Value::Object(map) => map.get("primary").and_then(Value::as_str),
            _ => None,
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    let (provider, _) = raw_model.split_once('/')?;
    let provider = provider.trim();
    if provider.is_empty() {
        None
    } else {
        Some(provider.to_string())
    }
}

fn ensure_object<'a>(map: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
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

#[cfg(test)]
mod tests {
    use super::runner_api_base_url;

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
}
