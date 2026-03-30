use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use agent_ruler::config::{load_runtime, RuntimeState};
use agent_ruler::utils::resolve_command_path;

const DEFAULT_GITHUB_REPO: &str = "steadeepanda/agent-ruler";
const EMBEDDED_INSTALL_SH: &str = include_str!("../../install/install.sh");
const GATEWAY_PID_RECORD_FILE_NAME: &str = "openclaw-gateway.pid.json";

#[derive(Debug, Serialize)]
struct UpdateCheckResult {
    checked_at: String,
    current_version: String,
    current_tag: String,
    repo: String,
    latest_tag: String,
    latest_version: String,
    release_url: Option<String>,
    published_at: Option<String>,
    release_notes_markdown: Option<String>,
    update_available: bool,
    requested_tag: Option<String>,
}

#[derive(Debug, Serialize)]
struct UpdateApplyResult {
    checked_at: String,
    current_version: String,
    previous_tag: String,
    target_tag: String,
    repo: String,
    updated: bool,
    release_url: Option<String>,
    release_notes_markdown: Option<String>,
    runner_restarted: bool,
    ui_restart_required: bool,
    install_logs: Option<String>,
    next_steps: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubReleaseAsset {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GitHubReleaseMetadata {
    tag_name: String,
    html_url: Option<String>,
    published_at: Option<String>,
    body: Option<String>,
    assets: Vec<GitHubReleaseAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SemverCore {
    major: u64,
    minor: u64,
    patch: u64,
    pre_release: Option<String>,
}

pub fn run_update(
    ruler_root: &Path,
    runtime_dir: Option<&Path>,
    check_only: bool,
    requested_version: Option<&str>,
    yes: bool,
    json: bool,
    from_ui: bool,
) -> Result<()> {
    let repo = detect_github_repo();
    let requested_tag = requested_version.map(normalize_release_tag);
    let release = resolve_release_metadata(&repo, requested_tag.as_deref())?;
    if !release_has_required_assets(&release) {
        return Err(anyhow!(
            "release {} is missing required assets",
            release.tag_name
        ));
    }

    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let current_tag = format!("v{}", current_version);
    let target_tag = normalize_release_tag(&release.tag_name);
    let target_version = target_tag.trim_start_matches('v').to_string();
    let release_notes_markdown = normalize_release_notes_markdown(release.body.as_deref());
    let update_available = is_newer_tag(&current_tag, &target_tag);

    let check = UpdateCheckResult {
        checked_at: Utc::now().to_rfc3339(),
        current_version: current_version.clone(),
        current_tag: current_tag.clone(),
        repo: repo.clone(),
        latest_tag: target_tag.clone(),
        latest_version: target_version,
        release_url: release.html_url.clone(),
        published_at: release.published_at.clone(),
        release_notes_markdown: release_notes_markdown.clone(),
        update_available,
        requested_tag: requested_tag.clone(),
    };

    if check_only {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "status": "ok",
                    "check": check
                }))?
            );
        } else if update_available {
            println!(
                "Update available: {} -> {} ({})",
                current_tag, target_tag, repo
            );
            if let Some(url) = release.html_url.as_deref() {
                println!("release: {url}");
            }
        } else {
            println!("Already up to date: {} ({})", current_tag, repo);
        }
        return Ok(());
    }

    if requested_tag.is_none() && !update_available {
        let payload = serde_json::json!({
            "status": "up_to_date",
            "check": check,
        });
        if json {
            println!("{}", serde_json::to_string_pretty(&payload)?);
        } else {
            println!("Already up to date: {}", current_tag);
        }
        return Ok(());
    }

    if !yes {
        return Err(anyhow!(
            "update requires --yes to confirm replacing the installed binary"
        ));
    }

    let runtime = load_runtime(ruler_root, runtime_dir).ok();
    let gateway_running_before = runtime
        .as_ref()
        .map(managed_gateway_is_running)
        .unwrap_or(false);

    let install_logs = run_embedded_release_installer(requested_tag.as_deref(), from_ui, json)
        .with_context(|| {
            if from_ui {
                "run embedded release installer from WebUI context"
            } else {
                "run embedded release installer"
            }
        })?;

    let mut runner_restarted = false;
    if gateway_running_before {
        if let Some(runtime) = runtime.as_ref() {
            runner_restarted = restart_managed_gateway(runtime).unwrap_or(false);
        }
    }

    let mut next_steps = Vec::new();
    if from_ui {
        next_steps.push(
            "Refresh the browser after update. If UI assets look stale, run `agent-ruler ui stop` then launch UI again."
                .to_string(),
        );
    }
    if gateway_running_before && !runner_restarted {
        next_steps.push(
            "Managed gateway was running before update but did not auto-restart. Run `agent-ruler run -- openclaw gateway`."
                .to_string(),
        );
    }
    if !gateway_running_before {
        next_steps.push(
            "If needed, start managed gateway with `agent-ruler run -- openclaw gateway`."
                .to_string(),
        );
    }
    next_steps.push("Runtime data/config were preserved (binary/assets only updated).".to_string());

    let apply = UpdateApplyResult {
        checked_at: Utc::now().to_rfc3339(),
        current_version,
        previous_tag: current_tag,
        target_tag,
        repo,
        updated: true,
        release_url: release.html_url,
        release_notes_markdown,
        runner_restarted,
        ui_restart_required: true,
        install_logs: if json { Some(install_logs) } else { None },
        next_steps,
    };

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "updated",
                "result": apply
            }))?
        );
    } else {
        println!("Updated Agent Ruler to {}", apply.target_tag);
        if apply.runner_restarted {
            println!("Managed OpenClaw gateway restarted.");
        }
        for step in &apply.next_steps {
            println!("- {step}");
        }
    }

    Ok(())
}

