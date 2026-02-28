//! OpenClaw runner adapter.
//!
//! This module owns project-local OpenClaw runtime provisioning and optional
//! host import. The core invariant is strict boundary separation:
//! - host OpenClaw home is treated as read-only source material,
//! - managed home/workspace are always created under Agent Ruler runtime root,
//! - setup writes only into managed/runtime-local paths.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde_json::{Map, Value};

use crate::config::{AppConfig, RuntimeState};
use crate::helpers::runners::openclaw::setup_config::{
    apply_tools_adapter_config, disable_session_memory_hook_for_non_anthropic,
    merge_imported_sections, runner_api_base_url, set_gateway_mode_local, set_workspace,
};
use crate::runners::{
    HostInstall, ImportReport, IntegrationOption, IntegrationSelection, ProvisionedPaths,
    RunnerAdapter, RunnerAssociation, RunnerKind, RunnerMissingDecision, RunnerMissingState,
    OPENCLAW_HOME_DIR_NAME, OPENCLAW_WORKSPACE_DIR_NAME, RUNTIME_USER_DATA_DIR_NAME,
};
use crate::utils::resolve_command_path;

const HOST_DEFAULT_DIR_NAME: &str = ".openclaw";
const OPENCLAW_STATE_DIR_NAME: &str = ".openclaw";
const OPENCLAW_CONFIG_FILE_NAME: &str = "openclaw.json";
const OPENCLAW_ENV_FILE_NAME: &str = ".env";
const OPENCLAW_AUTH_PROFILES_FILE_NAME: &str = "auth-profiles.json";
const OPENCLAW_AUTH_STORE_FILE_NAME: &str = "auth.json";
const OPENCLAW_IMPORT_SNAPSHOT_FILE_NAME: &str = "imported-openclaw.json";
const OPENCLAW_IMPORT_DIR_NAME: &str = "imported-host";
const OPENCLAW_RUNTIME_INTEGRATIONS_DIR_NAME: &str = "integrations";
const OPENCLAW_RUNTIME_INTEGRATIONS_SUBDIR: &str = "openclaw";
const OPENCLAW_TOOLS_SCRIPT_FILE_NAME: &str = "enable-agent-ruler-tools.sh";
const OPENCLAW_TOOLS_PLUGIN_ID: &str = "openclaw-agent-ruler-tools";
const OPENCLAW_AGENT_AUTH_PROFILES_RELATIVE: &str = "agents/main/agent/auth-profiles.json";
const OPENCLAW_AGENT_AUTH_STORE_RELATIVE: &str = "agents/main/agent/auth.json";
const OPENCLAW_AGENT_MODELS_RELATIVE: &str = "agents/main/agent/models.json";
const OPENCLAW_AGENTS_MODEL_PRIMARY_POINTER: &str = "/agents/defaults/model/primary";
const OPENCLAW_AGENTS_MODEL_POINTER: &str = "/agents/defaults/model";
const OPENCLAW_TELEGRAM_BOT_TOKEN_POINTER: &str = "/channels/telegram/botToken";
const OPENCLAW_TELEGRAM_TOKEN_POINTER: &str = "/channels/telegram/token";
const OPENCLAW_TELEGRAM_ENABLED_POINTER: &str = "/channels/telegram/enabled";
const IMPORTABLE_ROOT_KEYS: [&str; 12] = [
    "agents", "auth", "bindings", "channels", "commands", "gateway", "hooks", "messages", "models",
    "plugins", "skills", "tools",
];

const INTEGRATION_OPENCLAW_TOOLS: &str = "openclaw_tools_adapter";
const TEST_ALLOW_MISSING_RUNNER_ENV: &str = "AGENT_RULER_TEST_ALLOW_MISSING_RUNNER";

const OPENCLAW_INTEGRATIONS: [IntegrationOption; 1] = [IntegrationOption {
    id: INTEGRATION_OPENCLAW_TOOLS,
    label: "OpenClaw tools adapter",
    detail: "Writes a runtime-local helper under <runtime>/user_data/integrations/openclaw/.",
}];

fn tools_adapter_dir(ruler_root: &Path) -> PathBuf {
    let preferred = ruler_root
        .join("bridge")
        .join("openclaw")
        .join("openclaw-agent-ruler-tools");
    let legacy_scoped = ruler_root
        .join("bridge")
        .join("openclaw")
        .join("tools-adapter");
    let legacy_root = ruler_root.join("bridge").join("openclaw-agent-ruler-tools");
    if preferred.exists() || (!legacy_scoped.exists() && !legacy_root.exists()) {
        preferred
    } else if legacy_scoped.exists() {
        legacy_scoped
    } else {
        legacy_root
    }
}

/// Runner adapter implementation for OpenClaw.
#[derive(Debug, Default)]
pub struct OpenClawAdapter;

impl OpenClawAdapter {
    /// Construct a new OpenClaw adapter instance.
    pub fn new() -> Self {
        Self
    }
}

