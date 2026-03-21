//! Configuration management for Agent Ruler.
//!
//! This module handles all configuration loading, validation, and runtime state
//! management. It defines the policy structure, zone configuration, and network rules.
//!
//! # Key Types
//!
//! - [`AppConfig`] - Application-level configuration (paths, directories)
//! - [`Policy`] - Security policy with rules for each zone and action type
//! - [`RuntimeState`] - Loaded configuration ready for use
//! - [`RuntimeLayout`] - Resolved paths for runtime directories
//!
//! # Configuration Files
//!
//! - `config.yaml` - Application configuration (paths, UI settings)
//! - `policy.yaml` - Security policy (rules, profiles, zone definitions)
//!
//! # Policy Profiles
//!
//! Policies support different profiles with varying strictness:
//! - `strict` - Maximum security, limited customization
//! - `simple_user` - Locked defaults for everyday usage
//! - `balanced` - Moderate security with practical flexibility
//! - `coding_nerd` - More freedom for coding-oriented workflows
//! - `i_dont_care` - Relaxed security, full customization
//! - `custom` - User-defined profile with full control
//!
//! # Network Rules
//!
//! Network rules control egress access:
//! - `default_deny` - Block all network access by default
//! - `allowlist_hosts` - Domains permitted for requests
//! - `denylist_hosts` - Domains blocked from requests
//! - `invert_allowlist/denylist` - Flip list behavior
//!
//! # Zone Paths
//!
//! Zones are defined by path prefixes. The classification logic is in
//! [`crate::policy::zone`].
//!
//! # ruler_root
//!
//! `ruler_root` is the canonical Agent Ruler install root used to locate bundled assets
//! (UI, bridge plugins, etc.). It is derived from:
//! 1. `AGENT_RULER_ROOT` environment variable (if set)
//! 2. The directory containing the running binary
//! 3. Compile-time install prefix (if set)
//!
//! It MUST NOT depend on the current working directory.
//!
//! # Tests
//!
//! See `/tests/integration_policy_flow.rs` for policy loading tests.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::runners::RunnerAssociation;
use crate::utils::expand_tilde;

/// Application directory name in ~/.config or equivalent
pub const APP_DIR_NAME: &str = "agent-ruler";
/// Subdirectory for per-project configurations
pub const PROJECTS_DIR_NAME: &str = "projects";
/// State directory name (receipts, approvals)
pub const STATE_DIR_NAME: &str = "state";
/// Workspace directory name (agent working directory)
pub const WORKSPACE_DIR_NAME: &str = "workspace";
/// Shared zone directory name (export staging)
pub const SHARED_ZONE_DIR_NAME: &str = "shared-zone";

/// Main configuration file name
pub const CONFIG_FILE_NAME: &str = "config.yaml";
/// Policy file name
pub const POLICY_FILE_NAME: &str = "policy.yaml";
/// Audit receipts file name
pub const RECEIPTS_FILE_NAME: &str = "receipts.jsonl";
/// Pending approvals file name
pub const APPROVALS_FILE_NAME: &str = "approvals.json";
/// Export requests directory name
pub const EXPORT_REQUESTS_DIR_NAME: &str = "export-requests";
/// Execution layer directory name (elevation helpers)
pub const EXEC_LAYER_DIR_NAME: &str = "exec-layer";
/// Quarantine directory for suspicious files
pub const QUARANTINE_DIR_NAME: &str = "quarantine";
/// Staged exports tracking file
pub const STAGED_EXPORTS_FILE_NAME: &str = "staged-exports.json";
/// Default long-poll timeout for approval wait endpoints/tools.
pub const DEFAULT_APPROVAL_WAIT_TIMEOUT_SECS: u64 = 90;

/// Safe default domains for GET-style browsing and package/doc retrieval.
pub const DEFAULT_SAFE_GET_DOMAIN_ALLOWLIST_PRESETS: &[&str] = &[
    "github.com",
    "api.github.com",
    "raw.githubusercontent.com",
    "gist.githubusercontent.com",
    "docs.github.com",
    "pypi.org",
    "files.pythonhosted.org",
    "registry.npmjs.org",
    "docs.npmjs.com",
    "crates.io",
    "static.crates.io",
    "docs.rs",
    "packages.ubuntu.com",
    "manpages.ubuntu.com",
    "archive.ubuntu.com",
    "security.ubuntu.com",
    "developer.mozilla.org",
    "nodejs.org",
    "docs.python.org",
    "doc.rust-lang.org",
    "pkg.go.dev",
    "go.dev",
    "gitlab.com",
    "docs.gitlab.com",
    "platform.openai.com",
    "api.openai.com",
];

/// Safe default domains for POST-style API and webhook calls.
pub const DEFAULT_SAFE_POST_DOMAIN_ALLOWLIST_PRESETS: &[&str] = &[
    "api.github.com",
    "api.openai.com",
    "api.telegram.org",
    "upload.pypi.org",
    "registry.npmjs.org",
    "discord.com",
    "api.facebook.com",
    "graph.facebook.com",
    "whatsapp.com",
    "api.whatsapp.com",
];

pub const DEFAULT_ALLOWLISTED_APT_PACKAGES: &[&str] = &[
    "git",
    "curl",
    "wget",
    "ca-certificates",
    "unzip",
    "zip",
    "tar",
    "gzip",
    "xz-utils",
    "jq",
    "make",
    "build-essential",
    "pkg-config",
    "python3",
    "python3-pip",
    "python3-venv",
    "nodejs",
    "npm",
    "ripgrep",
    "fd-find",
    "sqlite3",
];

