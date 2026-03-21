//! Claude Code runner adapter.
//!
//! This adapter keeps Claude Code state under runtime-local managed paths while
//! preserving the same setup/remove lifecycle as other runners.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde_json::{Map, Value as JsonValue};

use crate::config::{AppConfig, RuntimeState};
use crate::embedded_bridge::ensure_runner_bridge_assets;
use crate::runners::{
    HostInstall, ImportReport, IntegrationSelection, ProvisionedPaths, RunnerAdapter,
    RunnerAssociation, RunnerKind, RunnerMissingState, RUNTIME_RUNNERS_DIR_NAME,
    RUNTIME_USER_DATA_DIR_NAME,
};
use crate::utils::resolve_command_path;

const RUNNER_SUBDIR: &str = "claudecode";
const HOME_SUBDIR: &str = "home";
const WORKSPACE_SUBDIR: &str = "workspace";
const HOST_CLAUDE_DIR_NAME: &str = ".claude";
const CLAUDE_SETTINGS_FILE_NAME: &str = "settings.json";
const CLAUDE_PERMISSION_DEFAULT_MODE: &str = "bypassPermissions";

#[derive(Debug, Clone, Copy, Default)]
pub struct ClaudeCodeAdapter;

impl ClaudeCodeAdapter {
    pub fn new() -> Self {
        Self
    }
}

/// Seed managed Claude settings from the host configuration when missing.
///
/// Claude Code accepts API-token/base-URL auth via `settings.json`, so managed
/// runtimes should reuse that host configuration before we assume OAuth login
/// is required inside the project-local home.
pub fn ensure_managed_settings_seed(runtime: &RuntimeState) -> Result<bool> {
    let Some(runner) = runtime.config.runner.as_ref() else {
        return Ok(false);
    };
    if runner.kind != RunnerKind::Claudecode {
        return Ok(false);
    }

    let managed_settings = managed_settings_path(&runner.managed_home);
    if managed_settings.is_file() {
        return Ok(false);
    }

    let Some(host_settings) = discover_host_settings_path() else {
        return Ok(false);
    };
    copy_settings_file(&host_settings, &managed_settings)?;
    Ok(true)
}

/// Keep the managed Claude settings pinned to the Agent Ruler permission model.
///
/// Agent Ruler remains the governing safety boundary, so the managed Claude
/// profile disables Claude's redundant prompt loop while preserving auth and
/// any other host-imported settings.
pub fn enforce_managed_settings_guard(runtime: &RuntimeState) -> Result<bool> {
    let managed_home = runtime
        .config
        .runner
        .as_ref()
        .filter(|runner| runner.kind == RunnerKind::Claudecode)
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
    ensure_managed_settings_profile(&managed_home)
}

/// Query Claude auth status in the managed runtime home.
///
/// This keeps auth checks scoped to Agent Ruler managed state so CLI guidance
/// is accurate for confined runs, not host-global shell state.
pub fn managed_auth_logged_in(runtime: &RuntimeState) -> Result<Option<bool>> {
    let Some(runner) = runtime.config.runner.as_ref() else {
        return Ok(None);
    };
    if runner.kind != RunnerKind::Claudecode {
        return Ok(None);
    }

    let output = Command::new("claude")
        .args(["auth", "status", "--json"])
        .env("CLAUDE_CONFIG_DIR", &runner.managed_home)
        .env("HOME", &runner.managed_home)
        .output();

    let output = match output {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };

    parse_auth_logged_in_from_output(&output.stdout, &output.stderr)
}

fn parse_auth_logged_in_from_output(stdout: &[u8], stderr: &[u8]) -> Result<Option<bool>> {
    let stdout_text = String::from_utf8_lossy(stdout).trim().to_string();
    if let Some(value) = parse_auth_logged_in_json(&stdout_text)? {
        return Ok(Some(value));
    }

    let stderr_text = String::from_utf8_lossy(stderr).trim().to_string();
    if let Some(value) = parse_auth_logged_in_json(&stderr_text)? {
        return Ok(Some(value));
    }

    Ok(None)
}