fn allow_missing_runner_in_test_mode() -> bool {
    std::env::var(TEST_ALLOW_MISSING_RUNNER_ENV)
        .map(|value| {
            let trimmed = value.trim();
            trimmed == "1"
                || trimmed.eq_ignore_ascii_case("true")
                || trimmed.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false)
}

impl RunnerAdapter for OpenClawAdapter {
    fn kind(&self) -> RunnerKind {
        RunnerKind::Openclaw
    }

    fn display_name(&self) -> &'static str {
        "OpenClaw"
    }

    fn detect_host_install(&self, host_hint: Option<&Path>) -> Result<Option<HostInstall>> {
        if let Some(path) = host_hint {
            if looks_like_openclaw_home(path) {
                return Ok(Some(HostInstall {
                    home: path.to_path_buf(),
                    detected_by: "manual path".to_string(),
                }));
            }
            return Ok(None);
        }

        // Respect explicit env override first to support advanced local setups.
        if let Ok(from_env) = std::env::var("OPENCLAW_HOME") {
            let env_home = PathBuf::from(from_env.trim());
            if looks_like_openclaw_home(&env_home) {
                return Ok(Some(HostInstall {
                    home: env_home,
                    detected_by: "OPENCLAW_HOME".to_string(),
                }));
            }
        }

        // Fall back to default host location used by OpenClaw.
        if let Some(home) = dirs::home_dir() {
            let default_home = home.join(HOST_DEFAULT_DIR_NAME);
            if looks_like_openclaw_home(&default_home) {
                return Ok(Some(HostInstall {
                    home: default_home,
                    detected_by: "~/.openclaw".to_string(),
                }));
            }
        }

        Ok(None)
    }

    fn provision_project_paths(&self, runtime: &RuntimeState) -> Result<ProvisionedPaths> {
        let managed_home = runtime
            .config
            .runtime_root
            .join(RUNTIME_USER_DATA_DIR_NAME)
            .join(OPENCLAW_HOME_DIR_NAME);
        // Keep existing workspace path only when it is already runtime-local.
        // Otherwise force a managed workspace under runtime root.
        let managed_workspace = if runtime
            .config
            .workspace
            .starts_with(&runtime.config.runtime_root)
        {
            runtime.config.workspace.clone()
        } else {
            runtime
                .config
                .runtime_root
                .join(RUNTIME_USER_DATA_DIR_NAME)
                .join(OPENCLAW_WORKSPACE_DIR_NAME)
        };

        fs::create_dir_all(&managed_home)
            .with_context(|| format!("create {}", managed_home.display()))?;
        fs::create_dir_all(&managed_workspace)
            .with_context(|| format!("create {}", managed_workspace.display()))?;

        Ok(ProvisionedPaths {
            managed_home,
            managed_workspace,
        })
    }

    fn optional_import_from_host(
        &self,
        host_install: Option<&HostInstall>,
        paths: &ProvisionedPaths,
        import_from_host: bool,
    ) -> Result<ImportReport> {
        if !import_from_host {
            return Ok(ImportReport::default());
        }

        let Some(host_install) = host_install else {
            println!("setup: no host OpenClaw home found; continuing without import");
            return Ok(ImportReport::default());
        };

        let import_dir = paths.managed_home.join(OPENCLAW_IMPORT_DIR_NAME);
        fs::create_dir_all(&import_dir)
            .with_context(|| format!("create {}", import_dir.display()))?;

        let mut report = ImportReport::default();
        let mut snapshot = Map::new();
        snapshot.insert(
            "source_home".to_string(),
            Value::String(host_install.home.to_string_lossy().to_string()),
        );
        snapshot.insert(
            "detected_by".to_string(),
            Value::String(host_install.detected_by.clone()),
        );

        let host_state_root = resolve_openclaw_state_root(&host_install.home);
        snapshot.insert(
            "source_state_root".to_string(),
            Value::String(host_state_root.to_string_lossy().to_string()),
        );

        if let Some(host_config_path) = find_openclaw_config_path(&host_install.home) {
            let raw = fs::read_to_string(&host_config_path)
                .with_context(|| format!("read {}", host_config_path.display()))?;
            let copied_cfg = import_dir.join("openclaw-host.json");
            fs::write(&copied_cfg, &raw)
                .with_context(|| format!("write {}", copied_cfg.display()))?;
            report
                .copied_items
                .push(OPENCLAW_CONFIG_FILE_NAME.to_string());
            snapshot.insert(
                "source_config_path".to_string(),
                Value::String(host_config_path.to_string_lossy().to_string()),
            );

            match json5::from_str::<Value>(&raw) {
                Ok(parsed) => {
                    let extracted = extract_importable_config(&parsed);
                    if !extracted.is_empty() {
                        let telegram_enabled = parsed
                            .pointer(OPENCLAW_TELEGRAM_ENABLED_POINTER)
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        report.expected_telegram_token =
                            telegram_enabled || config_has_telegram_token(&parsed);
                        report.expected_model_primary = parsed
                            .pointer(OPENCLAW_AGENTS_MODEL_PRIMARY_POINTER)
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToOwned::to_owned)
                            .or_else(|| {
                                parsed
                                    .pointer(OPENCLAW_AGENTS_MODEL_POINTER)
                                    .and_then(Value::as_str)
                                    .map(str::trim)
                                    .filter(|value| !value.is_empty())
                                    .map(ToOwned::to_owned)
                            });
                        snapshot.insert(
                            "extracted_config".to_string(),
                            Value::Object(extracted.clone()),
                        );
                        if let Some(model) = report.expected_model_primary.as_ref() {
                            snapshot.insert(
                                "expected_model_primary".to_string(),
                                Value::String(model.clone()),
                            );
                        }
                        report.imported_config = Some(Value::Object(extracted));
                    } else {
                        snapshot.insert("extracted_config".to_string(), Value::Object(Map::new()));
                    }
                }
                Err(err) => {
                    // Snapshot parse failures for setup diagnostics, but do not
                    // mutate host files or abort whole import on parse-only issues.
                    snapshot.insert("parse_error".to_string(), Value::String(err.to_string()));
                }
            }
        }

        report.expected_auth_profiles = host_state_root
            .join(OPENCLAW_AGENT_AUTH_PROFILES_RELATIVE)
            .is_file();
        report.expected_auth_store = host_state_root
            .join(OPENCLAW_AGENT_AUTH_STORE_RELATIVE)
            .is_file();

        if let Some(host_env) = find_openclaw_env_path(&host_install.home) {
            let managed_env = paths
                .managed_home
                .join(OPENCLAW_STATE_DIR_NAME)
                .join(OPENCLAW_ENV_FILE_NAME);
            if let Some(parent) = managed_env.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            fs::copy(&host_env, &managed_env).with_context(|| {
                format!("copy {} to {}", host_env.display(), managed_env.display())
            })?;
            let copied_env = import_dir.join("host.env");
            fs::copy(&host_env, &copied_env).with_context(|| {
                format!("copy {} to {}", host_env.display(), copied_env.display())
            })?;
            report.copied_items.push(OPENCLAW_ENV_FILE_NAME.to_string());
        }

        // Clone selected host state into managed state tree. The copy filter
        // excludes ephemeral session/history artifacts.
        let cloned = copy_host_state_tree(&host_install.home, &paths.managed_home)
            .context("clone host OpenClaw state into managed home")?;
        report.cloned_configs = cloned.clone();
        if report.expected_auth_profiles {
            report
                .copied_items
                .push(OPENCLAW_AUTH_PROFILES_FILE_NAME.to_string());
        }
        if report.expected_auth_store {
            report
                .copied_items
                .push(OPENCLAW_AUTH_STORE_FILE_NAME.to_string());
        }
        if !cloned.is_empty() {
            snapshot.insert(
                "cloned_paths".to_string(),
                Value::Array(
                    cloned
                        .iter()
                        .map(|path| Value::String(path.clone()))
                        .collect(),
                ),
            );
        }

        report.imported = !report.copied_items.is_empty() || !report.cloned_configs.is_empty();
        snapshot.insert(
            "copied_items".to_string(),
            Value::Array(
                report
                    .copied_items
                    .iter()
                    .map(|item| Value::String(item.clone()))
                    .collect(),
            ),
        );

        let snapshot_path = paths.managed_home.join(OPENCLAW_IMPORT_SNAPSHOT_FILE_NAME);
        fs::write(
            &snapshot_path,
            serde_json::to_string_pretty(&Value::Object(snapshot))
                .context("serialize OpenClaw import snapshot")?,
        )
        .with_context(|| format!("write {}", snapshot_path.display()))?;
        report.snapshot_path = Some(snapshot_path);

        Ok(report)
    }

    fn write_runner_config(
        &self,
        runtime: &RuntimeState,
        config: &mut AppConfig,
        paths: &ProvisionedPaths,
        import_report: &ImportReport,
        integrations: &[IntegrationSelection],
    ) -> Result<()> {
        let config_path = preferred_openclaw_config_path(&paths.managed_home);
        if !config_path.exists() {
            // Bootstrap writes only to managed home. Host OpenClaw state is never
            // modified in this setup path.
            bootstrap_managed_home(paths)?;
        }
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }

        let mut openclaw_config = read_config_map_or_default(&config_path)?;
        merge_imported_sections(
            &mut openclaw_config,
            import_report.imported_config.as_ref(),
            &IMPORTABLE_ROOT_KEYS,
        );
        set_workspace(&mut openclaw_config, &paths.managed_workspace);
        set_gateway_mode_local(&mut openclaw_config);
        disable_session_memory_hook_for_non_anthropic(&mut openclaw_config);
        // The OpenClaw tools adapter is mandatory for deterministic preflight
        // mediation of native file/exec tools into Agent Ruler receipts.
        apply_tools_adapter_config(runtime, &mut openclaw_config, OPENCLAW_TOOLS_PLUGIN_ID);

        fs::write(
            &config_path,
            serde_json::to_string_pretty(&Value::Object(openclaw_config))
                .context("serialize OpenClaw config")?,
        )
        .with_context(|| format!("write {}", config_path.display()))?;

        verify_import_requirements(paths, import_report)?;

        apply_integrations(runtime, paths, integrations)?;

        let mut integration_ids: BTreeSet<String> =
            integrations.iter().map(|item| item.id.clone()).collect();
        integration_ids.insert(INTEGRATION_OPENCLAW_TOOLS.to_string());

        config.runner = Some(RunnerAssociation {
            kind: RunnerKind::Openclaw,
            managed_home: paths.managed_home.clone(),
            managed_workspace: paths.managed_workspace.clone(),
            integrations: integration_ids.into_iter().collect(),
            missing: if allow_missing_runner_in_test_mode()
                && resolve_command_path(RunnerKind::Openclaw.executable_name()).is_none()
            {
                RunnerMissingState {
                    executable_missing: true,
                    decision: Some(RunnerMissingDecision::KeepData),
                }
            } else {
                RunnerMissingState::default()
            },
        });
        Ok(())
    }

    fn validate(&self, config: &AppConfig) -> Result<()> {
        let runner = config
            .runner
            .as_ref()
            .ok_or_else(|| anyhow!("runner config missing after setup"))?;
        if runner.kind != RunnerKind::Openclaw {
            return Err(anyhow!("runner kind mismatch; expected openclaw"));
        }
        if !runner.managed_home.exists() {
            return Err(anyhow!(
                "managed OpenClaw home is missing at {}",
                runner.managed_home.display()
            ));
        }
        if !runner.managed_workspace.exists() {
            return Err(anyhow!(
                "managed OpenClaw workspace is missing at {}",
                runner.managed_workspace.display()
            ));
        }
        if !runner.managed_home.starts_with(&config.runtime_root) {
            return Err(anyhow!(
                "managed OpenClaw home must stay under runtime root {}",
                config.runtime_root.display()
            ));
        }
        if !runner.managed_workspace.starts_with(&config.runtime_root) {
            return Err(anyhow!(
                "managed OpenClaw workspace must stay under runtime root {}",
                config.runtime_root.display()
            ));
        }

        let config_path = find_openclaw_config_path(&runner.managed_home).ok_or_else(|| {
            anyhow!(
                "generated OpenClaw config is missing under {}",
                runner.managed_home.display()
            )
        })?;
        let raw = fs::read_to_string(&config_path)
            .with_context(|| format!("read {}", config_path.display()))?;
        let parsed: Value = json5::from_str(&raw).context("parse generated OpenClaw config")?;
        validate_managed_config(&parsed, &runner.managed_workspace).with_context(|| {
            format!(
                "validate generated OpenClaw config at {}",
                config_path.display()
            )
        })?;
        validate_tools_adapter_wiring(
            &parsed,
            &config.ruler_root,
            &config.ui_bind,
            config.approval_wait_timeout_secs,
        )
        .with_context(|| {
            format!(
                "validate Agent Ruler tools adapter wiring in {}",
                config_path.display()
            )
        })?;

        Ok(())
    }

    fn print_next_steps(&self, _runtime: &RuntimeState, config: &AppConfig) {
        let Some(runner) = config.runner.as_ref() else {
            return;
        };
        println!("setup complete: OpenClaw runner configured");
        println!(
            "ruler-managed OpenClaw home: {}",
            runner.managed_home.display()
        );
        println!(
            "ruler-managed OpenClaw workspace: {}",
            runner.managed_workspace.display()
        );
        println!("next command:");
        println!("agent-ruler run -- openclaw gateway");
        println!("(Agent Ruler sets OPENCLAW_HOME to the managed home automatically.)");
    }

    fn integration_options(&self) -> &'static [IntegrationOption] {
        &OPENCLAW_INTEGRATIONS
    }
}