pub const DEFAULT_ALLOWLISTED_PIP_PACKAGES: &[&str] = &[
    "requests", "httpx", "pydantic", "pyyaml", "rich", "typer", "click", "pytest", "numpy",
    "pandas",
];

pub const DEFAULT_ALLOWLISTED_NPM_PACKAGES: &[&str] = &[
    "typescript",
    "tsx",
    "eslint",
    "prettier",
    "vite",
    "vitest",
    "express",
    "axios",
    "zod",
    "dotenv",
];

pub const DEFAULT_ALLOWLISTED_CARGO_PACKAGES: &[&str] = &[
    "serde",
    "serde_json",
    "tokio",
    "reqwest",
    "clap",
    "anyhow",
    "thiserror",
    "tracing",
    "axum",
    "sqlx",
];

pub const DEFAULT_DENYLISTED_APT_PACKAGES: &[&str] = &[
    "openssh-server",
    "xrdp",
    "tcpdump",
    "wireshark",
    "tshark",
    "bettercap",
    "nmap",
    "netcat-openbsd",
    "socat",
    "hydra",
    "sqlmap",
    "john",
    "hashcat",
    "aircrack-ng",
];

pub const DEFAULT_DENYLISTED_PIP_PACKAGES: &[&str] = &["mitmproxy", "scapy", "impacket"];

pub const DEFAULT_DENYLISTED_NPM_PACKAGES: &[&str] = &["node-pty"];

pub const DEFAULT_DENYLISTED_CARGO_PACKAGES: &[&str] = &[];

/// Detect the canonical Agent Ruler install root (ruler_root).
///
/// ruler_root is used to locate bundled assets (UI, bridge plugins, etc.).
/// It is derived from (in order of precedence):
/// 1. `AGENT_RULER_ROOT` environment variable (if set and non-empty)
/// 2. The directory containing the running binary (if it looks like an install location)
/// 3. The current directory as a fallback for dev mode
///
/// This function MUST NOT depend on the current working directory for release installs.
pub fn detect_ruler_root() -> PathBuf {
    // 1. Check for explicit environment variable override
    if let Ok(root) = env::var("AGENT_RULER_ROOT") {
        let root_path = PathBuf::from(root.trim());
        if !root_path.as_os_str().is_empty() && root_path.exists() {
            return canonical_or_original(&root_path);
        }
    }

    // 2. Try to derive from the running binary location
    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            // Check if we're running from a typical install location
            let exe_dir_str = exe_dir.to_string_lossy();

            // Release install patterns:
            // - ~/.local/share/agent-ruler/installs/<version>/bin/agent-ruler
            // - /usr/local/bin/agent-ruler
            // - /usr/bin/agent-ruler
            if exe_dir_str.contains("/installs/") && exe_dir_str.contains("/agent-ruler/") {
                // Installed via releases: ~/.local/share/agent-ruler/installs/<version>/
                if let Some(install_root) = exe_dir.parent() {
                    // From bin/ back to install root
                    return canonical_or_original(install_root);
                }
            }

            // Check if there's a bridge/ directory next to the binary (dev mode or install)
            let candidate_root = if exe_dir.file_name().map(|n| n == "bin").unwrap_or(false) {
                // Binary is in a bin/ directory, check parent
                exe_dir.parent().unwrap_or(exe_dir).to_path_buf()
            } else {
                exe_dir.to_path_buf()
            };

            // If bridge/ exists next to binary or its parent, use that as ruler_root
            if candidate_root.join("bridge").exists() {
                return canonical_or_original(&candidate_root);
            }

            // For /usr/local/bin or /usr/bin installs, check standard lib locations
            if exe_dir_str == "/usr/local/bin" {
                let lib_path = PathBuf::from("/usr/local/lib/agent-ruler");
                if lib_path.exists() {
                    return lib_path;
                }
            }
            if exe_dir_str == "/usr/bin" {
                let lib_path = PathBuf::from("/usr/lib/agent-ruler");
                if lib_path.exists() {
                    return lib_path;
                }
            }

            // Standalone user-bin installs (for example manual release install to
            // ~/.local/bin) still use the managed installs root for bundled assets.
            if is_user_bin_install_dir(exe_dir) {
                return default_user_installs_root();
            }
        }
    }

    // 3. Fallback: use current directory (dev mode)
    // This is only reached when running from source without AGENT_RULER_ROOT set
    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn is_user_bin_install_dir(exe_dir: &Path) -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    let candidates = [
        home.join(".local/bin"),
        home.join(".cargo/bin"),
        home.join("bin"),
    ];
    candidates.iter().any(|candidate| candidate == exe_dir)
}

fn default_user_installs_root() -> PathBuf {
    if let Ok(xdg_data_home) = env::var("XDG_DATA_HOME") {
        let trimmed = xdg_data_home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("agent-ruler").join("installs");
        }
    }
    if let Some(home) = dirs::home_dir() {
        return home
            .join(".local/share")
            .join("agent-ruler")
            .join("installs");
    }
    PathBuf::from(".local/share/agent-ruler/installs")
}

fn installed_user_data_root_from_exe_path(exe_path: &Path) -> Option<PathBuf> {
    let canonical = canonical_or_original(exe_path);
    let exe_dir = canonical.parent()?;
    let install_instance_dir = if exe_dir
        .file_name()
        .map(|name| name == "bin")
        .unwrap_or(false)
    {
        exe_dir.parent()?
    } else {
        exe_dir
    };
    let installs_dir = install_instance_dir.parent()?;
    if installs_dir.file_name()? != "installs" {
        return None;
    }
    let user_data_root = installs_dir.parent()?;
    if user_data_root.file_name()? != APP_DIR_NAME {
        return None;
    }
    Some(user_data_root.to_path_buf())
}

