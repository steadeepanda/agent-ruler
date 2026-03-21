//! OpenCode runner adapter.
//!
//! Supports runtime-local managed state for both one-shot command mode and
//! service mode execution patterns.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde_json::{Map, Value};

use crate::config::{AppConfig, RuntimeState};
use crate::embedded_bridge::ensure_runner_bridge_assets;
use crate::helpers::runners::openclaw::setup_config::runner_api_base_url;
use crate::runners::{
    HostInstall, ImportReport, IntegrationSelection, ProvisionedPaths, RunnerAdapter,
    RunnerAssociation, RunnerKind, RunnerMissingState, RUNTIME_RUNNERS_DIR_NAME,
    RUNTIME_USER_DATA_DIR_NAME,
};
use crate::utils::resolve_command_path;

const RUNNER_SUBDIR: &str = "opencode";
const HOME_SUBDIR: &str = "home";
const WORKSPACE_SUBDIR: &str = "workspace";
const OPENCODE_AUTH_RELATIVE: &str = ".local/share/opencode/auth.json";
const OPENCODE_CONFIG_RELATIVE: &str = ".config/opencode/opencode.json";
const OPENCODE_TOOLS_PLUGIN_RELATIVE: &str = "bridge/opencode/opencode-agent-ruler-tools/index.mjs";
const OPENCODE_SAFE_RUNTIME_GUIDE_RELATIVE: &str =
    "bridge/opencode/skills/agent-ruler-safe-runtime.md";
const AGENT_RULER_MCP_SERVER_RELATIVE: &str = "bridge/shared/agent_ruler_mcp_server.py";
const AGENT_RULER_MCP_SERVER_ID: &str = "agent_ruler";

#[derive(Debug, Clone, Copy, Default)]
pub struct OpenCodeAdapter;

impl OpenCodeAdapter {
    pub fn new() -> Self {
        Self
    }
}

