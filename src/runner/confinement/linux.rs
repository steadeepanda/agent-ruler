//! Linux confinement using bubblewrap (bwrap).
//!
//! This module provides the Linux implementation of command confinement using
//! bubblewrap for namespace-based isolation.
//!
//! # Requirements
//!
//! - `bwrap` (bubblewrap) must be installed and accessible in PATH
//! - Linux kernel with namespace support (standard in modern kernels)
//!
//! # Sandbox Configuration
//!
//! The sandbox is configured with:
//! - Read-only root filesystem (prevents modification of system files)
//! - Separate /proc and /dev (prevents access to host process info)
//! - Ephemeral /tmp and /run (prevents persistence)
//! - Writable workspace bind mount
//! - Read-only shared zone bind mount
//! - Secret paths masked with empty files/directories
//! - Runtime state files masked
//! - Optional network namespace for full deny mode

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};

use crate::config::RuntimeState;
use crate::policy::PolicyEngine;
use crate::utils::looks_like_glob;

/// Check if bubblewrap is available on this system.
pub fn is_available() -> bool {
    Command::new("bwrap")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build a bubblewrap command for Linux confinement.
///
/// Creates a sandboxed execution environment with:
/// - Read-only root filesystem
/// - Writable workspace
/// - Masked secrets and runtime state
/// - Optional network isolation
pub fn build_confined_command(
    cmd: &[String],
    runtime: &RuntimeState,
    engine: &PolicyEngine,
) -> Result<Command> {
    ensure_bwrap_available()?;

    let workspace = runtime
        .config
        .workspace
        .canonicalize()
        .unwrap_or_else(|_| runtime.config.workspace.clone());
    let shared_zone = runtime
        .config
        .shared_zone_dir
        .canonicalize()
        .unwrap_or_else(|_| runtime.config.shared_zone_dir.clone());

    let mut command = Command::new("bwrap");
    command
        // Security: terminate sandbox when parent dies
        .arg("--die-with-parent")
        // Security: new session to prevent terminal access
        .arg("--new-session")
        // Read-only root filesystem (prevents modification of system files)
        .arg("--ro-bind")
        .arg("/")
        .arg("/")
        // Isolated /proc (prevents access to /proc/<pid>/* of host processes)
        .arg("--proc")
        .arg("/proc")
        // Minimal /dev (no direct hardware access)
        .arg("--dev")
        .arg("/dev")
        // Ephemeral /tmp (prevents persistence via temp)
        .arg("--tmpfs")
        .arg("/tmp")
        // Ephemeral /run (prevents access to host runtime state)
        .arg("--tmpfs")
        .arg("/run")
        // Writable workspace (agent's working directory)
        .arg("--bind")
        .arg(&workspace)
        .arg(&workspace)
        // Read-only shared zone (agent can read but not modify exports)
        .arg("--ro-bind")
        .arg(&shared_zone)
        .arg(&shared_zone)
        // Start in workspace directory
        .arg("--chdir")
        .arg(&workspace);

    // Preserve host DNS configuration when /etc/resolv.conf points into /run.
    add_dns_runtime_bind(&mut command)?;

    // Runner-managed homes (for example OpenClaw) live under runtime user_data
    // and must be writable inside confinement for project-local runner state.
    for path in runner_managed_data_paths(runtime) {
        command.arg("--bind").arg(&path).arg(&path);
    }

    // Security: mask secrets with empty files/dirs
    add_secret_masks(&mut command, runtime, engine)?;
    // Security: mask agent-ruler's own state files and runtime internals
    add_runtime_state_masks(&mut command, runtime)?;

    // Security: use kernel-level network namespace only when policy represents
    // an effective "deny all" posture. If explicit host mediation is configured,
    // keep network namespace shared so deterministic host rules can apply.
    if should_unshare_network_namespace(engine, cmd) {
        command.arg("--unshare-net");
    }

    command.arg("--");
    for part in cmd {
        command.arg(part);
    }

    Ok(command)
}

fn add_dns_runtime_bind(command: &mut Command) -> Result<()> {
    let Some(bind_target) =
        resolv_conf_bind_target(Path::new("/etc/resolv.conf"), Path::new("/run"))
    else {
        return Ok(());
    };

    let Some(parent) = bind_target.parent() else {
        return Ok(());
    };
    for dir in mount_dir_chain(parent) {
        if dir == Path::new("/run") {
            continue;
        }
        command.arg("--dir").arg(dir);
    }

    command.arg("--ro-bind").arg(&bind_target).arg(&bind_target);
    Ok(())
}

fn resolv_conf_bind_target(resolv_conf_path: &Path, runtime_root: &Path) -> Option<PathBuf> {
    let resolved = resolv_conf_path.canonicalize().ok()?;
    if resolved.starts_with(runtime_root) {
        Some(resolved)
    } else {
        None
    }
}

fn mount_dir_chain(path: &Path) -> Vec<PathBuf> {
    if !path.is_absolute() {
        return Vec::new();
    }

    let mut output = Vec::new();
    let mut current = PathBuf::from("/");
    for part in path.components().skip(1) {
        current.push(part.as_os_str());
        output.push(current.clone());
    }
    output
}

/// Ensure bubblewrap is available, returning an error if not.
fn ensure_bwrap_available() -> Result<()> {
    if !is_available() {
        return Err(anyhow!(
            "bubblewrap (bwrap) is required for Linux confinement; install the bubblewrap package"
        ));
    }
    Ok(())
}

fn runner_managed_data_paths(runtime: &RuntimeState) -> Vec<PathBuf> {
    let mut output = Vec::new();
    let Some(runner) = runtime.config.runner.as_ref() else {
        return output;
    };
    let path = runner
        .managed_home
        .canonicalize()
        .unwrap_or_else(|_| runner.managed_home.clone());

    if !path.exists() {
        return output;
    }
    if !path.starts_with(&runtime.config.runtime_root) {
        return output;
    }
    if path.starts_with(&runtime.config.workspace) {
        return output;
    }

    output.push(path);
    output
}

/// Mask secret paths with empty files or directories.
///
/// This prevents accidental credential access even if the policy is misconfigured.
fn add_secret_masks(
    command: &mut Command,
    runtime: &RuntimeState,
    engine: &PolicyEngine,
) -> Result<()> {
    let empty_dir = runtime.config.state_dir.join("empty-secret-dir");
    let empty_file = runtime.config.state_dir.join("empty-secret-file");
    fs::create_dir_all(&empty_dir).with_context(|| format!("create {}", empty_dir.display()))?;
    if !empty_file.exists() {
        fs::write(&empty_file, "").with_context(|| format!("create {}", empty_file.display()))?;
    }

    for pattern in &engine.policy().zones.secrets_paths {
        if looks_like_glob(pattern) {
            continue;
        }
        let path = PathBuf::from(pattern);
        if !path.is_absolute() || !path.exists() {
            continue;
        }
        if path.is_dir() {
            command.arg("--ro-bind").arg(&empty_dir).arg(path);
        } else {
            command.arg("--ro-bind").arg(&empty_file).arg(path);
        }
    }

    Ok(())
}

/// Mask Agent Ruler's runtime state files.
///
/// This prevents agents from reading or modifying security configuration.
///
/// # Strategy
///
/// We mask specific sensitive paths within the runtime directory structure:
/// - state/ directory (contains policy.yaml, approvals.json, receipts.jsonl, etc.)
/// - exports/ directory (default delivery destination)
/// - exec-layer/ directory (ephemeral execution state)
/// - quarantine/ directory (quarantined files)
/// - Any other internal state files
///
/// We do NOT mask the entire runtime_root because workspace and shared-zone
/// are subdirectories of runtime_root and need to remain accessible. Instead,
/// we explicitly mask each sensitive subdirectory.
///
/// # Mount Order
///
/// Since we use --tmpfs /tmp, paths under /tmp don't exist in the sandbox until
/// we create them with --dir or bind-mount them. We need to ensure parent
/// directories exist before bind-mounting mask targets.
fn add_runtime_state_masks(command: &mut Command, runtime: &RuntimeState) -> Result<()> {
    // Use system temp directory for mask artifacts to avoid conflicts with /tmp tmpfs
    let mask_root = std::env::temp_dir().join("agent-ruler-sandbox-masks");
    let empty_dir = mask_root.join("empty-dir");
    let empty_file = mask_root.join("empty-file");

    fs::create_dir_all(&empty_dir).with_context(|| format!("create {}", empty_dir.display()))?;
    if !empty_file.exists() {
        fs::write(&empty_file, "").with_context(|| format!("create {}", empty_file.display()))?;
    }

    // Collect directory paths that should be hidden from the confined agent.
    // We focus on top-level directories because masking a directory with an empty one
    // automatically hides all its contents (no need to mask subdirectories).
    let mut hidden_dirs: Vec<PathBuf> = Vec::new();

    // Always mask the state directory - this contains all runtime internals
    // (policy.yaml, approvals.json, receipts.jsonl, exec-layer/, etc.)
    hidden_dirs.push(runtime.config.state_dir.clone());

    // Also mask the exports directory if it exists within runtime_root
    // (this is the default delivery destination, not operator-accessible)
    let exports_dir = runtime.config.runtime_root.join("exports");
    if exports_dir.exists() && exports_dir.is_dir() {
        hidden_dirs.push(exports_dir);
    }

    // Mask quarantine if it's directly under runtime_root (not inside state/)
    let quarantine = runtime.config.quarantine_dir.clone();
    if quarantine.is_absolute()
        && quarantine.exists()
        && quarantine.is_dir()
        && !quarantine.starts_with(&runtime.config.state_dir)
    {
        hidden_dirs.push(quarantine);
    }

    hidden_dirs.sort();
    hidden_dirs.dedup();

    for path in hidden_dirs {
        if !path.is_absolute() || !path.exists() {
            continue;
        }
        // Skip paths that are inside workspace or shared_zone - those are explicitly
        // exposed to the agent and should remain accessible
        if path.starts_with(&runtime.config.workspace)
            || path.starts_with(&runtime.config.shared_zone_dir)
        {
            continue;
        }

        // For paths under /tmp, we need to create the directory structure first
        // because --tmpfs /tmp replaces /tmp with an empty tmpfs
        if path.starts_with("/tmp") {
            // Create parent directories for the mount point
            if let Some(parent) = path.parent() {
                for dir in mount_dir_chain(parent) {
                    // Skip /tmp itself - it's already a tmpfs
                    if dir == Path::new("/tmp") {
                        continue;
                    }
                    command.arg("--dir").arg(dir);
                }
            }
        }

        // Mask the directory with an empty directory
        command.arg("--ro-bind").arg(&empty_dir).arg(path);
    }

    Ok(())
}

fn should_unshare_network_namespace(engine: &PolicyEngine, cmd: &[String]) -> bool {
    // OpenClaw gateway should remain host-visible on its configured port
    // (typically 18789) so host tooling and tailnet workflows can reach it.
    if is_openclaw_gateway_run_command(cmd) {
        return false;
    }

    let rules = &engine.policy().rules.network;
    if !rules.default_deny {
        return false;
    }

    // Explicit host allows can come from direct allowlist matching, or from an
    // inverted denylist used as an allow-set.
    let allows_via_allowlist = !rules.invert_allowlist && !rules.allowlist_hosts.is_empty();
    let allows_via_inverted_denylist = rules.invert_denylist && !rules.denylist_hosts.is_empty();

    !(allows_via_allowlist || allows_via_inverted_denylist)
}

fn is_openclaw_gateway_run_command(cmd: &[String]) -> bool {
    let tokens = command_tokens_without_env_prefix(cmd);
    if tokens.len() < 2 {
        return false;
    }
    if tokens[0] != "openclaw" || tokens[1] != "gateway" {
        return false;
    }
    !tokens
        .iter()
        .skip(2)
        .any(|token| *token == "stop" || *token == "status")
}

fn command_tokens_without_env_prefix(cmd: &[String]) -> Vec<&str> {
    if cmd.is_empty() {
        return Vec::new();
    }
    if cmd[0] != "env" {
        return cmd.iter().map(String::as_str).collect();
    }

    let mut out: Vec<&str> = Vec::new();
    let mut index = 1usize;
    while index < cmd.len() {
        let token = cmd[index].as_str();
        if token.contains('=') {
            index += 1;
            continue;
        }
        out.extend(cmd[index..].iter().map(String::as_str));
        return out;
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::config::Policy;
    use crate::policy::PolicyEngine;
    use tempfile::tempdir;

    use super::{mount_dir_chain, resolv_conf_bind_target, should_unshare_network_namespace};

    fn make_engine(mut policy: Policy) -> PolicyEngine {
        let workspace = PathBuf::from("/tmp/agent-ruler-linux-confinement-tests");
        policy = policy.expanded(&workspace);
        PolicyEngine::new(policy, workspace)
    }

    fn base_policy() -> Policy {
        serde_yaml::from_str(include_str!("../../../assets/default-policy.yaml"))
            .expect("default policy should parse")
    }

    #[test]
    fn unshares_network_for_effective_deny_all_mode() {
        let mut policy = base_policy();
        policy.rules.network.default_deny = true;
        policy.rules.network.allowlist_hosts.clear();
        policy.rules.network.denylist_hosts.clear();
        policy.rules.network.invert_allowlist = false;
        policy.rules.network.invert_denylist = false;

        let engine = make_engine(policy);
        assert!(should_unshare_network_namespace(&engine, &[]));
    }

    #[test]
    fn does_not_unshare_when_allowlist_can_admit_hosts() {
        let mut policy = base_policy();
        policy.rules.network.default_deny = true;
        policy.rules.network.allowlist_hosts = vec!["api.example.org".to_string()];
        policy.rules.network.invert_allowlist = false;

        let engine = make_engine(policy);
        assert!(!should_unshare_network_namespace(&engine, &[]));
    }

    #[test]
    fn does_not_unshare_when_inverted_denylist_is_used_as_allowset() {
        let mut policy = base_policy();
        policy.rules.network.default_deny = true;
        policy.rules.network.denylist_hosts = vec!["api.example.org".to_string()];
        policy.rules.network.invert_denylist = true;

        let engine = make_engine(policy);
        assert!(!should_unshare_network_namespace(&engine, &[]));
    }

    #[test]
    fn does_not_unshare_for_openclaw_gateway_run_commands() {
        let mut policy = base_policy();
        policy.rules.network.default_deny = true;
        policy.rules.network.allowlist_hosts.clear();
        policy.rules.network.denylist_hosts.clear();
        policy.rules.network.invert_allowlist = false;
        policy.rules.network.invert_denylist = false;

        let engine = make_engine(policy);
        let cmd = vec![
            "env".to_string(),
            "OPENCLAW_HOME=/tmp/openclaw".to_string(),
            "openclaw".to_string(),
            "gateway".to_string(),
            "run".to_string(),
        ];
        assert!(!should_unshare_network_namespace(&engine, &cmd));
    }

    #[cfg(unix)]
    #[test]
    fn resolves_runtime_backed_resolv_conf_target() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path();
        let etc = root.join("etc");
        let run_dir = root.join("run/systemd/resolve");
        fs::create_dir_all(&etc).expect("create etc");
        fs::create_dir_all(&run_dir).expect("create runtime dir");
        let stub = run_dir.join("stub-resolv.conf");
        fs::write(&stub, "nameserver 127.0.0.53\n").expect("write stub resolv.conf");
        std::os::unix::fs::symlink(
            "../run/systemd/resolve/stub-resolv.conf",
            etc.join("resolv.conf"),
        )
        .expect("create resolv.conf symlink");

        let resolved = resolv_conf_bind_target(&etc.join("resolv.conf"), &root.join("run"))
            .expect("resolve runtime target");
        assert_eq!(resolved, stub);
    }

    #[cfg(unix)]
    #[test]
    fn ignores_non_runtime_resolv_conf_target() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path();
        let etc = root.join("etc");
        let var = root.join("var/lib");
        fs::create_dir_all(&etc).expect("create etc");
        fs::create_dir_all(&var).expect("create var dir");
        let target = var.join("resolv.conf");
        fs::write(&target, "nameserver 1.1.1.1\n").expect("write target");
        std::os::unix::fs::symlink("../var/lib/resolv.conf", etc.join("resolv.conf"))
            .expect("create resolv.conf symlink");

        let resolved = resolv_conf_bind_target(&etc.join("resolv.conf"), &root.join("run"));
        assert!(resolved.is_none());
    }

    #[test]
    fn builds_absolute_mount_dir_chain() {
        let chain = mount_dir_chain(Path::new("/run/systemd/resolve"));
        assert_eq!(
            chain,
            vec![
                PathBuf::from("/run"),
                PathBuf::from("/run/systemd"),
                PathBuf::from("/run/systemd/resolve"),
            ]
        );
    }
}