fn normalize_release_notes_markdown(body: Option<&str>) -> Option<String> {
    let trimmed = body.unwrap_or("").trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn detect_github_repo() -> String {
    env::var("AGENT_RULER_GITHUB_REPO")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_GITHUB_REPO.to_string())
}

fn resolve_release_metadata(
    repo: &str,
    requested_tag: Option<&str>,
) -> Result<GitHubReleaseMetadata> {
    let endpoint = match requested_tag {
        Some(tag) => format!("https://api.github.com/repos/{repo}/releases/tags/{tag}"),
        None => format!("https://api.github.com/repos/{repo}/releases/latest"),
    };
    let body = github_api_get(&endpoint)?;
    serde_json::from_str::<GitHubReleaseMetadata>(&body)
        .with_context(|| format!("parse release metadata from {endpoint}"))
}

fn release_has_required_assets(release: &GitHubReleaseMetadata) -> bool {
    let mut has_archive = false;
    let mut has_sums = false;
    for asset in &release.assets {
        if asset.name == "agent-ruler-linux-x86_64.tar.gz" {
            has_archive = true;
        }
        if asset.name == "SHA256SUMS.txt" {
            has_sums = true;
        }
    }
    has_archive && has_sums
}

fn github_api_get(url: &str) -> Result<String> {
    let mut cmd = Command::new("curl");
    cmd.arg("-fsSL")
        .arg("-H")
        .arg("Accept: application/vnd.github+json");
    if let Ok(token) = env::var("GITHUB_TOKEN") {
        let token = token.trim();
        if !token.is_empty() {
            cmd.arg("-H").arg(format!("Authorization: Bearer {token}"));
        }
    }
    cmd.arg(url);
    let output = cmd
        .output()
        .with_context(|| format!("run curl for {url}"))?;
    if !output.status.success() {
        return Err(anyhow!(
            "GitHub API request failed for {url} (exit {})",
            output.status
        ));
    }
    String::from_utf8(output.stdout).context("decode GitHub API response as UTF-8")
}

fn normalize_release_tag(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with('v') {
        trimmed.to_string()
    } else {
        format!("v{trimmed}")
    }
}

fn is_newer_tag(current_tag: &str, latest_tag: &str) -> bool {
    if current_tag == latest_tag {
        return false;
    }
    match (
        parse_semver_core(current_tag),
        parse_semver_core(latest_tag),
    ) {
        (Some(current), Some(latest)) => latest > current,
        _ => true,
    }
}

fn parse_semver_core(tag: &str) -> Option<SemverCore> {
    let trimmed = tag.trim().trim_start_matches('v');
    let (core, pre_release) = if let Some((left, right)) = trimmed.split_once('-') {
        (left, Some(right.to_string()))
    } else {
        (trimmed, None)
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(SemverCore {
        major,
        minor,
        patch,
        pre_release,
    })
}

impl PartialOrd for SemverCore {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SemverCore {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.major.cmp(&other.major) {
            std::cmp::Ordering::Equal => {}
            order => return order,
        }
        match self.minor.cmp(&other.minor) {
            std::cmp::Ordering::Equal => {}
            order => return order,
        }
        match self.patch.cmp(&other.patch) {
            std::cmp::Ordering::Equal => {}
            order => return order,
        }
        match (&self.pre_release, &other.pre_release) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(_), None) => std::cmp::Ordering::Less,
            (Some(left), Some(right)) => left.cmp(right),
        }
    }
}