/// Seed managed OpenCode auth from host auth when managed auth is missing.
///
/// This keeps runner state project-local while allowing first run to reuse
/// existing host credentials without manual file copy.
pub fn ensure_managed_auth_seed(runtime: &RuntimeState) -> Result<bool> {
    let Some(runner) = runtime.config.runner.as_ref() else {
        return Ok(false);
    };
    if runner.kind != RunnerKind::Opencode {
        return Ok(false);
    }

    let managed_auth = runner.managed_home.join(OPENCODE_AUTH_RELATIVE);
    if managed_auth.is_file() {
        return Ok(false);
    }

    let Some(host_auth) = discover_host_auth_path() else {
        return Ok(false);
    };

    if let Some(parent) = managed_auth.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::copy(&host_auth, &managed_auth).with_context(|| {
        format!(
            "copy host OpenCode auth {} -> {}",
            host_auth.display(),
            managed_auth.display()
        )
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        let _ = fs::set_permissions(&managed_auth, perms);
    }

    Ok(true)
}

/// Keep managed OpenCode config pinned to Agent Ruler governance assets.
///
/// This guard re-applies plugin/mcp/instructions wiring when OpenCode rewrites
/// config so tool calls keep flowing through deterministic preflight receipts.
pub fn enforce_managed_governance_config_guard(runtime: &RuntimeState) -> Result<bool> {
    let managed_home = runtime
        .config
        .runner
        .as_ref()
        .filter(|runner| runner.kind == RunnerKind::Opencode)
        .map(|runner| runner.managed_home.clone())
        .unwrap_or_else(|| {
            runtime
                .config
                .runtime_root
                .join(RUNTIME_USER_DATA_DIR_NAME)
                .join(RUNTIME_RUNNERS_DIR_NAME)
                .join(RUNNER_SUBDIR)
                .join(HOME_SUBDIR)
        });

    ensure_managed_governance_config(runtime, &managed_home)
}

fn ensure_managed_governance_config(runtime: &RuntimeState, managed_home: &Path) -> Result<bool> {
    let config_path = managed_config_path(managed_home);
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    let mut config = read_managed_config_or_default(&config_path)?;
    let before = config.clone();

    let expected_plugin =
        file_url_from_path(&opencode_tools_plugin_path(&runtime.config.ruler_root));
    set_unique_string_array_item(&mut config, "plugin", &expected_plugin);

    let mcp_server_path = agent_ruler_mcp_server_path(&runtime.config.ruler_root)
        .to_string_lossy()
        .to_string();
    let base_url = runner_api_base_url(&runtime.config.ui_bind);
    let mcp_entry = serde_json::json!({
        "type": "local",
        "enabled": true,
        "command": ["python3", mcp_server_path],
        "environment": {
            "AGENT_RULER_BASE_URL": base_url,
            "AGENT_RULER_RUNNER_ID": RunnerKind::Opencode.id(),
        },
    });
    set_mcp_entry(&mut config, AGENT_RULER_MCP_SERVER_ID, mcp_entry);

    let safe_runtime_guide = opencode_safe_runtime_guide_path(&runtime.config.ruler_root)
        .to_string_lossy()
        .to_string();
    set_unique_string_array_item(&mut config, "instructions", &safe_runtime_guide);

    if config == before && config_path.exists() {
        return Ok(false);
    }

    fs::write(
        &config_path,
        serde_json::to_string_pretty(&Value::Object(config))
            .context("serialize managed OpenCode config")?,
    )
    .with_context(|| format!("write {}", config_path.display()))?;
    Ok(true)
}

fn validate_managed_governance_config(
    ruler_root: &Path,
    ui_bind: &str,
    managed_home: &Path,
) -> Result<()> {
    let config_path = managed_config_path(managed_home);
    let raw = fs::read_to_string(&config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let parsed: Value =
        json5::from_str(&raw).with_context(|| format!("parse {}", config_path.display()))?;

    let expected_plugin = file_url_from_path(&opencode_tools_plugin_path(ruler_root));
    let plugin_entries = parsed
        .get("plugin")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("managed OpenCode config must include `plugin` array"))?;
    if !plugin_entries
        .iter()
        .any(|value| value.as_str() == Some(expected_plugin.as_str()))
    {
        return Err(anyhow!(
            "managed OpenCode config is missing Agent Ruler plugin `{}` in `plugin`",
            expected_plugin
        ));
    }

    let mcp_entry_pointer = format!("/mcp/{AGENT_RULER_MCP_SERVER_ID}");
    let mcp_entry = parsed
        .pointer(&mcp_entry_pointer)
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("managed OpenCode config must include `{mcp_entry_pointer}`"))?;
    let mcp_type = mcp_entry
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("managed OpenCode config mcp entry must set `type`"))?;
    if mcp_type != "local" {
        return Err(anyhow!(
            "managed OpenCode mcp entry `{}` must use type `local` (got `{}`)",
            AGENT_RULER_MCP_SERVER_ID,
            mcp_type
        ));
    }
    let enabled = mcp_entry
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !enabled {
        return Err(anyhow!(
            "managed OpenCode mcp entry `{}` must set `enabled=true`",
            AGENT_RULER_MCP_SERVER_ID
        ));
    }

    let command = mcp_entry
        .get("command")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("managed OpenCode mcp entry must set `command` array"))?;
    let expected_server_path = agent_ruler_mcp_server_path(ruler_root)
        .to_string_lossy()
        .to_string();
    let expected_command = vec!["python3".to_string(), expected_server_path];
    let actual_command = command
        .iter()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if actual_command != expected_command {
        return Err(anyhow!(
            "managed OpenCode mcp command mismatch for `{}` (expected {:?}, got {:?})",
            AGENT_RULER_MCP_SERVER_ID,
            expected_command,
            actual_command
        ));
    }

    let env_base_url = mcp_entry
        .get("environment")
        .and_then(Value::as_object)
        .and_then(|env| env.get("AGENT_RULER_BASE_URL"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            anyhow!("managed OpenCode mcp entry must set `environment.AGENT_RULER_BASE_URL`")
        })?;
    let expected_base_url = runner_api_base_url(ui_bind);
    if env_base_url != expected_base_url {
        return Err(anyhow!(
            "managed OpenCode mcp base URL mismatch (expected `{}`, got `{}`)",
            expected_base_url,
            env_base_url
        ));
    }

    let expected_guide = opencode_safe_runtime_guide_path(ruler_root)
        .to_string_lossy()
        .to_string();
    let instructions = parsed
        .get("instructions")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("managed OpenCode config must include `instructions` array"))?;
    if !instructions
        .iter()
        .any(|value| value.as_str() == Some(expected_guide.as_str()))
    {
        return Err(anyhow!(
            "managed OpenCode config is missing Agent Ruler safe-runtime guide `{}` in `instructions`",
            expected_guide
        ));
    }

    Ok(())
}