fn parse_auth_logged_in_json(text: &str) -> Result<Option<bool>> {
    if text.is_empty() {
        return Ok(None);
    }
    let parsed: JsonValue = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    Ok(parsed.get("loggedIn").and_then(JsonValue::as_bool))
}

fn managed_settings_path(managed_home: &Path) -> PathBuf {
    managed_home.join(CLAUDE_SETTINGS_FILE_NAME)
}

fn ensure_managed_settings_profile(managed_home: &Path) -> Result<bool> {
    let settings_path = managed_settings_path(managed_home);
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    let mut settings = read_settings_or_default(&settings_path)?;
    let before = settings.clone();
    let permissions = ensure_object_key(&mut settings, "permissions");
    permissions.insert(
        "defaultMode".to_string(),
        JsonValue::String(CLAUDE_PERMISSION_DEFAULT_MODE.to_string()),
    );
    // Keep bypass enabled in the managed runtime; Agent Ruler policy is the
    // enforced prompt boundary for confined Claude runs.
    permissions.remove("disableBypassPermissionsMode");

    if settings == before && settings_path.exists() {
        return Ok(false);
    }

    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&JsonValue::Object(settings))
            .context("serialize managed Claude settings")?,
    )
    .with_context(|| format!("write {}", settings_path.display()))?;
    restrict_settings_permissions(&settings_path);
    Ok(true)
}

fn read_settings_or_default(path: &Path) -> Result<Map<String, JsonValue>> {
    if !path.exists() {
        return Ok(Map::new());
    }

    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let parsed: JsonValue =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    match parsed {
        JsonValue::Object(object) => Ok(object),
        _ => Err(anyhow!(
            "managed Claude settings must be a JSON object: {}",
            path.display()
        )),
    }
}

fn ensure_object_key<'a>(
    parent: &'a mut Map<String, JsonValue>,
    key: &str,
) -> &'a mut Map<String, JsonValue> {
    let needs_reset = !matches!(parent.get(key), Some(JsonValue::Object(_)));
    if needs_reset {
        parent.insert(key.to_string(), JsonValue::Object(Map::new()));
    }

    parent
        .get_mut(key)
        .and_then(JsonValue::as_object_mut)
        .expect("object key should exist")
}

fn discover_host_settings_path() -> Option<PathBuf> {
    let config_dir = env::var("CLAUDE_CONFIG_DIR").ok();
    let home = dirs::home_dir();
    discover_host_settings_path_with(config_dir.as_deref(), home.as_deref())
}

fn discover_host_settings_path_with(
    config_dir: Option<&str>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(config_dir) = config_dir {
        let trimmed = config_dir.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            let candidate = if path.is_dir() {
                path.join(CLAUDE_SETTINGS_FILE_NAME)
            } else {
                path
            };
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    let home = home?;
    let candidate = home
        .join(HOST_CLAUDE_DIR_NAME)
        .join(CLAUDE_SETTINGS_FILE_NAME);
    if candidate.is_file() {
        return Some(candidate);
    }
    None
}

fn copy_settings_file(src: &Path, dst: &Path) -> Result<()> {
    let Some(parent) = dst.parent() else {
        return Err(anyhow!("managed Claude settings path has no parent"));
    };
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    fs::copy(src, dst).with_context(|| format!("copy {} -> {}", src.display(), dst.display()))?;

    restrict_settings_permissions(dst);

    Ok(())
}

fn restrict_settings_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        let _ = fs::set_permissions(path, perms);
    }
}

impl RunnerAdapter for ClaudeCodeAdapter {
    fn kind(&self) -> RunnerKind {
        RunnerKind::Claudecode
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
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
        let Some(host_settings) = discover_host_settings_path() else {
            ensure_managed_settings_profile(&paths.managed_home)?;
            return Ok(ImportReport::default());
        };

        let managed_settings = managed_settings_path(&paths.managed_home);
        copy_settings_file(&host_settings, &managed_settings)?;
        ensure_managed_settings_profile(&paths.managed_home)?;

        let mut report = ImportReport::default();
        report.imported = true;
        report
            .copied_items
            .push(CLAUDE_SETTINGS_FILE_NAME.to_string());
        Ok(report)
    }