fn looks_like_openclaw_home(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }

    if find_openclaw_config_path(path).is_some() {
        return true;
    }

    let markers = [
        path.join(OPENCLAW_ENV_FILE_NAME),
        path.join("channels"),
        path.join("plugins"),
        path.join(OPENCLAW_STATE_DIR_NAME).join("channels"),
        path.join(OPENCLAW_STATE_DIR_NAME).join("plugins"),
    ];
    // Detection is intentionally broad because host installs vary by version.
    // False positives are still safe: setup imports read-only snapshots first.
    markers.iter().any(|entry| entry.exists())
}

fn openclaw_config_candidates(home: &Path) -> [PathBuf; 2] {
    [
        home.join(OPENCLAW_STATE_DIR_NAME)
            .join(OPENCLAW_CONFIG_FILE_NAME),
        home.join(OPENCLAW_CONFIG_FILE_NAME),
    ]
}

fn preferred_openclaw_config_path(home: &Path) -> PathBuf {
    find_openclaw_config_path(home).unwrap_or_else(|| {
        home.join(OPENCLAW_STATE_DIR_NAME)
            .join(OPENCLAW_CONFIG_FILE_NAME)
    })
}

fn find_openclaw_config_path(home: &Path) -> Option<PathBuf> {
    openclaw_config_candidates(home)
        .into_iter()
        .find(|candidate| candidate.exists())
}

fn find_openclaw_env_path(home: &Path) -> Option<PathBuf> {
    let candidates = [
        home.join(OPENCLAW_ENV_FILE_NAME),
        home.join(OPENCLAW_STATE_DIR_NAME)
            .join(OPENCLAW_ENV_FILE_NAME),
    ];
    candidates.into_iter().find(|candidate| candidate.exists())
}

fn resolve_openclaw_state_root(home: &Path) -> PathBuf {
    let nested = home.join(OPENCLAW_STATE_DIR_NAME);
    if nested.is_dir() {
        nested
    } else {
        home.to_path_buf()
    }
}

fn copy_host_state_tree(host_home: &Path, managed_home: &Path) -> Result<Vec<String>> {
    let source_root = resolve_openclaw_state_root(host_home);
    if !source_root.exists() {
        return Ok(Vec::new());
    }
    let destination_root = managed_home.join(OPENCLAW_STATE_DIR_NAME);
    if destination_root.exists() {
        fs::remove_dir_all(&destination_root)
            .with_context(|| format!("remove {}", destination_root.display()))?;
    }
    let mut copied = Vec::new();
    copy_directory_recursively_filtered(
        &source_root,
        &destination_root,
        &source_root,
        &mut copied,
    )?;
    copied.sort();
    copied.dedup();
    Ok(copied)
}

fn copy_directory_recursively_filtered(
    source: &Path,
    destination: &Path,
    source_root: &Path,
    copied: &mut Vec<String>,
) -> Result<()> {
    fs::create_dir_all(destination).with_context(|| format!("create {}", destination.display()))?;

    for entry in fs::read_dir(source).with_context(|| format!("read {}", source.display()))? {
        let entry = entry.with_context(|| format!("read entry under {}", source.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("read file type {}", entry.path().display()))?;
        let src = entry.path();
        let relative = src.strip_prefix(source_root).with_context(|| {
            format!(
                "strip source root {} from {}",
                source_root.display(),
                src.display()
            )
        })?;

        if should_skip_import_entry(relative, file_type.is_dir()) {
            continue;
        }

        let dest = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_directory_recursively_filtered(&src, &dest, source_root, copied)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::copy(&src, &dest)
            .with_context(|| format!("copy {} to {}", src.display(), dest.display()))?;
        copied.push(
            Path::new(OPENCLAW_STATE_DIR_NAME)
                .join(relative)
                .to_string_lossy()
                .to_string(),
        );
    }

    Ok(())
}

fn should_skip_import_entry(relative: &Path, is_dir: bool) -> bool {
    let normalized = relative.to_string_lossy().replace('\\', "/");
    if normalized.is_empty() {
        return false;
    }
    // Session trees are intentionally excluded so managed homes do not inherit
    // stale host conversation/session identity.
    if normalized.starts_with("agents/")
        && (normalized.contains("/sessions/") || normalized.ends_with("/sessions"))
    {
        return true;
    }
    // Runtime logs/workspace are local execution artifacts and should not be imported.
    if is_dir && (normalized == "logs" || normalized == "workspace") {
        return true;
    }
    normalized.starts_with("agents/main/sessions/") && normalized.ends_with(".jsonl")
}

fn read_config_map_or_default(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let parsed: Value =
        json5::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    Ok(parsed.as_object().cloned().unwrap_or_default())
}

fn extract_importable_config(parsed: &Value) -> Map<String, Value> {
    let mut extracted = Map::new();
    let Some(root) = parsed.as_object() else {
        return extracted;
    };

    for key in IMPORTABLE_ROOT_KEYS {
        if let Some(value) = root.get(key) {
            extracted.insert(key.to_string(), value.clone());
        }
    }

    extracted
}

/// Re-apply session-memory provider guard on managed config.
///
/// Returns `true` when config changed and was persisted.
pub fn enforce_session_memory_hook_guard(managed_home: &Path) -> Result<bool> {
    let Some(config_path) = find_openclaw_config_path(managed_home) else {
        return Ok(false);
    };
    let mut config = read_config_map_or_default(&config_path)?;
    let before = config.clone();
    disable_session_memory_hook_for_non_anthropic(&mut config);
    if config == before {
        return Ok(false);
    }
    fs::write(
        &config_path,
        serde_json::to_string_pretty(&Value::Object(config))
            .context("serialize OpenClaw config guard update")?,
    )
    .with_context(|| format!("write {}", config_path.display()))?;
    Ok(true)
}

/// Ensure managed OpenClaw config still contains Agent Ruler tools adapter wiring.
///
/// This guard fixes config drift when OpenClaw rewrites `openclaw.json` and drops
/// plugin load/entry keys, which would otherwise bypass Agent Ruler preflight
/// receipts for native tool calls.
pub fn enforce_tools_adapter_config_guard(runtime: &RuntimeState) -> Result<bool> {
    let managed_home = runtime
        .config
        .runner
        .as_ref()
        .filter(|runner| runner.kind == RunnerKind::Openclaw)
        .map(|runner| runner.managed_home.clone())
        .unwrap_or_else(|| {
            runtime
                .config
                .runtime_root
                .join(RUNTIME_USER_DATA_DIR_NAME)
                .join(OPENCLAW_HOME_DIR_NAME)
        });

    let Some(config_path) = find_openclaw_config_path(&managed_home) else {
        return Ok(false);
    };

    let mut config = read_config_map_or_default(&config_path)?;
    let before = config.clone();
    apply_tools_adapter_config(runtime, &mut config, OPENCLAW_TOOLS_PLUGIN_ID);
    if config == before {
        return Ok(false);
    }

    fs::write(
        &config_path,
        serde_json::to_string_pretty(&Value::Object(config))
            .context("serialize OpenClaw tools adapter guard update")?,
    )
    .with_context(|| format!("write {}", config_path.display()))?;
    Ok(true)
}

fn validate_managed_config(parsed: &Value, expected_workspace: &Path) -> Result<()> {
    let root = parsed
        .as_object()
        .ok_or_else(|| anyhow!("managed OpenClaw config root must be an object"))?;

    let mode = root
        .get("gateway")
        .and_then(Value::as_object)
        .and_then(|gateway| gateway.get("mode"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("`gateway.mode` must be set"))?;
    if mode != "local" {
        return Err(anyhow!(
            "`gateway.mode` must be `local` for managed OpenClaw config (got `{}`)",
            mode
        ));
    }

    let workspace = root
        .get("agents")
        .and_then(Value::as_object)
        .and_then(|agents| agents.get("defaults"))
        .and_then(Value::as_object)
        .and_then(|defaults| defaults.get("workspace"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("`agents.defaults.workspace` must be a string"))?;

    let expected = expected_workspace.to_string_lossy();
    if workspace != expected {
        return Err(anyhow!(
            "generated OpenClaw workspace mismatch (expected {}, got {})",
            expected,
            workspace
        ));
    }

    Ok(())
}

/// Confirm the managed OpenClaw config still binds the Agent Ruler tools plugin.
/// This guard prevents OpenClaw from dropping the plugin entry/load path that backs the
/// `/api/openclaw/tool/preflight` hook, ensuring receipts keep reflecting tool mediation.
fn validate_tools_adapter_wiring(
    parsed: &Value,
    ruler_root: &Path,
    ui_bind: &str,
    approval_wait_timeout_secs: u64,
) -> Result<()> {
    let plugin_entry = format!("/plugins/entries/{}", OPENCLAW_TOOLS_PLUGIN_ID);
    let enabled = parsed
        .pointer(&format!("{}/enabled", plugin_entry))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !enabled {
        return Err(anyhow!(
            "managed OpenClaw config must enable `plugins.entries.{}.enabled`",
            OPENCLAW_TOOLS_PLUGIN_ID
        ));
    }

    let expected_base_url = runner_api_base_url(ui_bind);
    let actual_base_url = parsed
        .pointer(&format!("{}/config/baseUrl", plugin_entry))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            anyhow!(
                "managed OpenClaw config must set `plugins.entries.{}.config.baseUrl`",
                OPENCLAW_TOOLS_PLUGIN_ID
            )
        })?;
    if actual_base_url != expected_base_url {
        return Err(anyhow!(
            "managed OpenClaw config tools adapter baseUrl mismatch (expected `{}`, got `{}`)",
            expected_base_url,
            actual_base_url
        ));
    }

    let expected_wait_timeout = approval_wait_timeout_secs.clamp(1, 300);
    let actual_wait_timeout = parsed
        .pointer(&format!("{}/config/approvalWaitTimeoutSecs", plugin_entry))
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            anyhow!(
                "managed OpenClaw config must set `plugins.entries.{}.config.approvalWaitTimeoutSecs`",
                OPENCLAW_TOOLS_PLUGIN_ID
            )
        })?;
    if actual_wait_timeout != expected_wait_timeout {
        return Err(anyhow!(
            "managed OpenClaw config approval wait timeout mismatch (expected `{}`, got `{}`)",
            expected_wait_timeout,
            actual_wait_timeout
        ));
    }

    let expected_plugin_path = tools_adapter_dir(ruler_root).to_string_lossy().to_string();
    let load_paths = parsed
        .pointer("/plugins/load/paths")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("managed OpenClaw config must include `plugins.load.paths`"))?;
    if !load_paths
        .iter()
        .any(|entry| entry.as_str() == Some(expected_plugin_path.as_str()))
    {
        return Err(anyhow!(
            "managed OpenClaw config is missing Agent Ruler plugin path `{}` in `plugins.load.paths`",
            expected_plugin_path
        ));
    }

    let allow = parsed
        .pointer("/agents/list/0/tools/allow")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow!("managed OpenClaw config must include `agents.list[0].tools.allow`")
        })?;
    if !allow
        .iter()
        .any(|entry| entry.as_str() == Some(OPENCLAW_TOOLS_PLUGIN_ID))
    {
        return Err(anyhow!(
            "managed OpenClaw config must allow `{}` in `agents.list[0].tools.allow`",
            OPENCLAW_TOOLS_PLUGIN_ID
        ));
    }

    Ok(())
}