fn read_managed_config_or_default(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let parsed: Value =
        json5::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    Ok(parsed.as_object().cloned().unwrap_or_default())
}

fn set_unique_string_array_item(config: &mut Map<String, Value>, key: &str, value: &str) {
    let mut entries = config
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| entry.as_str().map(ToOwned::to_owned))
        .collect::<Vec<_>>();

    if !entries.iter().any(|entry| entry == value) {
        entries.push(value.to_string());
    }

    config.insert(
        key.to_string(),
        Value::Array(entries.into_iter().map(Value::String).collect()),
    );
}

fn set_mcp_entry(config: &mut Map<String, Value>, key: &str, entry: Value) {
    let mut mcp = config
        .get("mcp")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    mcp.insert(key.to_string(), entry);
    config.insert("mcp".to_string(), Value::Object(mcp));
}

fn managed_config_path(managed_home: &Path) -> PathBuf {
    managed_home.join(OPENCODE_CONFIG_RELATIVE)
}

fn opencode_tools_plugin_path(ruler_root: &Path) -> PathBuf {
    ruler_root.join(OPENCODE_TOOLS_PLUGIN_RELATIVE)
}

fn opencode_safe_runtime_guide_path(ruler_root: &Path) -> PathBuf {
    ruler_root.join(OPENCODE_SAFE_RUNTIME_GUIDE_RELATIVE)
}

fn agent_ruler_mcp_server_path(ruler_root: &Path) -> PathBuf {
    ruler_root.join(AGENT_RULER_MCP_SERVER_RELATIVE)
}

fn file_url_from_path(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let mut encoded = String::with_capacity(normalized.len() + 8);
    for ch in normalized.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.' | '~' | ':') {
            encoded.push(ch);
            continue;
        }
        let mut buffer = [0u8; 4];
        for byte in ch.encode_utf8(&mut buffer).as_bytes() {
            encoded.push('%');
            encoded.push_str(&format!("{byte:02X}"));
        }
    }

    if encoded.starts_with('/') {
        format!("file://{encoded}")
    } else {
        format!("file:///{encoded}")
    }
}

fn discover_host_auth_path() -> Option<std::path::PathBuf> {
    let xdg_data_home = env::var("XDG_DATA_HOME").ok();
    let home = dirs::home_dir();
    discover_host_auth_path_with(xdg_data_home.as_deref(), home.as_deref())
}

