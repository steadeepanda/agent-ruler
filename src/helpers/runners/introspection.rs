use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::config::RuntimeState;
use crate::runners::{RunnerAssociation, RunnerKind};
use crate::utils::resolve_command_path;

const RUNNERS_CACHE_TTL: Duration = Duration::from_secs(30);
const RUNNER_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
static RUNNERS_CACHE: OnceLock<Mutex<BTreeMap<String, RunnersCacheEntry>>> = OnceLock::new();

#[derive(Debug, Clone)]
struct RunnersCacheEntry {
    generated_at: Instant,
    items: Vec<RunnerIntrospectionView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunnerBinaryView {
    pub command: String,
    pub path: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunnerHealthView {
    pub status: String,
    pub handshake: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunnerModeView {
    pub current: String,
    pub supported: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunnerConfigView {
    pub managed_home: Option<String>,
    pub managed_workspace: Option<String>,
    pub integrations: Vec<String>,
    pub masked: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunnerIntrospectionView {
    pub id: String,
    pub label: String,
    pub selected: bool,
    pub configured: bool,
    pub installed: bool,
    pub binary: RunnerBinaryView,
    pub health: RunnerHealthView,
    pub mode: RunnerModeView,
    pub capabilities: Vec<String>,
    pub warnings: Vec<String>,
    pub config: RunnerConfigView,
}

pub fn all_runners_view(runtime: &RuntimeState) -> Vec<RunnerIntrospectionView> {
    [
        RunnerKind::Openclaw,
        RunnerKind::Claudecode,
        RunnerKind::Opencode,
    ]
    .into_iter()
    .map(|kind| runner_view(runtime, kind))
    .collect()
}

pub fn cached_runners_view(
    runtime: &RuntimeState,
    force_refresh: bool,
) -> (Vec<RunnerIntrospectionView>, bool) {
    let runtime_key = runtime_cache_key(runtime);
    let cache = RUNNERS_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut guard = cache.lock().expect("runners cache lock poisoned");

    if !force_refresh {
        if let Some(entry) = guard.get(&runtime_key) {
            let fresh = entry.generated_at.elapsed() <= RUNNERS_CACHE_TTL;
            if fresh {
                return (entry.items.clone(), true);
            }
        }
    }

    let items = all_runners_view(runtime);
    guard.insert(
        runtime_key,
        RunnersCacheEntry {
            generated_at: Instant::now(),
            items: items.clone(),
        },
    );
    (items, false)
}

pub fn runner_view(runtime: &RuntimeState, kind: RunnerKind) -> RunnerIntrospectionView {
    let selected_kind = runtime.config.runner.as_ref().map(|runner| runner.kind);
    let selected = selected_kind == Some(kind);
    let configured_runner = runtime
        .config
        .runner
        .as_ref()
        .filter(|runner| runner.kind == kind);
    let configured = configured_runner.is_some();

    let resolved_path = resolve_command_path(kind.executable_name());
    let installed = resolved_path.is_some();
    let version = resolve_version(kind, resolved_path.as_ref());
    let capabilities = detect_capabilities(kind);
    let mode = mode_view(kind);
    let health = health_view(selected, installed, configured_runner);
    let config = config_view(kind, configured_runner);
    let warnings = build_warnings(kind, configured, installed, version.as_deref());

    RunnerIntrospectionView {
        id: kind.id().to_string(),
        label: kind.display_name().to_string(),
        selected,
        configured,
        installed,
        binary: RunnerBinaryView {
            command: kind.executable_name().to_string(),
            path: resolved_path.as_ref().map(|path| path_to_string(path)),
            version,
        },
        health,
        mode,
        capabilities,
        warnings,
        config,
    }
}

fn runtime_cache_key(runtime: &RuntimeState) -> String {
    let runner_key = runtime
        .config
        .runner
        .as_ref()
        .map(|runner| {
            format!(
                "{}|{}|{}",
                runner.kind.id(),
                path_to_string(&runner.managed_home),
                path_to_string(&runner.managed_workspace)
            )
        })
        .unwrap_or_else(|| "none".to_string());
    let path_env = std::env::var("PATH").unwrap_or_default();
    format!(
        "{}|{}|{}|{}",
        path_to_string(&runtime.config.ruler_root),
        path_to_string(&runtime.config.runtime_root),
        runner_key,
        path_env
    )
}

fn resolve_version(kind: RunnerKind, resolved_path: Option<&PathBuf>) -> Option<String> {
    let executable = resolved_path
        .map(|path| path.as_path())
        .unwrap_or_else(|| Path::new(kind.executable_name()));
    let output =
        command_output_with_timeout(executable, &["--version"], RUNNER_VERSION_PROBE_TIMEOUT)?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stdout.is_empty() {
        return Some(stdout);
    }
    if !stderr.is_empty() {
        return Some(stderr);
    }
    None
}

fn command_output_with_timeout(
    executable: &Path,
    args: &[&str],
    timeout: Duration,
) -> Option<std::process::Output> {
    let mut child = Command::new(executable)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(_) => return None,
        }
    }
}

fn detect_capabilities(kind: RunnerKind) -> Vec<String> {
    let mut output = Vec::new();
    match kind {
        RunnerKind::Openclaw => {
            output.push("gateway".to_string());
            output.push("managed_config".to_string());
            output.push("tools_adapter".to_string());
        }
        RunnerKind::Claudecode => {
            output.push("json_output".to_string());
            output.push("mcp_bridge".to_string());
            output.push("permission_controls".to_string());
            output.push("sandbox_controls".to_string());
            output.push("streaming_output".to_string());
            output.push("tool_mapping".to_string());
        }
        RunnerKind::Opencode => {
            output.push("command_mode".to_string());
            output.push("mcp_bridge".to_string());
            output.push("service_mode".to_string());
            output.push("tool_mapping".to_string());
        }
    }

    output.sort();
    output.dedup();
    output
}

fn mode_view(kind: RunnerKind) -> RunnerModeView {
    match kind {
        RunnerKind::Openclaw => RunnerModeView {
            current: "service".to_string(),
            supported: vec!["one_shot".to_string(), "service".to_string()],
        },
        RunnerKind::Claudecode => RunnerModeView {
            current: "one_shot".to_string(),
            supported: vec!["one_shot".to_string()],
        },
        RunnerKind::Opencode => RunnerModeView {
            current: "one_shot".to_string(),
            supported: vec!["one_shot".to_string(), "service".to_string()],
        },
    }
}

fn health_view(
    selected: bool,
    installed: bool,
    configured: Option<&RunnerAssociation>,
) -> RunnerHealthView {
    if !selected {
        return RunnerHealthView {
            status: "not_selected".to_string(),
            handshake: if configured.is_some() {
                "configured_not_selected".to_string()
            } else {
                "n/a".to_string()
            },
        };
    }

    if !installed {
        return RunnerHealthView {
            status: "binary_missing".to_string(),
            handshake: "unavailable".to_string(),
        };
    }

    if let Some(association) = configured {
        let ready = association.managed_home.exists() && association.managed_workspace.exists();
        return RunnerHealthView {
            status: if ready {
                "ready".to_string()
            } else {
                "managed_state_missing".to_string()
            },
            handshake: if ready {
                "ok".to_string()
            } else {
                "state_not_ready".to_string()
            },
        };
    }

    RunnerHealthView {
        status: "installed_unconfigured".to_string(),
        handshake: "n/a".to_string(),
    }
}

fn config_view(kind: RunnerKind, configured: Option<&RunnerAssociation>) -> RunnerConfigView {
    let mut masked = BTreeMap::new();
    for key in env_override_keys(kind) {
        masked.insert(key.to_string(), "<managed_path>".to_string());
    }

    let (managed_home, managed_workspace, integrations) = if let Some(runner) = configured {
        (
            Some(path_to_string(&runner.managed_home)),
            Some(path_to_string(&runner.managed_workspace)),
            runner.integrations.clone(),
        )
    } else {
        (None, None, Vec::new())
    };

    RunnerConfigView {
        managed_home,
        managed_workspace,
        integrations,
        masked,
    }
}

fn env_override_keys(kind: RunnerKind) -> &'static [&'static str] {
    match kind {
        RunnerKind::Openclaw => &["OPENCLAW_HOME"],
        RunnerKind::Claudecode => &["CLAUDE_CONFIG_DIR", "HOME"],
        RunnerKind::Opencode => &[
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_STATE_HOME",
            "XDG_CACHE_HOME",
        ],
    }
}

fn build_warnings(
    kind: RunnerKind,
    configured: bool,
    installed: bool,
    version: Option<&str>,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if configured && !installed {
        warnings.push(format!(
            "Configured runner is missing executable `{}` in PATH.",
            kind.executable_name()
        ));
    }
    if installed && version.is_none() {
        warnings.push("Unable to read runner version via `--version`.".to_string());
    }
    if kind == RunnerKind::Opencode {
        warnings.push(
            "Service/web modes can persist session state; keep managed runtime paths under review."
                .to_string(),
        );
    }
    warnings
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use super::command_output_with_timeout;

    #[test]
    fn command_output_with_timeout_returns_output_for_fast_command() {
        let output = command_output_with_timeout(
            Path::new("sh"),
            &["-c", "printf ok"],
            Duration::from_millis(500),
        )
        .expect("fast command should complete");
        assert!(
            output.status.success(),
            "expected successful command status"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "ok",
            "expected command stdout payload"
        );
    }

    #[test]
    fn command_output_with_timeout_returns_none_for_hung_command() {
        let output = command_output_with_timeout(
            Path::new("sh"),
            &["-c", "sleep 2"],
            Duration::from_millis(50),
        );
        assert!(
            output.is_none(),
            "hung command should time out and return no output"
        );
    }
}