fn verify_import_requirements(paths: &ProvisionedPaths, report: &ImportReport) -> Result<()> {
    if !report.imported {
        return Ok(());
    }

    // Import is only considered successful when key auth/model assets expected
    // from host state are present and resolvable in managed config.
    let auth_profiles_path = paths
        .managed_home
        .join(OPENCLAW_STATE_DIR_NAME)
        .join(OPENCLAW_AGENT_AUTH_PROFILES_RELATIVE);
    if report.expected_auth_profiles && !auth_profiles_path.is_file() {
        return Err(anyhow!(
            "setup import incomplete: missing `{}`. Expected managed auth profiles at {}. Re-run `agent-ruler setup` and choose import again.",
            OPENCLAW_AUTH_PROFILES_FILE_NAME,
            auth_profiles_path.display()
        ));
    }

    let auth_store_path = paths
        .managed_home
        .join(OPENCLAW_STATE_DIR_NAME)
        .join(OPENCLAW_AGENT_AUTH_STORE_RELATIVE);
    if report.expected_auth_store && !auth_store_path.is_file() {
        return Err(anyhow!(
            "setup import incomplete: missing `{}`. Expected managed auth store at {}. Re-run `agent-ruler setup` and choose import again.",
            OPENCLAW_AUTH_STORE_FILE_NAME,
            auth_store_path.display()
        ));
    }

    if report.expected_telegram_token && !managed_config_has_telegram_token(paths)? {
        return Err(anyhow!(
            "setup import incomplete: Telegram token was detected in host config but missing in managed config (`channels.telegram.botToken` or `channels.telegram.token`). Re-run `agent-ruler setup` and choose import again."
        ));
    }

    let selected_model = managed_selected_model(paths)?.ok_or_else(|| {
        anyhow!(
            "setup import incomplete: managed config is missing `agents.defaults.model.primary` (or `agents.defaults.model`). Re-run `agent-ruler setup` and choose import again."
        )
    })?;
    if let Some(expected_model) = report.expected_model_primary.as_ref() {
        if &selected_model != expected_model {
            return Err(anyhow!(
                "setup import incomplete: managed selected model changed from `{}` to `{}`. Re-run `agent-ruler setup` and choose import again.",
                expected_model,
                selected_model
            ));
        }
    }
    if !managed_model_reference_resolvable(paths, &selected_model)? {
        return Err(anyhow!(
            "setup import incomplete: selected model `{}` is not resolvable from managed provider/model config. Expected model definitions in `{}` or `{}`.",
            selected_model,
            paths
                .managed_home
                .join(OPENCLAW_STATE_DIR_NAME)
                .join(OPENCLAW_CONFIG_FILE_NAME)
                .display(),
            paths
                .managed_home
                .join(OPENCLAW_STATE_DIR_NAME)
                .join(OPENCLAW_AGENT_MODELS_RELATIVE)
                .display()
        ));
    }

    Ok(())
}