    fn write_runner_config(
        &self,
        _runtime: &RuntimeState,
        config: &mut AppConfig,
        paths: &ProvisionedPaths,
        _import_report: &ImportReport,
        integrations: &[IntegrationSelection],
    ) -> Result<()> {
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
            return Err(anyhow!("runner kind mismatch; expected claudecode"));
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
        ensure_runner_bridge_assets(&config.ruler_root, RunnerKind::Claudecode)
            .context("validate Claude Code bridge assets")?;
        Ok(())
    }

    fn print_next_steps(&self, _runtime: &RuntimeState, config: &AppConfig) {
        let Some(runner) = config.runner.as_ref() else {
            return;
        };
        println!("setup complete: Claude Code runner configured");
        println!("managed home: {}", runner.managed_home.display());
        println!("managed workspace: {}", runner.managed_workspace.display());
        println!(
            "managed settings: permissions.defaultMode={} (Agent Ruler remains the safety boundary)",
            CLAUDE_PERMISSION_DEFAULT_MODE
        );
        println!();
        println!("interactive mode example:");
        println!("agent-ruler run -- claude");
        println!();
        println!("one-shot command mode example:");
        println!("agent-ruler run -- claude -p \"Summarize workspace TODOs\"");
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use crate::config::{init_layout, load_runtime};
    use crate::runners::{ImportReport, ProvisionedPaths, RunnerAdapter, RunnerKind};

    use super::{
        discover_host_settings_path_with, ensure_managed_settings_profile,
        parse_auth_logged_in_from_output, ClaudeCodeAdapter, JsonValue,
        CLAUDE_PERMISSION_DEFAULT_MODE,
    };

    #[test]
    fn claudecode_adapter_provisions_runtime_local_paths() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");
        init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
        let runtime =
            load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");
        let mut runtime = runtime;
        runtime.config.ruler_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

        let adapter = ClaudeCodeAdapter::new();
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
            .ends_with("user_data/runners/claudecode/home"));
        assert!(paths
            .managed_workspace
            .ends_with("user_data/runners/claudecode/workspace"));
    }

    #[test]
    fn claudecode_adapter_writes_and_validates_runner_config() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");
        init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
        let runtime =
            load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");
        let mut runtime = runtime;
        runtime.config.ruler_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

        let adapter = ClaudeCodeAdapter::new();
        let mut config = runtime.config.clone();
        let paths = ProvisionedPaths {
            managed_home: runtime
                .config
                .runtime_root
                .join("user_data/runners/claudecode/home"),
            managed_workspace: runtime
                .config
                .runtime_root
                .join("user_data/runners/claudecode/workspace"),
        };

        adapter
            .write_runner_config(&runtime, &mut config, &paths, &ImportReport::default(), &[])
            .expect("write runner config");
        adapter
            .validate(&config)
            .expect("validate claudecode config");

        let runner = config.runner.expect("runner config");
        assert_eq!(runner.kind, RunnerKind::Claudecode);
        assert_eq!(runner.managed_home, paths.managed_home);
        assert_eq!(runner.managed_workspace, paths.managed_workspace);
    }

    #[test]
    fn parse_managed_auth_status_reads_logged_in_flag_from_stdout_json() {
        let status = parse_auth_logged_in_from_output(
            br#"{"loggedIn":true,"authMethod":"oauth_token"}"#,
            b"",
        )
        .expect("parse auth status");
        assert_eq!(status, Some(true));
    }

    #[test]
    fn parse_managed_auth_status_falls_back_to_stderr_json() {
        let status =
            parse_auth_logged_in_from_output(b"", br#"{"loggedIn":false,"authMethod":"none"}"#)
                .expect("parse auth status");
        assert_eq!(status, Some(false));
    }

    #[test]
    fn parse_managed_auth_status_ignores_non_json_output() {
        let status = parse_auth_logged_in_from_output(b"Not logged in. Please run /login", b"")
            .expect("parse auth status");
        assert_eq!(status, None);
    }

    #[test]
    fn discover_host_settings_prefers_explicit_config_dir() {
        let root = tempdir().expect("root tempdir");
        let config_dir = root.path().join("claude-config");
        fs::create_dir_all(&config_dir).expect("create config dir");
        let config_settings = config_dir.join("settings.json");
        fs::write(&config_settings, "{}").expect("write settings");

        let home = root.path().join("home");
        let home_settings = home.join(".claude/settings.json");
        fs::create_dir_all(home_settings.parent().expect("home settings parent"))
            .expect("create home settings dir");
        fs::write(&home_settings, "{}").expect("write home settings");

        let found = discover_host_settings_path_with(
            Some(config_dir.to_string_lossy().as_ref()),
            Some(&home),
        )
        .expect("discover settings path");
        assert_eq!(found, config_settings);
    }

    #[test]
    fn discover_host_settings_falls_back_to_home_claude_dir() {
        let root = tempdir().expect("root tempdir");
        let home = root.path().join("home");
        let home_settings = home.join(".claude/settings.json");
        fs::create_dir_all(home_settings.parent().expect("home settings parent"))
            .expect("create home settings dir");
        fs::write(&home_settings, "{}").expect("write home settings");

        let found =
            discover_host_settings_path_with(None, Some(&home)).expect("discover settings path");
        assert_eq!(found, home_settings);
    }

    #[test]
    fn discover_host_settings_ignores_missing_locations() {
        let root = tempdir().expect("root tempdir");
        let home = root.path().join("home");
        assert!(
            discover_host_settings_path_with(Some("/tmp/does-not-exist"), Some(&home)).is_none()
        );
    }

    #[test]
    fn managed_settings_profile_creates_bypass_permissions_defaults() {
        let root = tempdir().expect("root tempdir");
        let managed_home = root.path().join("managed-home");

        let changed =
            ensure_managed_settings_profile(&managed_home).expect("write managed settings");
        assert!(changed, "expected initial managed settings write");

        let settings: JsonValue = serde_json::from_str(
            &fs::read_to_string(managed_home.join("settings.json")).expect("read settings"),
        )
        .expect("parse settings");
        assert_eq!(
            settings.pointer("/permissions/defaultMode"),
            Some(&JsonValue::String(
                CLAUDE_PERMISSION_DEFAULT_MODE.to_string()
            ))
        );
        assert!(
            settings
                .pointer("/permissions/disableBypassPermissionsMode")
                .is_none(),
            "managed profile should not disable bypass permissions mode"
        );
    }

    #[test]
    fn managed_settings_profile_preserves_existing_config_while_repairing_permissions() {
        let root = tempdir().expect("root tempdir");
        let managed_home = root.path().join("managed-home");
        fs::create_dir_all(&managed_home).expect("create managed home");
        fs::write(
            managed_home.join("settings.json"),
            serde_json::json!({
                "apiKeyHelper": "/usr/local/bin/claude-token",
                "permissions": {
                    "defaultMode": "acceptEdits",
                    "disableBypassPermissionsMode": "disable",
                    "allow": ["Bash(git status)"]
                }
            })
            .to_string(),
        )
        .expect("write existing settings");

        let changed =
            ensure_managed_settings_profile(&managed_home).expect("repair managed settings");
        assert!(changed, "expected permissions repair to rewrite settings");

        let settings: JsonValue = serde_json::from_str(
            &fs::read_to_string(managed_home.join("settings.json")).expect("read settings"),
        )
        .expect("parse settings");
        assert_eq!(
            settings.pointer("/apiKeyHelper"),
            Some(&JsonValue::String(
                "/usr/local/bin/claude-token".to_string()
            ))
        );
        assert_eq!(
            settings.pointer("/permissions/defaultMode"),
            Some(&JsonValue::String(
                CLAUDE_PERMISSION_DEFAULT_MODE.to_string()
            ))
        );
        assert_eq!(
            settings.pointer("/permissions/allow/0"),
            Some(&JsonValue::String("Bash(git status)".to_string()))
        );
        assert!(
            settings
                .pointer("/permissions/disableBypassPermissionsMode")
                .is_none(),
            "managed profile should remove settings that block bypass mode"
        );
    }
}