fn discover_host_auth_path_with(
    xdg_data_home: Option<&str>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(xdg_data_home) = xdg_data_home {
        let trimmed = xdg_data_home.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed).join("opencode/auth.json"));
        }
    }

    if let Some(home) = home {
        candidates.push(home.join(".local/share/opencode/auth.json"));
        candidates.push(home.join("snap/code/current/.local/share/opencode/auth.json"));

        let snap_root = home.join("snap/code");
        if let Ok(entries) = fs::read_dir(&snap_root) {
            for entry in entries.flatten() {
                candidates.push(entry.path().join(".local/share/opencode/auth.json"));
            }
        }
    }

    candidates.into_iter().find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use serde_json::Value;
    use tempfile::tempdir;

    use crate::config::{init_layout, load_runtime};
    use crate::runners::{ImportReport, ProvisionedPaths, RunnerAdapter, RunnerKind};

    use super::{
        discover_host_auth_path_with, enforce_managed_governance_config_guard, managed_config_path,
        opencode_safe_runtime_guide_path, opencode_tools_plugin_path, OpenCodeAdapter,
        AGENT_RULER_MCP_SERVER_ID,
    };

    fn create_temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "agent-ruler-opencode-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    fn write_auth_file(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create auth parent");
        }
        fs::write(path, "{\"profiles\":[]}").expect("write auth file");
    }

    #[test]
    fn discover_host_auth_prefers_xdg_data_home() {
        let root = create_temp_dir("xdg-priority");
        let xdg_data_home = root.join("xdg-data");
        let home = root.join("home");
        let xdg_auth = xdg_data_home.join("opencode/auth.json");
        let home_auth = home.join(".local/share/opencode/auth.json");

        write_auth_file(&xdg_auth);
        write_auth_file(&home_auth);

        let found = discover_host_auth_path_with(
            Some(xdg_data_home.to_string_lossy().as_ref()),
            Some(&home),
        )
        .expect("discover auth path");
        assert_eq!(found, xdg_auth);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn discover_host_auth_falls_back_to_snap_code_current() {
        let root = create_temp_dir("snap-current");
        let home = root.join("home");
        let snap_auth = home.join("snap/code/current/.local/share/opencode/auth.json");
        write_auth_file(&snap_auth);

        let found = discover_host_auth_path_with(None, Some(&home)).expect("discover auth path");
        assert_eq!(found, snap_auth);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn opencode_adapter_provisions_runtime_local_paths() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");
        init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
        let runtime =
            load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");

        let adapter = OpenCodeAdapter::new();
        let paths = adapter
            .provision_project_paths(&runtime)
            .expect("provision paths");

        assert!(
            paths.managed_home.starts_with(&runtime.config.runtime_root),
            "managed home should stay under runtime root"
        );
        assert!(
            paths
                .managed_workspace
                .starts_with(&runtime.config.runtime_root),
            "managed workspace should stay under runtime root"
        );
        assert!(paths
            .managed_home
            .ends_with("user_data/runners/opencode/home"));
        assert!(paths
            .managed_workspace
            .ends_with("user_data/runners/opencode/workspace"));
    }

    #[test]
    fn opencode_adapter_writes_and_validates_runner_config() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");
        init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
        let runtime =
            load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");
        let mut runtime = runtime;
        runtime.config.ruler_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

        let adapter = OpenCodeAdapter::new();
        let mut config = runtime.config.clone();
        let paths = ProvisionedPaths {
            managed_home: runtime
                .config
                .runtime_root
                .join("user_data/runners/opencode/home"),
            managed_workspace: runtime
                .config
                .runtime_root
                .join("user_data/runners/opencode/workspace"),
        };

        adapter
            .write_runner_config(&runtime, &mut config, &paths, &ImportReport::default(), &[])
            .expect("write runner config");
        adapter.validate(&config).expect("validate opencode config");

        let runner = config.runner.expect("runner config");
        assert_eq!(runner.kind, RunnerKind::Opencode);
        assert_eq!(runner.managed_home, paths.managed_home);
        assert_eq!(runner.managed_workspace, paths.managed_workspace);
    }

    #[test]
    fn opencode_adapter_wires_managed_governance_assets_into_config() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");
        init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
        let runtime =
            load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");
        let mut runtime = runtime;
        runtime.config.ruler_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

        let adapter = OpenCodeAdapter::new();
        let mut config = runtime.config.clone();
        let paths = ProvisionedPaths {
            managed_home: runtime
                .config
                .runtime_root
                .join("user_data/runners/opencode/home"),
            managed_workspace: runtime
                .config
                .runtime_root
                .join("user_data/runners/opencode/workspace"),
        };

        adapter
            .write_runner_config(&runtime, &mut config, &paths, &ImportReport::default(), &[])
            .expect("write runner config");

        let config_path = managed_config_path(&paths.managed_home);
        let parsed: Value = serde_json::from_str(
            &fs::read_to_string(&config_path).expect("read managed OpenCode config"),
        )
        .expect("parse managed OpenCode config");

        let expected_plugin =
            super::file_url_from_path(&opencode_tools_plugin_path(&runtime.config.ruler_root));
        let plugins = parsed
            .get("plugin")
            .and_then(Value::as_array)
            .expect("plugin array");
        assert!(
            plugins
                .iter()
                .any(|value| value.as_str() == Some(expected_plugin.as_str())),
            "managed OpenCode config should include Agent Ruler plugin"
        );

        let mcp = parsed
            .pointer(&format!("/mcp/{AGENT_RULER_MCP_SERVER_ID}"))
            .and_then(Value::as_object)
            .expect("agent_ruler MCP entry");
        assert_eq!(
            mcp.get("type").and_then(Value::as_str),
            Some("local"),
            "Agent Ruler MCP entry should use local transport"
        );
        assert_eq!(
            mcp.get("enabled").and_then(Value::as_bool),
            Some(true),
            "Agent Ruler MCP entry should be explicitly enabled"
        );

        let instructions = parsed
            .get("instructions")
            .and_then(Value::as_array)
            .expect("instructions array");
        let expected_guide = opencode_safe_runtime_guide_path(&runtime.config.ruler_root)
            .to_string_lossy()
            .to_string();
        assert!(
            instructions
                .iter()
                .any(|value| value.as_str() == Some(expected_guide.as_str())),
            "managed OpenCode config should include safe-runtime guide"
        );
    }

    #[test]
    fn opencode_governance_guard_repairs_removed_plugin_wiring() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");
        init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
        let mut runtime =
            load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");
        runtime.config.ruler_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

        let adapter = OpenCodeAdapter::new();
        let mut config = runtime.config.clone();
        let paths = ProvisionedPaths {
            managed_home: config.runtime_root.join("user_data/runners/opencode/home"),
            managed_workspace: config
                .runtime_root
                .join("user_data/runners/opencode/workspace"),
        };

        adapter
            .write_runner_config(&runtime, &mut config, &paths, &ImportReport::default(), &[])
            .expect("write runner config");
        runtime.config = config;

        let config_path = managed_config_path(&paths.managed_home);
        let mut parsed: Value = serde_json::from_str(
            &fs::read_to_string(&config_path).expect("read managed OpenCode config"),
        )
        .expect("parse managed OpenCode config");
        parsed
            .as_object_mut()
            .expect("managed config root object")
            .insert("plugin".to_string(), Value::Array(vec![]));
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&parsed).expect("serialize tampered config"),
        )
        .expect("write tampered config");

        let changed = enforce_managed_governance_config_guard(&runtime).expect("apply guard");
        assert!(changed, "guard should re-apply missing plugin wiring");

        let repaired: Value =
            serde_json::from_str(&fs::read_to_string(&config_path).expect("read repaired config"))
                .expect("parse repaired config");
        let plugins = repaired
            .get("plugin")
            .and_then(Value::as_array)
            .expect("plugin array after guard");
        assert!(!plugins.is_empty(), "guard should restore plugin entries");
    }
}