fn read_managed_config(managed_home: &Path) -> Result<Option<(PathBuf, Value)>> {
    let Some(config_path) = find_openclaw_config_path(managed_home) else {
        return Ok(None);
    };
    let raw = fs::read_to_string(&config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let parsed: Value =
        json5::from_str(&raw).with_context(|| format!("parse {}", config_path.display()))?;
    Ok(Some((config_path, parsed)))
}

fn managed_config_has_telegram_token(paths: &ProvisionedPaths) -> Result<bool> {
    let Some((_, parsed)) = read_managed_config(&paths.managed_home)? else {
        return Ok(false);
    };
    Ok(config_has_telegram_token(&parsed))
}

fn config_has_telegram_token(config: &Value) -> bool {
    for pointer in [
        OPENCLAW_TELEGRAM_BOT_TOKEN_POINTER,
        OPENCLAW_TELEGRAM_TOKEN_POINTER,
    ] {
        if config
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(|token| !token.trim().is_empty())
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

fn managed_selected_model(paths: &ProvisionedPaths) -> Result<Option<String>> {
    let Some((_, parsed)) = read_managed_config(&paths.managed_home)? else {
        return Ok(None);
    };
    let primary = parsed
        .pointer(OPENCLAW_AGENTS_MODEL_PRIMARY_POINTER)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if primary.is_some() {
        return Ok(primary);
    }
    Ok(parsed
        .pointer(OPENCLAW_AGENTS_MODEL_POINTER)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned))
}

fn managed_model_reference_resolvable(paths: &ProvisionedPaths, model_ref: &str) -> Result<bool> {
    let Some((_, parsed)) = read_managed_config(&paths.managed_home)? else {
        return Ok(false);
    };
    if model_ref.trim().is_empty() {
        return Ok(false);
    }

    let Some((provider, model_id)) = model_ref.split_once('/') else {
        return Ok(false);
    };

    if provider_model_exists_in_models_value(&parsed, provider, model_id) {
        return Ok(true);
    }

    let models_path = paths
        .managed_home
        .join(OPENCLAW_STATE_DIR_NAME)
        .join(OPENCLAW_AGENT_MODELS_RELATIVE);
    if !models_path.is_file() {
        return Ok(false);
    }

    let raw = fs::read_to_string(&models_path)
        .with_context(|| format!("read {}", models_path.display()))?;
    let parsed_models: Value =
        json5::from_str(&raw).with_context(|| format!("parse {}", models_path.display()))?;
    Ok(provider_model_exists_in_models_value(
        &parsed_models,
        provider,
        model_id,
    ))
}

fn provider_model_exists_in_models_value(value: &Value, provider: &str, model_id: &str) -> bool {
    let pointers = [
        format!("/models/providers/{provider}/models"),
        format!("/providers/{provider}/models"),
    ];
    pointers.iter().any(|pointer| {
        value
            .pointer(pointer)
            .and_then(Value::as_array)
            .map(|models| {
                models.iter().any(|entry| {
                    entry
                        .get("id")
                        .and_then(Value::as_str)
                        .map(|id| id == model_id)
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Telegram channel status extracted from managed OpenClaw config.
pub struct ManagedTelegramConfigStatus {
    pub configured: bool,
    pub enabled: bool,
    pub token_present: bool,
}

/// Inspect managed config and summarize Telegram channel readiness.
pub fn inspect_managed_telegram_config(managed_home: &Path) -> Result<ManagedTelegramConfigStatus> {
    let Some((_, parsed)) = read_managed_config(managed_home)? else {
        return Ok(ManagedTelegramConfigStatus {
            configured: false,
            enabled: false,
            token_present: false,
        });
    };
    let enabled = parsed
        .pointer(OPENCLAW_TELEGRAM_ENABLED_POINTER)
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let token_present = config_has_telegram_token(&parsed);
    Ok(ManagedTelegramConfigStatus {
        configured: parsed.pointer("/channels/telegram").is_some(),
        enabled,
        token_present,
    })
}

fn managed_config_is_gateway_ready(paths: &ProvisionedPaths) -> Result<bool> {
    let Some(config_path) = find_openclaw_config_path(&paths.managed_home) else {
        return Ok(false);
    };
    let raw = fs::read_to_string(&config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let parsed: Value =
        json5::from_str(&raw).with_context(|| format!("parse {}", config_path.display()))?;
    Ok(validate_managed_config(&parsed, &paths.managed_workspace).is_ok())
}

fn bootstrap_managed_home(paths: &ProvisionedPaths) -> Result<()> {
    if managed_config_is_gateway_ready(paths)? {
        return Ok(());
    }

    let workspace = paths.managed_workspace.to_string_lossy().to_string();
    let attempts: [Vec<String>; 2] = [
        vec![
            "onboard".to_string(),
            "--non-interactive".to_string(),
            "--accept-risk".to_string(),
            "--mode".to_string(),
            "local".to_string(),
            "--workspace".to_string(),
            workspace.clone(),
            "--skip-channels".to_string(),
            "--skip-skills".to_string(),
            "--skip-ui".to_string(),
            "--skip-health".to_string(),
            "--skip-daemon".to_string(),
            "--auth-choice".to_string(),
            "skip".to_string(),
            "--json".to_string(),
        ],
        vec![
            "setup".to_string(),
            "--non-interactive".to_string(),
            "--mode".to_string(),
            "local".to_string(),
            "--workspace".to_string(),
            workspace.clone(),
        ],
    ];

    let mut attempt_errors: Vec<String> = Vec::new();
    for args in attempts {
        match run_openclaw_command(paths, &args) {
            Ok(output) if output.status.success() => {
                if managed_config_is_gateway_ready(paths)? {
                    return Ok(());
                }
                // Successful command execution is not enough; setup requires
                // deterministic gateway-ready config shape.
                attempt_errors.push(format!(
                    "`openclaw {}` succeeded but managed config was still not gateway-ready",
                    args.join(" ")
                ));
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let detail = if !stderr.is_empty() { stderr } else { stdout };
                attempt_errors.push(format!(
                    "`openclaw {}` failed with status {}{}",
                    args.join(" "),
                    output.status,
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", detail)
                    }
                ));
            }
            Err(err) => {
                if err
                    .downcast_ref::<std::io::Error>()
                    .map(|io_err| io_err.kind() == std::io::ErrorKind::NotFound)
                    .unwrap_or(false)
                {
                    if allow_missing_runner_in_test_mode() {
                        eprintln!(
                            "setup: `{}` not found in PATH; continuing because {} is enabled (test-only mode)",
                            RunnerKind::Openclaw.executable_name(),
                            TEST_ALLOW_MISSING_RUNNER_ENV
                        );
                        return Ok(());
                    }
                    return Err(anyhow!(
                        "`openclaw` executable was not found in PATH. Install OpenClaw first, then rerun `agent-ruler setup`."
                    ));
                }
                attempt_errors.push(err.to_string());
            }
        }
    }

    let manual_cmd = format!(
        "OPENCLAW_HOME=\"{}\" openclaw onboard --mode local --workspace \"{}\"",
        paths.managed_home.display(),
        paths.managed_workspace.display()
    );

    if io::stdin().is_terminal() {
        // Interactive fallback is opt-in so non-interactive setup remains deterministic.
        println!(
            "setup: non-interactive OpenClaw bootstrap failed; managed home is not gateway-ready."
        );
        println!("setup: run interactive bootstrap now? [Y/n]");
        if prompt_yes_no(true)? {
            let status = Command::new("openclaw")
                .env("OPENCLAW_HOME", &paths.managed_home)
                .arg("onboard")
                .arg("--mode")
                .arg("local")
                .arg("--workspace")
                .arg(&paths.managed_workspace)
                .status()
                .context("run interactive OpenClaw bootstrap")?;
            if !status.success() {
                return Err(anyhow!(
                    "interactive OpenClaw bootstrap failed (status: {}). Run: {}",
                    status,
                    manual_cmd
                ));
            }
            if managed_config_is_gateway_ready(paths)? {
                return Ok(());
            }
            return Err(anyhow!(
                "interactive OpenClaw bootstrap completed but managed config is still not gateway-ready. Run: {}",
                manual_cmd
            ));
        }
    }

    Err(anyhow!(
        "unable to bootstrap managed OpenClaw home non-interactively. {}. Run: {}",
        attempt_errors.join(" | "),
        manual_cmd
    ))
}

fn run_openclaw_command(paths: &ProvisionedPaths, args: &[String]) -> Result<std::process::Output> {
    Command::new("openclaw")
        .env("OPENCLAW_HOME", &paths.managed_home)
        .args(args)
        .output()
        .with_context(|| format!("run `openclaw {}`", args.join(" ")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Listener metadata for an occupied gateway port.
pub struct GatewayListenerInfo {
    pub pid: Option<u32>,
    pub openclaw_home: Option<String>,
    pub ss_line: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Diagnostics payload returned when gateway startup likely failed due to port conflict.
pub struct GatewayPortDiagnostics {
    pub port: Option<u16>,
    pub listeners: Vec<GatewayListenerInfo>,
}

/// Parse gateway startup output and enrich "port in use" failures with listener ownership data.
pub fn maybe_collect_gateway_port_diagnostics(
    managed_home: &Path,
    stdout: &str,
    stderr: &str,
) -> Result<Option<GatewayPortDiagnostics>> {
    let combined = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    if !is_port_in_use_text(&combined) {
        return Ok(None);
    }

    // Prefer explicit port in failure output, then fallback to configured gateway.port.
    let port = extract_port_hint(stdout)
        .or_else(|| extract_port_hint(stderr))
        .or_else(|| query_gateway_port(managed_home));

    let listeners = if let Some(port) = port {
        collect_gateway_listeners(port)?
    } else {
        Vec::new()
    };

    Ok(Some(GatewayPortDiagnostics { port, listeners }))
}

/// Locate a live OpenClaw gateway listener PID for the provided managed home.
///
/// This is used by detached gateway startup recovery when OpenClaw does not
/// emit a parseable daemon PID line quickly enough.
pub fn find_managed_gateway_listener_pid(managed_home: &Path) -> Result<Option<u32>> {
    let output = Command::new("ss")
        .args(["-ltnp"])
        .output()
        .context("run `ss -ltnp` while resolving managed gateway listener")?;
    if !output.status.success() {
        return Ok(None);
    }

    let expected_home = managed_home.to_string_lossy().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut fallback: Option<u32> = None;
    for line in stdout.lines() {
        if !line.contains("openclaw-gateway") {
            continue;
        }
        let Some(pid) = parse_pid_from_ss_line(line) else {
            continue;
        };
        if let Some(home) = read_openclaw_home_from_proc(pid) {
            if home == expected_home {
                return Ok(Some(pid));
            }
        } else if fallback.is_none() {
            // `/proc/<pid>/environ` may be unreadable momentarily; keep a
            // single best-effort fallback for short-lived detection windows.
            fallback = Some(pid);
        }
    }
    Ok(fallback)
}

fn is_port_in_use_text(text: &str) -> bool {
    text.contains("address already in use")
        || text.contains("port in use")
        || text.contains("already listening")
        || text.contains("eaddrinuse")
}

fn extract_port_hint(text: &str) -> Option<u16> {
    for token in text.split_whitespace() {
        let value = token.trim_matches(|c: char| {
            !(c.is_ascii_alphanumeric() || c == ':' || c == '.' || c == '[' || c == ']')
        });
        if let Some(port) = extract_port_from_token(value) {
            return Some(port);
        }
    }
    None
}

fn extract_port_from_token(token: &str) -> Option<u16> {
    let index = token.rfind(':')?;
    let candidate = &token[index + 1..];
    if candidate.is_empty() || candidate.len() > 5 || !candidate.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let port = candidate.parse::<u16>().ok()?;
    if port == 0 {
        return None;
    }
    Some(port)
}

fn query_gateway_port(managed_home: &Path) -> Option<u16> {
    let output = Command::new("openclaw")
        .env("OPENCLAW_HOME", managed_home)
        .args(["config", "get", "gateway.port"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u16>()
        .ok()
}

fn collect_gateway_listeners(port: u16) -> Result<Vec<GatewayListenerInfo>> {
    let output = Command::new("ss")
        .args(["-ltnp"])
        .output()
        .context("run `ss -ltnp` for gateway diagnostics")?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let mut listeners = parse_ss_output_for_port(&stdout, port);
    for listener in &mut listeners {
        if let Some(pid) = listener.pid {
            listener.openclaw_home = read_openclaw_home_from_proc(pid);
        }
    }
    Ok(listeners)
}

fn parse_ss_output_for_port(output: &str, port: u16) -> Vec<GatewayListenerInfo> {
    output
        .lines()
        .filter(|line| ss_line_mentions_port(line, port))
        .map(|line| GatewayListenerInfo {
            pid: parse_pid_from_ss_line(line),
            openclaw_home: None,
            ss_line: line.trim().to_string(),
        })
        .collect()
}

fn ss_line_mentions_port(line: &str, port: u16) -> bool {
    let needle = format!(":{port}");
    let mut start = 0usize;
    while let Some(index) = line[start..].find(&needle) {
        let absolute = start + index;
        let next = line[absolute + needle.len()..].chars().next();
        if !next.map(|value| value.is_ascii_digit()).unwrap_or(false) {
            return true;
        }
        start = absolute + needle.len();
    }
    false
}

fn parse_pid_from_ss_line(line: &str) -> Option<u32> {
    let marker = "pid=";
    let start = line.find(marker)? + marker.len();
    let digits: String = line[start..]
        .chars()
        .take_while(|value| value.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u32>().ok()
}

fn read_openclaw_home_from_proc(pid: u32) -> Option<String> {
    let path = PathBuf::from(format!("/proc/{pid}/environ"));
    let bytes = fs::read(path).ok()?;
    parse_openclaw_home_from_environ(&bytes)
}

fn parse_openclaw_home_from_environ(raw: &[u8]) -> Option<String> {
    raw.split(|value| *value == 0)
        .filter_map(|entry| std::str::from_utf8(entry).ok())
        .find_map(|entry| {
            entry
                .strip_prefix("OPENCLAW_HOME=")
                .map(|value| value.to_string())
        })
}

fn prompt_yes_no(default_yes: bool) -> Result<bool> {
    let suffix = if default_yes { "[Y/n]" } else { "[y/N]" };
    print!("Selection {}: ", suffix);
    io::stdout().flush().context("flush stdout")?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("read stdin line")?;
    let value = input.trim().to_ascii_lowercase();
    if value.is_empty() {
        return Ok(default_yes);
    }
    match value.as_str() {
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        _ => Ok(default_yes),
    }
}

fn apply_integrations(
    runtime: &RuntimeState,
    paths: &ProvisionedPaths,
    integrations: &[IntegrationSelection],
) -> Result<()> {
    if integrations.is_empty() {
        return Ok(());
    }

    let selected: BTreeSet<&str> = integrations.iter().map(|entry| entry.id.as_str()).collect();
    if selected.contains(INTEGRATION_OPENCLAW_TOOLS) {
        write_tools_adapter_script(runtime, paths)?;
    }

    Ok(())
}

/// Emit a runtime-local helper script that keeps the tools plugin entry/load path in sync.
/// The script enforces that managed OpenClaw always loads the bridge plugin and allows
/// it on the primary agent, even if OpenClaw rewrites `openclaw.json`.
fn write_tools_adapter_script(runtime: &RuntimeState, paths: &ProvisionedPaths) -> Result<()> {
    // Integration artifacts are runtime-local so setup never writes bridge config
    // back into the repository working tree.
    let integration_dir = runtime
        .config
        .runtime_root
        .join(RUNTIME_USER_DATA_DIR_NAME)
        .join(OPENCLAW_RUNTIME_INTEGRATIONS_DIR_NAME)
        .join(OPENCLAW_RUNTIME_INTEGRATIONS_SUBDIR);
    fs::create_dir_all(&integration_dir)
        .with_context(|| format!("create {}", integration_dir.display()))?;

    let script_path = integration_dir.join(OPENCLAW_TOOLS_SCRIPT_FILE_NAME);
    let plugin_id = OPENCLAW_TOOLS_PLUGIN_ID;
    let plugin_load_path = "$AR_DIR/bridge/openclaw/openclaw-agent-ruler-tools";
    let content = format!(
        r#"#!/usr/bin/env bash
set -euo pipefail

AR_DIR="${{AR_DIR:-{ruler_root}}}"
OPENCLAW_HOME="${{OPENCLAW_HOME:-{managed_home}}}"

openclaw config set plugins.load.paths "[\"{plugin_path}\"]" --json
openclaw config set plugins.entries.{plugin_id}.enabled true
openclaw config set plugins.entries.{plugin_id}.config.baseUrl "http://{ui_bind}"
openclaw config set plugins.entries.{plugin_id}.config.approvalWaitTimeoutSecs "{approval_wait_timeout_secs}"
openclaw config set agents.list[0].tools.allow "[\"{plugin_id}\"]" --json
"#,
        ruler_root = runtime.config.ruler_root.to_string_lossy(),
        managed_home = paths.managed_home.to_string_lossy(),
        plugin_id = plugin_id,
        plugin_path = plugin_load_path,
        ui_bind = runtime.config.ui_bind.trim(),
        approval_wait_timeout_secs = runtime.config.approval_wait_timeout_secs.clamp(1, 300),
    );
    fs::write(&script_path, content).with_context(|| format!("write {}", script_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path)
            .with_context(|| format!("read metadata {}", script_path.display()))?
            .permissions();
        perms.set_mode(0o750);
        fs::set_permissions(&script_path, perms)
            .with_context(|| format!("set permissions {}", script_path.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::{Context, Result};
    use serde_json::Value;
    use tempfile::tempdir;

    use crate::config::{init_layout, load_runtime};
    use crate::runners::{
        HostInstall, IntegrationSelection, RunnerAdapter, RunnerAssociation, RunnerKind,
        RunnerMissingState,
    };

    use super::{
        enforce_tools_adapter_config_guard, extract_port_hint, parse_openclaw_home_from_environ,
        parse_ss_output_for_port, preferred_openclaw_config_path, OpenClawAdapter,
        OPENCLAW_AUTH_PROFILES_FILE_NAME, OPENCLAW_AUTH_STORE_FILE_NAME, OPENCLAW_CONFIG_FILE_NAME,
        OPENCLAW_RUNTIME_INTEGRATIONS_DIR_NAME, OPENCLAW_RUNTIME_INTEGRATIONS_SUBDIR,
        OPENCLAW_STATE_DIR_NAME, OPENCLAW_TOOLS_PLUGIN_ID, OPENCLAW_TOOLS_SCRIPT_FILE_NAME,
        RUNTIME_USER_DATA_DIR_NAME,
    };

    #[test]
    fn provisioning_creates_managed_paths_without_host_writes_when_import_skipped() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");
        init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
        let runtime =
            load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");

        let host_home = tempdir().expect("host tempdir");
        let host_cfg = host_home.path().join(OPENCLAW_CONFIG_FILE_NAME);
        fs::write(&host_cfg, "{ channels: { telegram: {} } }").expect("write host config");
        let before = fs::read_to_string(&host_cfg).expect("read host config");

        let adapter = OpenClawAdapter::new();
        let paths = adapter
            .provision_project_paths(&runtime)
            .expect("provision paths");
        let report = adapter
            .optional_import_from_host(
                Some(&HostInstall {
                    home: host_home.path().to_path_buf(),
                    detected_by: "test".to_string(),
                }),
                &paths,
                false,
            )
            .expect("optional import");

        assert!(paths.managed_home.exists(), "managed home should exist");
        assert!(
            paths.managed_workspace.exists(),
            "managed workspace should exist"
        );
        assert!(!report.imported, "import should stay disabled");
        assert!(
            !paths.managed_home.join("imported-openclaw.json").exists(),
            "snapshot should not be written when import is skipped"
        );

        let after = fs::read_to_string(&host_cfg).expect("read host config after");
        assert_eq!(before, after, "host OpenClaw config must not be modified");
    }

    #[test]
    fn import_generates_gateway_ready_managed_config_with_preserved_types() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");
        init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
        let runtime =
            load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");

        let host_home = tempdir().expect("host tempdir");
        let host_cfg = host_home.path().join(OPENCLAW_CONFIG_FILE_NAME);
        fs::write(
            &host_cfg,
            r#"{
  channels: {
    telegram: { token: "abc", retries: 3, enabled: true }
  },
  bindings: {
    primary: ["telegram"]
  },
  models: {
    providers: {
      zai: {
        models: [{ id: "glm-4.7" }]
      }
    }
  },
  agents: {
    defaults: {
      model: { primary: "zai/glm-4.7" }
    }
  },
  gateway: {
    auth: { token: "secret-token" },
    remote: { url: "ws://127.0.0.1:18789", timeout: 3000 },
    ignored: { shouldNotCarry: true }
  },
  hooks: {
    internal: {
      entries: {
        "session-memory": { enabled: true }
      }
    }
  },
  plugins: { noise: true }
}"#,
        )
        .expect("write host config");
        let host_before = fs::read_to_string(&host_cfg).expect("read host config");

        let adapter = OpenClawAdapter::new();
        let paths = adapter
            .provision_project_paths(&runtime)
            .expect("provision paths");
        seed_managed_config(&paths).expect("seed managed config");

        let report = adapter
            .optional_import_from_host(
                Some(&HostInstall {
                    home: host_home.path().to_path_buf(),
                    detected_by: "test".to_string(),
                }),
                &paths,
                true,
            )
            .expect("import from host");
        assert!(report.imported, "expected import report");
        assert!(
            report.snapshot_path.is_some(),
            "expected import snapshot for debugging"
        );

        let mut config = runtime.config.clone();
        adapter
            .write_runner_config(&runtime, &mut config, &paths, &report, &[])
            .expect("write runner config");
        adapter.validate(&config).expect("validate runner config");

        let managed_cfg = fs::read_to_string(preferred_openclaw_config_path(&paths.managed_home))
            .expect("read managed config");
        let managed_json: Value = json5::from_str(&managed_cfg).expect("parse managed config");

        assert_eq!(
            managed_json.pointer("/channels/telegram/retries"),
            Some(&Value::from(3)),
            "numeric type should be preserved"
        );
        assert_eq!(
            managed_json.pointer("/channels/telegram/enabled"),
            Some(&Value::from(true)),
            "boolean type should be preserved"
        );
        assert_eq!(
            managed_json.pointer("/bindings/primary/0"),
            Some(&Value::from("telegram")),
            "array/string type should be preserved"
        );
        assert_eq!(
            managed_json.pointer("/gateway/ignored/shouldNotCarry"),
            Some(&Value::from(true)),
            "gateway nested config should be preserved from host import"
        );
        assert_eq!(
            managed_json.pointer("/gateway/mode"),
            Some(&Value::from("local")),
            "managed config should force gateway.mode=local"
        );
        assert!(
            managed_json
                .pointer("/agents/defaults/workspace")
                .and_then(Value::as_str)
                .is_some(),
            "managed workspace should always be set"
        );
        assert_eq!(
            managed_json.pointer("/hooks/internal/entries/session-memory/enabled"),
            Some(&Value::from(false)),
            "session-memory hook should be disabled for non-anthropic primary models"
        );

        let host_after = fs::read_to_string(&host_cfg).expect("read host config after");
        assert_eq!(
            host_before, host_after,
            "host config should remain unchanged after import"
        );
    }

    #[test]
    fn import_copies_auth_profiles_and_auth_store_without_touching_host() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");
        init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
        let runtime =
            load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");

        let host_home = tempdir().expect("host tempdir");
        let host_cfg = host_home.path().join(OPENCLAW_CONFIG_FILE_NAME);
        fs::write(
            &host_cfg,
            r#"{
  channels: {
    telegram: { token: "telegram-token" }
  },
  models: {
    providers: {
      zai: {
        models: [{ id: "glm-4.7" }]
      }
    }
  },
  agents: {
    defaults: {
      model: { primary: "zai/glm-4.7" }
    }
  }
}"#,
        )
        .expect("write host config");

        let host_agent_dir = host_home.path().join("agents/main/agent");
        fs::create_dir_all(&host_agent_dir).expect("create host auth dir");
        let host_auth_profiles = host_agent_dir.join(OPENCLAW_AUTH_PROFILES_FILE_NAME);
        let host_auth_store = host_agent_dir.join(OPENCLAW_AUTH_STORE_FILE_NAME);
        fs::write(&host_auth_profiles, r#"{ "default": "anthropic" }"#)
            .expect("write host auth profiles");
        fs::write(
            &host_auth_store,
            r#"{ "anthropic": { "apiKey": "k-123" } }"#,
        )
        .expect("write host auth store");

        let host_sessions_dir = host_home.path().join("agents/main/sessions");
        fs::create_dir_all(&host_sessions_dir).expect("create host sessions dir");
        fs::write(
            host_sessions_dir.join("sessions.json"),
            r#"{ "agent:main:main": { "modelProvider": "anthropic", "model": "claude-opus-4-6" } }"#,
        )
        .expect("write host sessions index");
        fs::write(host_sessions_dir.join("old-session.jsonl"), "{}\n")
            .expect("write host session transcript");

        let before_profiles = fs::read_to_string(&host_auth_profiles).expect("read host profiles");
        let before_store = fs::read_to_string(&host_auth_store).expect("read host store");

        let adapter = OpenClawAdapter::new();
        let paths = adapter
            .provision_project_paths(&runtime)
            .expect("provision paths");
        seed_managed_config(&paths).expect("seed managed config");
        let stale_sessions = paths
            .managed_home
            .join(OPENCLAW_STATE_DIR_NAME)
            .join("agents/main/sessions/sessions.json");
        if let Some(parent) = stale_sessions.parent() {
            fs::create_dir_all(parent).expect("create managed stale sessions dir");
        }
        fs::write(
            &stale_sessions,
            r#"{ "agent:main:main": { "modelProvider": "anthropic" } }"#,
        )
        .expect("seed stale managed sessions");

        let report = adapter
            .optional_import_from_host(
                Some(&HostInstall {
                    home: host_home.path().to_path_buf(),
                    detected_by: "test".to_string(),
                }),
                &paths,
                true,
            )
            .expect("import from host");
        assert!(report.imported, "expected import to copy auth files");
        assert!(
            report.expected_auth_profiles,
            "auth profiles should be marked as required after import"
        );
        assert!(
            report.expected_auth_store,
            "auth store should be marked as required after import"
        );
        assert!(
            report.expected_telegram_token,
            "telegram token should be detected from host config"
        );

        let mut config = runtime.config.clone();
        adapter
            .write_runner_config(&runtime, &mut config, &paths, &report, &[])
            .expect("write runner config");

        let managed_profiles = paths
            .managed_home
            .join(OPENCLAW_STATE_DIR_NAME)
            .join("agents/main/agent")
            .join(OPENCLAW_AUTH_PROFILES_FILE_NAME);
        let managed_store = paths
            .managed_home
            .join(OPENCLAW_STATE_DIR_NAME)
            .join("agents/main/agent")
            .join(OPENCLAW_AUTH_STORE_FILE_NAME);

        assert!(
            managed_profiles.exists(),
            "managed auth profiles should exist"
        );
        assert!(managed_store.exists(), "managed auth store should exist");
        assert_eq!(
            fs::read_to_string(&managed_profiles).expect("read managed profiles"),
            before_profiles
        );
        assert_eq!(
            fs::read_to_string(&managed_store).expect("read managed store"),
            before_store
        );
        assert!(
            !paths
                .managed_home
                .join(OPENCLAW_STATE_DIR_NAME)
                .join("agents/main/sessions/sessions.json")
                .exists(),
            "managed import should not carry host sessions index"
        );
        assert!(
            !paths
                .managed_home
                .join(OPENCLAW_STATE_DIR_NAME)
                .join("agents/main/sessions/old-session.jsonl")
                .exists(),
            "managed import should not carry host session transcripts"
        );

        assert_eq!(
            fs::read_to_string(&host_auth_profiles).expect("read host profiles after"),
            before_profiles,
            "host auth profiles must remain untouched"
        );
        assert_eq!(
            fs::read_to_string(&host_auth_store).expect("read host store after"),
            before_store,
            "host auth store must remain untouched"
        );
    }

    #[test]
    fn integration_output_stays_under_runtime_and_does_not_write_repo_tree() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");
        init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
        let runtime =
            load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");

        let bridge_dir = project.path().join("bridge").join("openclaw");
        fs::create_dir_all(&bridge_dir).expect("create bridge dir");
        fs::write(bridge_dir.join("channel-bridge.example.json"), "{}\n")
            .expect("write example bridge config");

        let adapter = OpenClawAdapter::new();
        let paths = adapter
            .provision_project_paths(&runtime)
            .expect("provision paths");
        seed_managed_config(&paths).expect("seed managed config");

        let mut config = runtime.config.clone();
        adapter
            .write_runner_config(
                &runtime,
                &mut config,
                &paths,
                &Default::default(),
                &[IntegrationSelection::new("openclaw_tools_adapter")],
            )
            .expect("write config with integrations");

        let runtime_script = runtime
            .config
            .runtime_root
            .join(RUNTIME_USER_DATA_DIR_NAME)
            .join(OPENCLAW_RUNTIME_INTEGRATIONS_DIR_NAME)
            .join(OPENCLAW_RUNTIME_INTEGRATIONS_SUBDIR)
            .join(OPENCLAW_TOOLS_SCRIPT_FILE_NAME);
        assert!(
            runtime_script.exists(),
            "integration output should be created under runtime user_data"
        );

        let managed_config_path = preferred_openclaw_config_path(&paths.managed_home);
        let managed_config_raw =
            fs::read_to_string(&managed_config_path).expect("read managed OpenClaw config");
        let managed_config: Value =
            json5::from_str(&managed_config_raw).expect("parse managed OpenClaw config");

        let plugin_entry = format!("/plugins/entries/{}", OPENCLAW_TOOLS_PLUGIN_ID);
        assert_eq!(
            managed_config
                .pointer(&format!("{}/enabled", plugin_entry))
                .and_then(Value::as_bool),
            Some(true),
            "integration should enable plugin entry in managed OpenClaw config"
        );
        assert_eq!(
            managed_config
                .pointer(&format!("{}/config/baseUrl", plugin_entry))
                .and_then(Value::as_str),
            Some("http://127.0.0.1:4622"),
            "integration should set managed Agent Ruler API base URL"
        );
        assert_eq!(
            managed_config
                .pointer(&format!("{}/config/approvalWaitTimeoutSecs", plugin_entry))
                .and_then(Value::as_u64),
            Some(runtime.config.approval_wait_timeout_secs),
            "integration should set approval wait timeout from runtime config"
        );
        let expected_plugin_path = super::tools_adapter_dir(&runtime.config.ruler_root)
            .to_string_lossy()
            .to_string();
        let load_paths = managed_config
            .pointer("/plugins/load/paths")
            .and_then(Value::as_array)
            .expect("plugins.load.paths should exist as an array");
        assert!(
            load_paths
                .iter()
                .any(|entry| entry.as_str() == Some(expected_plugin_path.as_str())),
            "integration should add plugin load path into managed OpenClaw config"
        );
        let tool_allow = managed_config
            .pointer("/agents/list/0/tools/allow")
            .and_then(Value::as_array)
            .expect("agents.list[0].tools.allow should exist as an array");
        assert!(
            tool_allow
                .iter()
                .any(|entry| entry.as_str() == Some(OPENCLAW_TOOLS_PLUGIN_ID)),
            "integration should allow plugin optional tools on the primary agent"
        );

        assert!(
            !project
                .path()
                .join("bridge/openclaw/channel-bridge.local.json")
                .exists(),
            "setup/integrations must not write repo-local bridge stubs"
        );
    }

    #[test]
    fn tools_adapter_guard_restores_plugin_wiring_when_missing() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");
        init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
        let mut runtime =
            load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");

        let adapter = OpenClawAdapter::new();
        let paths = adapter
            .provision_project_paths(&runtime)
            .expect("provision paths");
        seed_managed_config(&paths).expect("seed managed config");

        runtime.config.runner = Some(RunnerAssociation {
            kind: RunnerKind::Openclaw,
            managed_home: paths.managed_home.clone(),
            managed_workspace: paths.managed_workspace.clone(),
            integrations: vec![],
            missing: RunnerMissingState::default(),
        });

        let first = enforce_tools_adapter_config_guard(&runtime).expect("apply guard");
        assert!(first, "guard should patch missing plugin wiring");

        let managed_cfg = fs::read_to_string(preferred_openclaw_config_path(&paths.managed_home))
            .expect("read managed config");
        let managed_json: Value = json5::from_str(&managed_cfg).expect("parse managed config");
        let plugin_entry = format!("/plugins/entries/{}", OPENCLAW_TOOLS_PLUGIN_ID);
        assert_eq!(
            managed_json
                .pointer(&format!("{}/enabled", plugin_entry))
                .and_then(Value::as_bool),
            Some(true),
            "guard should enable tools adapter entry"
        );
        assert_eq!(
            managed_json
                .pointer(&format!("{}/config/approvalWaitTimeoutSecs", plugin_entry))
                .and_then(Value::as_u64),
            Some(runtime.config.approval_wait_timeout_secs),
            "guard should set approval wait timeout in plugin config"
        );
        let load_paths = managed_json
            .pointer("/plugins/load/paths")
            .and_then(Value::as_array)
            .expect("plugins.load.paths should exist");
        let expected_plugin_path = super::tools_adapter_dir(&runtime.config.ruler_root)
            .to_string_lossy()
            .to_string();
        assert!(
            load_paths
                .iter()
                .any(|entry| entry.as_str() == Some(expected_plugin_path.as_str())),
            "guard should add plugin load path"
        );

        let second = enforce_tools_adapter_config_guard(&runtime).expect("reapply guard");
        assert!(
            !second,
            "guard should be idempotent when wiring already exists"
        );
    }

    #[test]
    fn managed_config_defaults_to_state_subdir_path() {
        let home = tempdir().expect("home tempdir");
        let expected = home
            .path()
            .join(".openclaw")
            .join(OPENCLAW_CONFIG_FILE_NAME);
        let resolved = preferred_openclaw_config_path(home.path());
        assert_eq!(
            resolved, expected,
            "managed config should default to <managed_home>/.openclaw/openclaw.json"
        );
    }

    #[test]
    fn parses_ss_output_for_gateway_port_and_pid() {
        let sample = r#"State  Recv-Q Send-Q Local Address:Port  Peer Address:Port Process
LISTEN 0      128    127.0.0.1:4622      0.0.0.0:*    users:(("openclaw",pid=4242,fd=13))
LISTEN 0      128    127.0.0.1:8080      0.0.0.0:*    users:(("python3",pid=999,fd=5))
"#;

        let listeners = parse_ss_output_for_port(sample, 4622);
        assert_eq!(
            listeners.len(),
            1,
            "should find one listener on target port"
        );
        assert_eq!(listeners[0].pid, Some(4242));
        assert!(listeners[0].ss_line.contains("127.0.0.1:4622"));
    }

    #[test]
    fn parses_openclaw_home_from_proc_environ_bytes() {
        let env = b"PATH=/usr/bin\0OPENCLAW_HOME=/tmp/managed-openclaw\0HOME=/tmp\0";
        let parsed = parse_openclaw_home_from_environ(env);
        assert_eq!(parsed.as_deref(), Some("/tmp/managed-openclaw"));
    }

    #[test]
    fn extracts_gateway_port_hint_without_accepting_timezone_zero() {
        let text = "2026-02-21T23:26:42.519-05:00 another gateway instance is already listening on ws://127.0.0.1:18789";
        assert_eq!(extract_port_hint(text), Some(18789));
    }

    fn seed_managed_config(paths: &crate::runners::ProvisionedPaths) -> Result<()> {
        let cfg_path = preferred_openclaw_config_path(&paths.managed_home);
        if let Some(parent) = cfg_path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::write(
            &cfg_path,
            serde_json::to_string_pretty(&serde_json::json!({
                "agents": {
                    "defaults": {
                        "workspace": paths.managed_workspace.to_string_lossy().to_string()
                    }
                },
                "gateway": {
                    "mode": "local"
                }
            }))
            .context("serialize seed config")?,
        )
        .with_context(|| format!("write {}", cfg_path.display()))?;
        Ok(())
    }
}