fn installed_user_data_root_from_current_exe() -> Option<PathBuf> {
    let exe_path = env::current_exe().ok()?;
    installed_user_data_root_from_exe_path(&exe_path)
}

#[derive(Debug, Clone)]
pub struct RuntimeLayout {
    pub ruler_root: PathBuf,
    pub runtime_root: PathBuf,
    pub state_dir: PathBuf,
    pub workspace_dir: PathBuf,
    pub shared_zone_dir: PathBuf,
    pub config_file: PathBuf,
    pub policy_file: PathBuf,
    pub receipts_file: PathBuf,
    pub approvals_file: PathBuf,
    pub export_requests_dir: PathBuf,
    pub exec_layer_dir: PathBuf,
    pub quarantine_dir: PathBuf,
    pub staged_exports_file: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// The canonical Agent Ruler install root for locating bundled assets.
    /// Previously named `project_root` - both field names are accepted for backward compatibility.
    #[serde(alias = "project_root")]
    pub ruler_root: PathBuf,
    #[serde(default)]
    pub runtime_root: PathBuf,
    pub workspace: PathBuf,
    #[serde(default)]
    pub shared_zone_dir: PathBuf,
    pub state_dir: PathBuf,
    pub policy_file: PathBuf,
    pub receipts_file: PathBuf,
    pub approvals_file: PathBuf,
    pub export_requests_dir: PathBuf,
    pub exec_layer_dir: PathBuf,
    pub quarantine_dir: PathBuf,
    #[serde(default)]
    pub staged_exports_file: PathBuf,
    #[serde(default)]
    pub default_delivery_dir: PathBuf,
    #[serde(default)]
    pub ui_show_debug_tools: bool,
    #[serde(default)]
    pub allow_degraded_confinement: bool,
    #[serde(default = "default_approval_wait_timeout_secs")]
    pub approval_wait_timeout_secs: u64,
    pub ui_bind: String,
    #[serde(default)]
    pub runner: Option<RunnerAssociation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub version: String,
    pub profile: String,
    pub zones: ZonesConfig,
    pub rules: RulesConfig,
    pub safeguards: Safeguards,
    pub approvals: ApprovalConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZonesConfig {
    pub workspace_paths: Vec<String>,
    pub user_data_paths: Vec<String>,
    pub shared_paths: Vec<String>,
    pub system_critical_paths: Vec<String>,
    pub secrets_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RulesConfig {
    pub filesystem: ZoneRuleMatrix,
    pub network: NetworkRules,
    pub execution: ExecutionRules,
    pub persistence: PersistenceRules,
    #[serde(default)]
    pub elevation: ElevationRules,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneRuleMatrix {
    pub workspace: RuleDisposition,
    pub user_data: RuleDisposition,
    pub shared: RuleDisposition,
    pub system_critical: RuleDisposition,
    pub secrets: RuleDisposition,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleDisposition {
    Allow,
    Deny,
    Approval,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkRules {
    pub default_deny: bool,
    pub allowlist_hosts: Vec<String>,
    #[serde(default = "default_true")]
    pub require_approval_for_post: bool,
    /// GET request denylist - domains blocked from GET requests
    #[serde(default)]
    pub denylist_hosts: Vec<String>,
    /// If true, allowlist_hosts becomes a denylist (inverts behavior)
    #[serde(default)]
    pub invert_allowlist: bool,
    /// If true, denylist_hosts becomes an allowlist (inverts behavior)
    #[serde(default)]
    pub invert_denylist: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRules {
    pub deny_workspace_exec: bool,
    pub deny_tmp_exec: bool,
    pub quarantine_on_download_exec_chain: bool,
    pub allowed_exec_prefixes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistenceRules {
    pub deny_autostart: bool,
    pub approval_paths: Vec<String>,
    pub deny_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElevationRules {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub require_operator_auth: bool,
    #[serde(default = "default_true")]
    pub use_allowlist: bool,
    #[serde(default)]
    pub allowed_packages: Vec<String>,
    #[serde(default)]
    pub denied_packages: Vec<String>,
}

impl Default for ElevationRules {
    fn default() -> Self {
        Self {
            enabled: false,
            require_operator_auth: default_true(),
            use_allowlist: default_true(),
            allowed_packages: Vec::new(),
            denied_packages: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Safeguards {
    pub mass_delete_threshold: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalConfig {
    pub ttl_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct RuntimeState {
    pub config: AppConfig,
    pub policy: Policy,
    pub policy_hash: String,
}

impl Policy {
    pub fn policy_hash(&self) -> Result<String> {
        let mut cloned = self.clone();

        // Stable ordering improves reproducibility for receipts and approvals.
        sort_dedup_vec(&mut cloned.zones.workspace_paths);
        sort_dedup_vec(&mut cloned.zones.user_data_paths);
        sort_dedup_vec(&mut cloned.zones.shared_paths);
        sort_dedup_vec(&mut cloned.zones.system_critical_paths);
        sort_dedup_vec(&mut cloned.zones.secrets_paths);
        sort_dedup_vec(&mut cloned.rules.network.allowlist_hosts);
        sort_dedup_vec(&mut cloned.rules.network.denylist_hosts);
        sort_dedup_vec(&mut cloned.rules.execution.allowed_exec_prefixes);
        sort_dedup_vec(&mut cloned.rules.persistence.approval_paths);
        sort_dedup_vec(&mut cloned.rules.persistence.deny_paths);
        sort_dedup_vec(&mut cloned.rules.elevation.allowed_packages);
        sort_dedup_vec(&mut cloned.rules.elevation.denied_packages);

        let payload = serde_json::to_vec(&cloned).context("serialize policy")?;
        let mut hasher = Sha256::new();
        hasher.update(payload);
        Ok(hex::encode(hasher.finalize()))
    }

    pub fn expanded(&self, workspace: &Path) -> Self {
        let mut cloned = self.clone();
        cloned.zones.workspace_paths = expand_patterns(&cloned.zones.workspace_paths, workspace);
        cloned.zones.user_data_paths = expand_patterns(&cloned.zones.user_data_paths, workspace);
        cloned.zones.shared_paths = expand_patterns(&cloned.zones.shared_paths, workspace);
        cloned.zones.system_critical_paths =
            expand_patterns(&cloned.zones.system_critical_paths, workspace);
        cloned.zones.secrets_paths = expand_patterns(&cloned.zones.secrets_paths, workspace);
        cloned.rules.execution.allowed_exec_prefixes =
            expand_patterns(&cloned.rules.execution.allowed_exec_prefixes, workspace);
        cloned.rules.persistence.approval_paths =
            expand_patterns(&cloned.rules.persistence.approval_paths, workspace);
        cloned.rules.persistence.deny_paths =
            expand_patterns(&cloned.rules.persistence.deny_paths, workspace);
        cloned
    }
}

fn default_true() -> bool {
    true
}

fn default_approval_wait_timeout_secs() -> u64 {
    DEFAULT_APPROVAL_WAIT_TIMEOUT_SECS
}

// Resolve all runtime paths from a single root so CLI/UI/tests share identical layout semantics.
pub fn resolve_runtime_layout(
    ruler_root: &Path,
    runtime_dir: Option<&Path>,
) -> Result<RuntimeLayout> {
    let ruler_root = canonical_or_original(ruler_root);

    let runtime_root = match runtime_dir {
        Some(path) => {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                ruler_root.join(path)
            }
        }
        None => {
            let key = project_runtime_key(&ruler_root)?;
            default_runtime_base_dir().join(PROJECTS_DIR_NAME).join(key)
        }
    };

    let runtime_root = canonical_or_original(&runtime_root);
    let state_dir = runtime_root.join(STATE_DIR_NAME);

    Ok(RuntimeLayout {
        ruler_root,
        runtime_root: runtime_root.clone(),
        state_dir: state_dir.clone(),
        workspace_dir: runtime_root.join(WORKSPACE_DIR_NAME),
        shared_zone_dir: runtime_root.join(SHARED_ZONE_DIR_NAME),
        config_file: state_dir.join(CONFIG_FILE_NAME),
        policy_file: state_dir.join(POLICY_FILE_NAME),
        receipts_file: state_dir.join(RECEIPTS_FILE_NAME),
        approvals_file: state_dir.join(APPROVALS_FILE_NAME),
        export_requests_dir: state_dir.join(EXPORT_REQUESTS_DIR_NAME),
        exec_layer_dir: state_dir.join(EXEC_LAYER_DIR_NAME),
        quarantine_dir: state_dir.join(QUARANTINE_DIR_NAME),
        staged_exports_file: state_dir.join(STAGED_EXPORTS_FILE_NAME),
    })
}

// Create or reset runtime state at the resolved runtime root and persist the initial config/policy files.
pub fn init_layout(
    ruler_root: &Path,
    runtime_dir: Option<&Path>,
    workspace_override: Option<PathBuf>,
    force: bool,
) -> Result<AppConfig> {
    let layout = resolve_runtime_layout(ruler_root, runtime_dir)?;

    if layout.runtime_root.exists() {
        if force {
            fs::remove_dir_all(&layout.runtime_root).with_context(|| {
                format!(
                    "remove existing runtime directory {}",
                    layout.runtime_root.display()
                )
            })?;
        } else {
            return Err(anyhow!(
                "runtime directory already exists at {} (use --force to overwrite files)",
                layout.runtime_root.display()
            ));
        }
    }

    let workspace = resolve_workspace_path(&layout, workspace_override)?;

    fs::create_dir_all(&layout.runtime_root)
        .with_context(|| format!("create runtime root {}", layout.runtime_root.display()))?;
    fs::create_dir_all(&layout.state_dir)
        .with_context(|| format!("create state directory {}", layout.state_dir.display()))?;
    fs::create_dir_all(&workspace)
        .with_context(|| format!("create workspace directory {}", workspace.display()))?;
    fs::create_dir_all(&layout.shared_zone_dir).with_context(|| {
        format!(
            "create shared-zone directory {}",
            layout.shared_zone_dir.display()
        )
    })?;
    fs::create_dir_all(&layout.export_requests_dir)
        .with_context(|| format!("create {}", layout.export_requests_dir.display()))?;
    fs::create_dir_all(&layout.exec_layer_dir)
        .with_context(|| format!("create {}", layout.exec_layer_dir.display()))?;
    fs::create_dir_all(&layout.quarantine_dir)
        .with_context(|| format!("create {}", layout.quarantine_dir.display()))?;

    fs::write(
        &layout.policy_file,
        include_str!("../assets/default-policy.yaml"),
    )
    .context("write default policy")?;

    if !layout.receipts_file.exists() {
        fs::File::create(&layout.receipts_file).context("create receipts file")?;
    }

    if !layout.approvals_file.exists() {
        fs::write(&layout.approvals_file, "[]\n").context("create approvals file")?;
    }

    if !layout.staged_exports_file.exists() {
        fs::write(&layout.staged_exports_file, "[]\n").context("create staged exports file")?;
    }

    let default_delivery_dir = default_delivery_dir_for_project(&layout.ruler_root);
    fs::create_dir_all(&default_delivery_dir).with_context(|| {
        format!(
            "create default delivery directory {}",
            default_delivery_dir.display()
        )
    })?;

    let config = AppConfig {
        ruler_root: layout.ruler_root,
        runtime_root: layout.runtime_root,
        workspace,
        shared_zone_dir: layout.shared_zone_dir,
        state_dir: layout.state_dir,
        policy_file: layout.policy_file,
        receipts_file: layout.receipts_file,
        approvals_file: layout.approvals_file,
        export_requests_dir: layout.export_requests_dir,
        exec_layer_dir: layout.exec_layer_dir,
        quarantine_dir: layout.quarantine_dir,
        staged_exports_file: layout.staged_exports_file,
        default_delivery_dir,
        ui_show_debug_tools: false,
        allow_degraded_confinement: false,
        approval_wait_timeout_secs: default_approval_wait_timeout_secs(),
        ui_bind: "127.0.0.1:4622".to_string(),
        runner: None,
    };

    let yaml = serde_yaml::to_string(&config).context("serialize config")?;
    fs::write(&layout.config_file, yaml)
        .with_context(|| format!("write config file {}", layout.config_file.display()))?;

    Ok(config)
}

pub fn reset_layout(
    ruler_root: &Path,
    runtime_dir: Option<&Path>,
    keep_config: bool,
) -> Result<AppConfig> {
    if !keep_config {
        return init_layout(ruler_root, runtime_dir, None, true);
    }

    let existing_runtime = load_runtime(ruler_root, runtime_dir)
        .context("load runtime before keep-config reset (run `agent-ruler init` first)")?;
    let preserved_config = existing_runtime.config.clone();
    let preserved_policy = existing_runtime.policy.clone();

    let workspace_override = Some(preserved_config.workspace.clone());
    let initialized = init_layout(ruler_root, runtime_dir, workspace_override, true)?;
    let config_path = initialized.state_dir.join(CONFIG_FILE_NAME);
    let policy_path = initialized.state_dir.join(POLICY_FILE_NAME);

    save_config(&config_path, &preserved_config).with_context(|| {
        format!(
            "restore preserved config during keep-config reset at {}",
            config_path.display()
        )
    })?;
    save_policy(&policy_path, &preserved_policy).with_context(|| {
        format!(
            "restore preserved policy during keep-config reset at {}",
            policy_path.display()
        )
    })?;
    ensure_runtime_artifacts(&preserved_config)?;

    Ok(preserved_config)
}

pub fn safe_domain_allowlist_presets() -> Vec<String> {
    DEFAULT_SAFE_POST_DOMAIN_ALLOWLIST_PRESETS
        .iter()
        .map(|entry| entry.to_string())
        .collect()
}

pub fn safe_domain_denylist_presets() -> Vec<String> {
    DEFAULT_SAFE_GET_DOMAIN_ALLOWLIST_PRESETS
        .iter()
        .map(|entry| entry.to_string())
        .collect()
}

pub fn safe_get_domain_allowlist_presets() -> Vec<String> {
    DEFAULT_SAFE_GET_DOMAIN_ALLOWLIST_PRESETS
        .iter()
        .map(|entry| entry.to_string())
        .collect()
}

pub fn safe_post_domain_allowlist_presets() -> Vec<String> {
    DEFAULT_SAFE_POST_DOMAIN_ALLOWLIST_PRESETS
        .iter()
        .map(|entry| entry.to_string())
        .collect()
}

pub fn allowlisted_package_presets() -> BTreeMap<String, Vec<String>> {
    let mut map = BTreeMap::new();
    map.insert(
        "apt".to_string(),
        DEFAULT_ALLOWLISTED_APT_PACKAGES
            .iter()
            .map(|entry| entry.to_string())
            .collect(),
    );
    map.insert(
        "pip".to_string(),
        DEFAULT_ALLOWLISTED_PIP_PACKAGES
            .iter()
            .map(|entry| entry.to_string())
            .collect(),
    );
    map.insert(
        "npm".to_string(),
        DEFAULT_ALLOWLISTED_NPM_PACKAGES
            .iter()
            .map(|entry| entry.to_string())
            .collect(),
    );
    map.insert(
        "cargo".to_string(),
        DEFAULT_ALLOWLISTED_CARGO_PACKAGES
            .iter()
            .map(|entry| entry.to_string())
            .collect(),
    );
    map
}

pub fn denylisted_package_presets() -> BTreeMap<String, Vec<String>> {
    let mut map = BTreeMap::new();
    map.insert(
        "apt".to_string(),
        DEFAULT_DENYLISTED_APT_PACKAGES
            .iter()
            .map(|entry| entry.to_string())
            .collect(),
    );
    map.insert(
        "pip".to_string(),
        DEFAULT_DENYLISTED_PIP_PACKAGES
            .iter()
            .map(|entry| entry.to_string())
            .collect(),
    );
    map.insert(
        "npm".to_string(),
        DEFAULT_DENYLISTED_NPM_PACKAGES
            .iter()
            .map(|entry| entry.to_string())
            .collect(),
    );
    map.insert(
        "cargo".to_string(),
        DEFAULT_DENYLISTED_CARGO_PACKAGES
            .iter()
            .map(|entry| entry.to_string())
            .collect(),
    );
    map
}

pub fn load_runtime(ruler_root: &Path, runtime_dir: Option<&Path>) -> Result<RuntimeState> {
    let layout = resolve_runtime_layout(ruler_root, runtime_dir)?;
    let config_raw = fs::read_to_string(&layout.config_file).with_context(|| {
        format!(
            "read config file {} (run `agent-ruler init` first)",
            layout.config_file.display()
        )
    })?;

    let mut config: AppConfig = serde_yaml::from_str(&config_raw).context("parse config yaml")?;

    if config.runtime_root.as_os_str().is_empty() {
        config.runtime_root = layout.runtime_root.clone();
    }
    if config.shared_zone_dir.as_os_str().is_empty() {
        config.shared_zone_dir = layout.shared_zone_dir.clone();
    }
    if config.staged_exports_file.as_os_str().is_empty() {
        config.staged_exports_file = layout.staged_exports_file.clone();
    }
    if config.default_delivery_dir.as_os_str().is_empty() {
        config.default_delivery_dir = default_delivery_dir_for_project(&layout.ruler_root);
    }
    if config.approval_wait_timeout_secs == 0 {
        config.approval_wait_timeout_secs = default_approval_wait_timeout_secs();
    }
    config.approval_wait_timeout_secs = config.approval_wait_timeout_secs.clamp(1, 300);

    let loaded_ruler_root = canonical_or_original(&config.ruler_root);
    let expected_ruler_root = canonical_or_original(&layout.ruler_root);
    config.ruler_root = loaded_ruler_root.clone();
    config.runtime_root = canonical_or_original(&config.runtime_root);
    config.workspace = canonical_or_original(&config.workspace);
    config.shared_zone_dir = canonical_or_original(&config.shared_zone_dir);
    config.state_dir = canonical_or_original(&config.state_dir);
    config.policy_file = canonical_or_original(&config.policy_file);
    config.receipts_file = canonical_or_original(&config.receipts_file);
    config.approvals_file = canonical_or_original(&config.approvals_file);
    config.export_requests_dir = canonical_or_original(&config.export_requests_dir);
    config.exec_layer_dir = canonical_or_original(&config.exec_layer_dir);
    config.quarantine_dir = canonical_or_original(&config.quarantine_dir);
    config.staged_exports_file = canonical_or_original(&config.staged_exports_file);
    config.default_delivery_dir = canonical_or_original(&config.default_delivery_dir);
    if let Some(runner) = config.runner.as_mut() {
        runner.managed_home = canonical_or_original(&runner.managed_home);
        runner.managed_workspace = canonical_or_original(&runner.managed_workspace);
    }

    if loaded_ruler_root != expected_ruler_root {
        let previous_default_delivery = default_delivery_dir_for_project(&loaded_ruler_root);
        let legacy_previous_default_delivery =
            legacy_default_delivery_dir_for_project(&loaded_ruler_root);
        config.ruler_root = expected_ruler_root.clone();
        if config.default_delivery_dir == previous_default_delivery
            || config.default_delivery_dir == legacy_previous_default_delivery
        {
            config.default_delivery_dir = default_delivery_dir_for_project(&expected_ruler_root);
        }
        save_config(&layout.config_file, &config).with_context(|| {
            format!(
                "update runtime config root mapping at {}",
                layout.config_file.display()
            )
        })?;
    }

    let mut policy = load_policy(&config.policy_file)?.expanded(&config.workspace);
    if matches!(
        policy.profile.as_str(),
        "strict" | "simple_user" | "balanced"
    ) {
        policy.safeguards.mass_delete_threshold = 40;
    }
    let policy_hash = policy.policy_hash()?;

    Ok(RuntimeState {
        config,
        policy,
        policy_hash,
    })
}

pub fn load_policy(path: &Path) -> Result<Policy> {
    let content =
        fs::read_to_string(path).with_context(|| format!("read policy file {}", path.display()))?;
    let policy: Policy = serde_yaml::from_str(&content).context("parse policy yaml")?;
    Ok(policy)
}

pub fn save_policy(path: &Path, policy: &Policy) -> Result<()> {
    let yaml = serde_yaml::to_string(policy).context("serialize policy")?;
    fs::write(path, yaml).with_context(|| format!("write policy file {}", path.display()))?;
    Ok(())
}

// Persist runtime config updates (for example UI-edited shared-zone and delivery paths).
pub fn save_config(path: &Path, config: &AppConfig) -> Result<()> {
    let yaml = serde_yaml::to_string(config).context("serialize config yaml")?;
    fs::write(path, yaml).with_context(|| format!("write config file {}", path.display()))?;
    Ok(())
}

fn resolve_workspace_path(
    layout: &RuntimeLayout,
    workspace_override: Option<PathBuf>,
) -> Result<PathBuf> {
    match workspace_override {
        Some(path) if path.is_absolute() => Ok(path),
        Some(path) => {
            let workspace = layout.ruler_root.join(path);
            Ok(workspace)
        }
        None => Ok(layout.workspace_dir.clone()),
    }
}

fn ensure_runtime_artifacts(config: &AppConfig) -> Result<()> {
    fs::create_dir_all(&config.runtime_root)
        .with_context(|| format!("create runtime root {}", config.runtime_root.display()))?;
    fs::create_dir_all(&config.state_dir)
        .with_context(|| format!("create state dir {}", config.state_dir.display()))?;
    fs::create_dir_all(&config.workspace)
        .with_context(|| format!("create workspace {}", config.workspace.display()))?;
    fs::create_dir_all(&config.shared_zone_dir)
        .with_context(|| format!("create shared zone {}", config.shared_zone_dir.display()))?;
    fs::create_dir_all(&config.export_requests_dir)
        .with_context(|| format!("create {}", config.export_requests_dir.display()))?;
    fs::create_dir_all(&config.exec_layer_dir)
        .with_context(|| format!("create {}", config.exec_layer_dir.display()))?;
    fs::create_dir_all(&config.quarantine_dir)
        .with_context(|| format!("create {}", config.quarantine_dir.display()))?;
    fs::create_dir_all(&config.default_delivery_dir).with_context(|| {
        format!(
            "create default delivery directory {}",
            config.default_delivery_dir.display()
        )
    })?;

    if !config.receipts_file.exists() {
        fs::File::create(&config.receipts_file)
            .with_context(|| format!("create receipts file {}", config.receipts_file.display()))?;
    }
    if !config.approvals_file.exists() {
        fs::write(&config.approvals_file, "[]\n").with_context(|| {
            format!("create approvals file {}", config.approvals_file.display())
        })?;
    }
    if !config.staged_exports_file.exists() {
        fs::write(&config.staged_exports_file, "[]\n").with_context(|| {
            format!(
                "create staged exports file {}",
                config.staged_exports_file.display()
            )
        })?;
    }

    Ok(())
}

// Use XDG data dir for persistent mutable state; this keeps source trees clean by default.
fn default_runtime_base_dir() -> PathBuf {
    // Release/dev installs keep mutable runtime state alongside the managed
    // installs root so shell-specific XDG wrappers (for example VS Code Snap)
    // cannot silently split one machine into multiple Agent Ruler runtimes.
    if let Some(root) = installed_user_data_root_from_current_exe() {
        return root;
    }

    if let Ok(xdg_data_home) = env::var("XDG_DATA_HOME") {
        if !xdg_data_home.trim().is_empty() {
            return PathBuf::from(xdg_data_home).join(APP_DIR_NAME);
        }
    }

    if let Some(path) = dirs::data_local_dir() {
        return path.join(APP_DIR_NAME);
    }

    if let Some(home) = dirs::home_dir() {
        return home.join(".local").join("share").join(APP_DIR_NAME);
    }

    PathBuf::from("/tmp").join(APP_DIR_NAME)
}

pub fn runtime_projects_dir() -> PathBuf {
    default_runtime_base_dir().join(PROJECTS_DIR_NAME)
}

// Derive a stable per-project runtime key from canonical path to avoid collisions across checkouts.
fn project_runtime_key(ruler_root: &Path) -> Result<String> {
    let canonical = canonical_or_original(ruler_root);
    let canonical_str = canonical.to_string_lossy().to_string();

    let mut hasher = Sha256::new();
    hasher.update(canonical_str.as_bytes());
    let digest = hex::encode(hasher.finalize());
    let digest_short = &digest[..12];

    let base_name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());
    let sanitized = sanitize_segment(&base_name);

    if sanitized.is_empty() {
        return Err(anyhow!("failed to derive project runtime key"));
    }

    Ok(format!("{}-{}", sanitized, digest_short))
}

fn sanitize_segment(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }

    out.trim_matches('-').to_string()
}

fn default_delivery_dir_for_project(ruler_root: &Path) -> PathBuf {
    default_delivery_dir_with_roots(ruler_root, dirs::document_dir(), dirs::home_dir())
}

fn default_delivery_dir_with_roots(
    ruler_root: &Path,
    documents: Option<PathBuf>,
    home_dir: Option<PathBuf>,
) -> PathBuf {
    if let Some(documents) = documents {
        return documents.join("agent-ruler-deliveries");
    }

    if let Some(home_dir) = home_dir {
        return home_dir.join("Documents").join("agent-ruler-deliveries");
    }

    ruler_root.join("exports")
}

fn legacy_default_delivery_dir_for_project(ruler_root: &Path) -> PathBuf {
    let project_name = ruler_root
        .file_name()
        .map(|n| sanitize_segment(&n.to_string_lossy()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "project".to_string());

    default_delivery_dir_with_roots(
        ruler_root,
        dirs::document_dir().map(|documents| documents.join(project_name)),
        None,
    )
}

fn canonical_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn sort_dedup_vec(values: &mut Vec<String>) {
    let mut uniq = BTreeSet::new();
    for value in values.drain(..) {
        uniq.insert(value);
    }
    values.extend(uniq);
}

fn expand_patterns(patterns: &[String], workspace: &Path) -> Vec<String> {
    patterns
        .iter()
        .map(|p| p.replace("${WORKSPACE}", &workspace.to_string_lossy()))
        .map(|p| expand_tilde(&p))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use tempfile::tempdir;

    use super::{
        default_delivery_dir_with_roots, init_layout, installed_user_data_root_from_exe_path,
        load_policy, load_runtime, resolve_runtime_layout, DEFAULT_ALLOWLISTED_APT_PACKAGES,
        DEFAULT_ALLOWLISTED_CARGO_PACKAGES, DEFAULT_ALLOWLISTED_NPM_PACKAGES,
        DEFAULT_ALLOWLISTED_PIP_PACKAGES, DEFAULT_DENYLISTED_APT_PACKAGES,
        DEFAULT_DENYLISTED_CARGO_PACKAGES, DEFAULT_DENYLISTED_NPM_PACKAGES,
        DEFAULT_DENYLISTED_PIP_PACKAGES, DEFAULT_SAFE_GET_DOMAIN_ALLOWLIST_PRESETS,
        DEFAULT_SAFE_POST_DOMAIN_ALLOWLIST_PRESETS,
    };

    #[test]
    fn default_runtime_layout_avoids_project_tree() {
        let project = tempdir().expect("project tempdir");
        let layout = resolve_runtime_layout(project.path(), None).expect("resolve runtime layout");
        assert!(!layout.runtime_root.starts_with(project.path()));
    }

    #[test]
    fn override_runtime_layout_uses_provided_path() {
        let project = tempdir().expect("project tempdir");
        let override_root = tempdir().expect("override tempdir");
        let layout = resolve_runtime_layout(project.path(), Some(override_root.path()))
            .expect("resolve runtime layout");
        assert_eq!(layout.runtime_root, override_root.path());
    }

    #[test]
    fn init_layout_seeds_default_lists_for_fresh_runtime() {
        let project = tempdir().expect("project tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");

        let config = init_layout(project.path(), Some(runtime_root.path()), None, true)
            .expect("init runtime layout");
        let policy = load_policy(&config.policy_file).expect("load generated policy");

        let expected_hosts: BTreeSet<String> = DEFAULT_SAFE_GET_DOMAIN_ALLOWLIST_PRESETS
            .iter()
            .chain(DEFAULT_SAFE_POST_DOMAIN_ALLOWLIST_PRESETS.iter())
            .map(|value| value.to_string())
            .collect();
        let actual_hosts: BTreeSet<String> = policy
            .rules
            .network
            .allowlist_hosts
            .iter()
            .cloned()
            .collect();
        assert_eq!(actual_hosts, expected_hosts);
        assert!(policy.rules.network.denylist_hosts.is_empty());
        assert!(!policy.rules.network.invert_allowlist);
        assert!(!policy.rules.network.invert_denylist);

        let expected_allowed_packages: BTreeSet<String> = DEFAULT_ALLOWLISTED_APT_PACKAGES
            .iter()
            .chain(DEFAULT_ALLOWLISTED_PIP_PACKAGES.iter())
            .chain(DEFAULT_ALLOWLISTED_NPM_PACKAGES.iter())
            .chain(DEFAULT_ALLOWLISTED_CARGO_PACKAGES.iter())
            .map(|value| value.to_string())
            .collect();
        let actual_allowed_packages: BTreeSet<String> = policy
            .rules
            .elevation
            .allowed_packages
            .iter()
            .cloned()
            .collect();
        assert_eq!(actual_allowed_packages, expected_allowed_packages);

        let expected_denied_packages: BTreeSet<String> = DEFAULT_DENYLISTED_APT_PACKAGES
            .iter()
            .chain(DEFAULT_DENYLISTED_PIP_PACKAGES.iter())
            .chain(DEFAULT_DENYLISTED_NPM_PACKAGES.iter())
            .chain(DEFAULT_DENYLISTED_CARGO_PACKAGES.iter())
            .map(|value| value.to_string())
            .collect();
        let actual_denied_packages: BTreeSet<String> = policy
            .rules
            .elevation
            .denied_packages
            .iter()
            .cloned()
            .collect();
        assert_eq!(actual_denied_packages, expected_denied_packages);
    }

    #[test]
    fn load_runtime_rewrites_stale_ruler_root_to_current_instance() {
        let old_root = tempdir().expect("old root tempdir");
        let new_root = tempdir().expect("new root tempdir");
        let runtime_root = tempdir().expect("runtime tempdir");

        init_layout(old_root.path(), Some(runtime_root.path()), None, true)
            .expect("init runtime layout");
        let runtime = load_runtime(new_root.path(), Some(runtime_root.path()))
            .expect("load runtime with new root");
        assert_eq!(
            runtime.config.ruler_root,
            super::canonical_or_original(new_root.path()),
            "runtime config root should follow the current binary root"
        );
    }

    #[test]
    fn installed_runtime_data_root_follows_install_layout() {
        let exe = Path::new("/home/test/.local/share/agent-ruler/installs/dev/agent-ruler");
        let root = installed_user_data_root_from_exe_path(exe).expect("install root");
        assert_eq!(root, Path::new("/home/test/.local/share/agent-ruler"));
    }

    #[test]
    fn installed_runtime_data_root_supports_bin_layout() {
        let exe = Path::new("/home/test/.local/share/agent-ruler/installs/v1/bin/agent-ruler");
        let root = installed_user_data_root_from_exe_path(exe).expect("install root");
        assert_eq!(root, Path::new("/home/test/.local/share/agent-ruler"));
    }

    #[test]
    fn installed_runtime_data_root_ignores_dev_tree_binaries() {
        let exe = Path::new("/home/test/src/agent-ruler/target/debug/agent-ruler");
        assert!(installed_user_data_root_from_exe_path(exe).is_none());
    }

    #[test]
    fn default_delivery_dir_uses_documents_root_without_project_suffix() {
        let ruler_root = Path::new("/tmp/example-project");
        let documents = Path::new("/tmp/docs");
        let delivery = default_delivery_dir_with_roots(
            ruler_root,
            Some(documents.to_path_buf()),
            Some(Path::new("/tmp/home").to_path_buf()),
        );
        assert_eq!(delivery, documents.join("agent-ruler-deliveries"));
    }

    #[test]
    fn default_delivery_dir_falls_back_to_home_documents_when_documents_unavailable() {
        let ruler_root = Path::new("/tmp/example-project");
        let home = Path::new("/tmp/home");
        let delivery = default_delivery_dir_with_roots(ruler_root, None, Some(home.to_path_buf()));
        assert_eq!(
            delivery,
            home.join("Documents").join("agent-ruler-deliveries")
        );
    }

    #[test]
    fn default_delivery_dir_falls_back_to_project_exports_when_no_user_dirs_are_available() {
        let ruler_root = Path::new("/tmp/example-project");
        let delivery = default_delivery_dir_with_roots(ruler_root, None, None);
        assert_eq!(delivery, ruler_root.join("exports"));
    }
}