impl RunnerAdapter for OpenCodeAdapter {
    fn kind(&self) -> RunnerKind {
        RunnerKind::Opencode
    }

    fn display_name(&self) -> &'static str {
        "OpenCode"
    }

    fn detect_host_install(&self, host_hint: Option<&Path>) -> Result<Option<HostInstall>> {
        if let Some(path) = host_hint {
            if path.exists() {
                return Ok(Some(HostInstall {
                    home: path.to_path_buf(),
                    detected_by: "manual path".to_string(),
                }));
            }
        }

        if let Some(path) = resolve_command_path(self.kind().executable_name()) {
            return Ok(Some(HostInstall {
                home: path,
                detected_by: "PATH".to_string(),
            }));
        }

        Ok(None)
    }

    fn provision_project_paths(&self, runtime: &RuntimeState) -> Result<ProvisionedPaths> {
        let runner_root = runtime
            .config
            .runtime_root
            .join(RUNTIME_USER_DATA_DIR_NAME)
            .join(RUNTIME_RUNNERS_DIR_NAME)
            .join(RUNNER_SUBDIR);
        let managed_home = runner_root.join(HOME_SUBDIR);
        let managed_workspace = runner_root.join(WORKSPACE_SUBDIR);
        fs::create_dir_all(&managed_home)?;
        fs::create_dir_all(&managed_workspace)?;
        Ok(ProvisionedPaths {
            managed_home,
            managed_workspace,
        })
    }

    fn optional_import_from_host(
        &self,
        _host_install: Option<&HostInstall>,
        paths: &ProvisionedPaths,
        _import_from_host: bool,
    ) -> Result<ImportReport> {
        let Some(host_auth) = discover_host_auth_path() else {
            return Ok(ImportReport::default());
        };

        let managed_auth = paths.managed_home.join(OPENCODE_AUTH_RELATIVE);
        if let Some(parent) = managed_auth.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::copy(&host_auth, &managed_auth).with_context(|| {
            format!(
                "copy host OpenCode auth {} -> {}",
                host_auth.display(),
                managed_auth.display()
            )
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            let _ = fs::set_permissions(&managed_auth, perms);
        }

        let mut report = ImportReport::default();
        report.imported = true;
        report.copied_items.push("auth.json".to_string());
        Ok(report)
    }

    fn write_runner_config(
        &self,
        runtime: &RuntimeState,
        config: &mut AppConfig,
        paths: &ProvisionedPaths,
        _import_report: &ImportReport,
        integrations: &[IntegrationSelection],
    ) -> Result<()> {
        ensure_managed_governance_config(runtime, &paths.managed_home)?;

        config.runner = Some(RunnerAssociation {
            kind: self.kind(),
            managed_home: paths.managed_home.clone(),
            managed_workspace: paths.managed_workspace.clone(),
            integrations: integrations.iter().map(|item| item.id.clone()).collect(),
            missing: RunnerMissingState {
                executable_missing: resolve_command_path(self.kind().executable_name()).is_none(),
                decision: None,
            },
        });
        Ok(())
    }

    fn validate(&self, config: &AppConfig) -> Result<()> {
        let runner = config
            .runner
            .as_ref()
            .ok_or_else(|| anyhow!("runner config missing after setup"))?;
        if runner.kind != self.kind() {
            return Err(anyhow!("runner kind mismatch; expected opencode"));
        }
        if !runner.managed_home.starts_with(&config.runtime_root) {
            return Err(anyhow!(
                "managed home is outside runtime root: {}",
                runner.managed_home.display()
            ));
        }
        if !runner.managed_workspace.starts_with(&config.runtime_root) {
            return Err(anyhow!(
                "managed workspace is outside runtime root: {}",
                runner.managed_workspace.display()
            ));
        }
        ensure_runner_bridge_assets(&config.ruler_root, RunnerKind::Opencode)
            .context("validate OpenCode bridge assets")?;

        validate_managed_governance_config(
            &config.ruler_root,
            &config.ui_bind,
            &runner.managed_home,
        )?;

        Ok(())
    }

    fn print_next_steps(&self, _runtime: &RuntimeState, config: &AppConfig) {
        let Some(runner) = config.runner.as_ref() else {
            return;
        };
        println!("setup complete: OpenCode runner configured");
        println!("managed home: {}", runner.managed_home.display());
        println!("managed workspace: {}", runner.managed_workspace.display());
        println!();
        println!("interactive mode example:");
        println!("agent-ruler run -- opencode");
        println!();
        println!("one-shot command mode example:");
        println!("agent-ruler run -- opencode run \"Summarize TODO.md\"");
        println!();
        println!("service mode example:");
        println!("agent-ruler run -- opencode serve");
    }
}