fn run_embedded_release_installer(
    requested_tag: Option<&str>,
    skip_stop: bool,
    capture_output: bool,
) -> Result<String> {
    let temp_root = env::temp_dir().join(format!("agent-ruler-update-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_root)
        .with_context(|| format!("create updater temp directory {}", temp_root.display()))?;
    let script_path = temp_root.join("install.sh");
    fs::write(&script_path, EMBEDDED_INSTALL_SH)
        .with_context(|| format!("write embedded installer to {}", script_path.display()))?;
    fs::set_permissions(&script_path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("set executable permissions on {}", script_path.display()))?;

    let mut cmd = Command::new("bash");
    cmd.arg(&script_path).arg("--release");
    if let Some(tag) = requested_tag {
        cmd.arg("--version").arg(tag);
    }
    if skip_stop {
        cmd.env("AGENT_RULER_INSTALL_SKIP_STOP", "1");
    }

    let result = if capture_output {
        let output = cmd.output().context("run embedded installer command")?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if !output.status.success() {
            let _ = fs::remove_dir_all(&temp_root);
            return Err(anyhow!(
                "embedded installer failed (exit {})\nstdout:\n{}\nstderr:\n{}",
                output.status,
                stdout,
                stderr
            ));
        }
        format!("{stdout}{stderr}")
    } else {
        let status = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .context("run embedded installer command")?;
        if !status.success() {
            let _ = fs::remove_dir_all(&temp_root);
            return Err(anyhow!("embedded installer failed (exit {})", status));
        }
        String::new()
    };

    let _ = fs::remove_dir_all(&temp_root);
    Ok(result)
}

fn managed_gateway_is_running(runtime: &RuntimeState) -> bool {
    let record_path = runtime
        .config
        .runtime_root
        .join("user_data")
        .join("logs")
        .join(GATEWAY_PID_RECORD_FILE_NAME);
    let Ok(raw) = fs::read_to_string(record_path) else {
        return false;
    };
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    let Some(pid) = payload
        .get("pid")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as u32)
    else {
        return false;
    };
    process_exists(pid)
}

fn restart_managed_gateway(runtime: &RuntimeState) -> Result<bool> {
    let binary = resolve_command_path("agent-ruler")
        .or_else(|| env::current_exe().ok())
        .ok_or_else(|| anyhow!("resolve agent-ruler binary path for runner restart"))?;

    let _ = Command::new(&binary)
        .arg("--runtime-dir")
        .arg(&runtime.config.runtime_root)
        .args(["run", "--", "openclaw", "gateway", "stop"])
        .status();

    let start = Command::new(&binary)
        .arg("--runtime-dir")
        .arg(&runtime.config.runtime_root)
        .args(["run", "--", "openclaw", "gateway"])
        .status()
        .context("restart managed OpenClaw gateway after update")?;
    Ok(start.success())
}

fn process_exists(pid: u32) -> bool {
    let proc_path = PathBuf::from(format!("/proc/{pid}"));
    if !proc_path.exists() {
        return false;
    }
    let stat_path = proc_path.join("stat");
    if let Ok(stat_raw) = fs::read_to_string(stat_path) {
        let parts: Vec<&str> = stat_raw.split_whitespace().collect();
        if parts.get(2) == Some(&"Z") {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{is_newer_tag, normalize_release_tag, parse_semver_core};

    #[test]
    fn normalize_release_tag_adds_v_prefix() {
        assert_eq!(normalize_release_tag("0.1.7"), "v0.1.7");
        assert_eq!(normalize_release_tag("v0.1.7"), "v0.1.7");
    }

    #[test]
    fn semver_parser_handles_prerelease() {
        let parsed = parse_semver_core("v1.2.3-rc.1").expect("parse semver");
        assert_eq!(parsed.major, 1);
        assert_eq!(parsed.minor, 2);
        assert_eq!(parsed.patch, 3);
        assert_eq!(parsed.pre_release.as_deref(), Some("rc.1"));
    }

    #[test]
    fn newer_tag_detection_prefers_higher_semver() {
        assert!(is_newer_tag("v0.1.6", "v0.1.7"));
        assert!(!is_newer_tag("v0.1.7", "v0.1.6"));
        assert!(!is_newer_tag("v0.1.7", "v0.1.7"));
    }
}
