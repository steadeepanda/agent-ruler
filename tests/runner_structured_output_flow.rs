#![cfg(target_os = "linux")]

mod common;

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use agent_ruler::config::{init_layout, load_runtime, save_config, save_policy, CONFIG_FILE_NAME};
use agent_ruler::runners::{RunnerAssociation, RunnerKind, RunnerMissingState};

use common::TestRuntimeDir;

fn bin_path() -> PathBuf {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_agent-ruler") {
        return PathBuf::from(path);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("agent-ruler")
}

fn write_runner_shim(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    let script = format!("#!/usr/bin/env bash\nset -euo pipefail\n{body}\n");
    fs::write(&path, script).expect("write runner shim");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path).expect("shim metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("chmod shim");
    }

    path
}

fn configure_runtime_with_runner(
    project: &TestRuntimeDir,
    runtime_root: &TestRuntimeDir,
    kind: RunnerKind,
) -> agent_ruler::config::RuntimeState {
    init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
    let mut runtime =
        load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");

    let runner_root = runtime
        .config
        .runtime_root
        .join("user_data")
        .join("runners")
        .join(kind.id());
    let managed_home = runner_root.join("home");
    let managed_workspace = runner_root.join("workspace");
    fs::create_dir_all(&managed_home).expect("create managed home");
    fs::create_dir_all(&managed_workspace).expect("create managed workspace");

    runtime.config.runner = Some(RunnerAssociation {
        kind,
        managed_home,
        managed_workspace,
        integrations: Vec::new(),
        missing: RunnerMissingState::default(),
    });

    runtime.policy.rules.execution.deny_workspace_exec = false;
    runtime.policy.rules.execution.deny_tmp_exec = false;

    // Keep each test runtime on a unique UI port to avoid cross-test bind
    // conflicts when runner preflight auto-starts the Agent Ruler UI.
    let ui_listener = TcpListener::bind("127.0.0.1:0").expect("reserve ui bind port");
    let ui_port = ui_listener.local_addr().expect("ui bind addr").port();
    drop(ui_listener);
    runtime.config.ui_bind = format!("127.0.0.1:{ui_port}");

    save_config(
        &runtime.config.state_dir.join(CONFIG_FILE_NAME),
        &runtime.config,
    )
    .expect("save config");
    save_policy(&runtime.config.policy_file, &runtime.policy).expect("save policy");

    runtime
}

fn runner_workspace(runtime: &agent_ruler::config::RuntimeState) -> &Path {
    runtime
        .config
        .runner
        .as_ref()
        .expect("runner config")
        .managed_workspace
        .as_path()
}

fn confinement_env_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("setting up uid map")
        || lower.contains("unprivileged_userns_clone")
        || lower.contains("permission denied")
}

#[test]
fn claudecode_run_appends_structured_output_summary_receipt() {
    let project = TestRuntimeDir::new("runner-structured-claude-project");
    let runtime_root = TestRuntimeDir::new("runner-structured-claude-runtime");
    let runtime = configure_runtime_with_runner(&project, &runtime_root, RunnerKind::Claudecode);

    write_runner_shim(
        runner_workspace(&runtime),
        "claude",
        "echo '{\"type\":\"tool_use\",\"tool_name\":\"write\",\"approval_id\":\"ap-1\"}'",
    );

    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", runner_workspace(&runtime).display(), path_env);

    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["run", "--", "claude", "-p", "reply with exactly ok"])
        .env("PATH", merged_path)
        .output()
        .expect("run claudecode command");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if confinement_env_error(&stderr) {
            eprintln!("skipping claudecode structured-output assertion due host limits: {stderr}");
            return;
        }
    }
    assert!(
        output.status.success(),
        "claudecode run should succeed; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let receipts_raw = fs::read_to_string(runtime.config.receipts_file).expect("read receipts");
    assert!(
        receipts_raw.contains("\"operation\":\"runner_structured_output_parse\""),
        "expected structured-output parse receipt"
    );
    assert!(
        receipts_raw.contains("\"runner_id\":\"claudecode\""),
        "expected claudecode runner id in structured output receipt"
    );
    assert!(
        receipts_raw.contains("\"parser\":\"claude-json\""),
        "expected claude-json parser label in structured output receipt"
    );
}

#[test]
fn opencode_run_appends_structured_output_summary_receipt() {
    let project = TestRuntimeDir::new("runner-structured-opencode-project");
    let runtime_root = TestRuntimeDir::new("runner-structured-opencode-runtime");
    let runtime = configure_runtime_with_runner(&project, &runtime_root, RunnerKind::Opencode);

    write_runner_shim(
        runner_workspace(&runtime),
        "opencode",
        "echo '{\"tool_calls\":[{\"name\":\"write\"}],\"approval_id\":\"ap-2\"}'",
    );

    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", runner_workspace(&runtime).display(), path_env);

    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["run", "--", "opencode", "run", "reply with exactly ok"])
        .env("PATH", merged_path)
        .output()
        .expect("run opencode command");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if confinement_env_error(&stderr) {
            eprintln!("skipping opencode structured-output assertion due host limits: {stderr}");
            return;
        }
    }
    assert!(
        output.status.success(),
        "opencode run should succeed; stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );

    let receipts_raw = fs::read_to_string(runtime.config.receipts_file).expect("read receipts");
    assert!(
        receipts_raw.contains("\"operation\":\"runner_structured_output_parse\""),
        "expected structured-output parse receipt"
    );
    assert!(
        receipts_raw.contains("\"runner_id\":\"opencode\""),
        "expected opencode runner id in structured output receipt"
    );
    assert!(
        receipts_raw.contains("\"parser\":\"opencode-json\""),
        "expected opencode-json parser label in structured output receipt"
    );
}

#[test]
fn run_rejects_runner_mismatch_before_launching_command() {
    let project = TestRuntimeDir::new("runner-structured-mismatch-project");
    let runtime_root = TestRuntimeDir::new("runner-structured-mismatch-runtime");
    let runtime = configure_runtime_with_runner(&project, &runtime_root, RunnerKind::Claudecode);

    write_runner_shim(
        runner_workspace(&runtime),
        "opencode",
        "echo mismatch-shim-should-not-run",
    );

    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", runner_workspace(&runtime).display(), path_env);

    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["run", "--", "opencode", "run", "reply with exactly ok"])
        .env("PATH", merged_path)
        .output()
        .expect("run mismatched runner command");

    assert!(
        !output.status.success(),
        "runner mismatch should fail before command launch"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("runner mismatch"),
        "expected runner mismatch guidance; stderr={stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("mismatch-shim-should-not-run"),
        "runner command should not execute when mapping mismatches"
    );
}

#[test]
fn opencode_run_uses_managed_xdg_paths() {
    let project = TestRuntimeDir::new("runner-structured-opencode-xdg-project");
    let runtime_root = TestRuntimeDir::new("runner-structured-opencode-xdg-runtime");
    let runtime = configure_runtime_with_runner(&project, &runtime_root, RunnerKind::Opencode);
    let managed_home = runtime
        .config
        .runner
        .as_ref()
        .expect("runner config")
        .managed_home
        .to_string_lossy()
        .to_string();

    write_runner_shim(
        runner_workspace(&runtime),
        "opencode",
        "printf '{\"home\":\"%s\",\"xdg_config\":\"%s\",\"xdg_data\":\"%s\",\"xdg_state\":\"%s\",\"xdg_cache\":\"%s\"}\\n' \"$HOME\" \"$XDG_CONFIG_HOME\" \"$XDG_DATA_HOME\" \"$XDG_STATE_HOME\" \"$XDG_CACHE_HOME\"",
    );

    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", runner_workspace(&runtime).display(), path_env);

    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["run", "--", "opencode", "run", "reply with exactly ok"])
        .env("PATH", merged_path)
        .env("HOME", "/tmp/host-home")
        .env("XDG_CONFIG_HOME", "/tmp/host-config")
        .env("XDG_DATA_HOME", "/tmp/host-data")
        .env("XDG_STATE_HOME", "/tmp/host-state")
        .env("XDG_CACHE_HOME", "/tmp/host-cache")
        .output()
        .expect("run opencode command with host xdg overrides");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if confinement_env_error(&stderr) {
            eprintln!("skipping managed xdg assertion due host confinement limits: {stderr}");
            return;
        }
    }
    assert!(
        output.status.success(),
        "opencode run should succeed; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_json_line = stdout
        .lines()
        .find(|line| line.trim_start().starts_with('{'))
        .expect("expected json output line from shim");
    let value: serde_json::Value =
        serde_json::from_str(first_json_line).expect("parse shim json output");
    let xdg = value
        .as_object()
        .expect("json object output from shim should be present");
    let expected_config = format!("{managed_home}/.config");
    let expected_data = format!("{managed_home}/.local/share");
    let expected_state = format!("{managed_home}/.local/state");
    let expected_cache = format!("{managed_home}/.cache");

    assert_eq!(
        xdg.get("home").and_then(|item| item.as_str()),
        Some(managed_home.as_str())
    );
    assert_eq!(
        xdg.get("xdg_config").and_then(|item| item.as_str()),
        Some(expected_config.as_str())
    );
    assert_eq!(
        xdg.get("xdg_data").and_then(|item| item.as_str()),
        Some(expected_data.as_str())
    );
    assert_eq!(
        xdg.get("xdg_state").and_then(|item| item.as_str()),
        Some(expected_state.as_str())
    );
    assert_eq!(
        xdg.get("xdg_cache").and_then(|item| item.as_str()),
        Some(expected_cache.as_str())
    );
}

#[test]
fn openclaw_run_uses_managed_home_and_xdg_paths() {
    let project = TestRuntimeDir::new("runner-structured-openclaw-home-project");
    let runtime_root = TestRuntimeDir::new("runner-structured-openclaw-home-runtime");
    let runtime = configure_runtime_with_runner(&project, &runtime_root, RunnerKind::Openclaw);
    let managed_home = runtime
        .config
        .runner
        .as_ref()
        .expect("runner config")
        .managed_home
        .to_string_lossy()
        .to_string();

    write_runner_shim(
        runner_workspace(&runtime),
        "openclaw",
        "printf '{\"openclaw_home\":\"%s\",\"home\":\"%s\",\"xdg_config\":\"%s\",\"xdg_data\":\"%s\",\"xdg_state\":\"%s\",\"xdg_cache\":\"%s\"}\\n' \"$OPENCLAW_HOME\" \"$HOME\" \"$XDG_CONFIG_HOME\" \"$XDG_DATA_HOME\" \"$XDG_STATE_HOME\" \"$XDG_CACHE_HOME\"",
    );
    write_runner_shim(
        runner_workspace(&runtime),
        "tailscale",
        "echo >&2 'tailscale disabled in test'; exit 1",
    );

    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", runner_workspace(&runtime).display(), path_env);

    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["run", "--", "openclaw", "status"])
        .env("AGENT_RULER_ROOT", env!("CARGO_MANIFEST_DIR"))
        .env("PATH", merged_path)
        .env("OPENCLAW_HOME", "/tmp/host-openclaw")
        .env("HOME", "/tmp/host-home")
        .env("XDG_CONFIG_HOME", "/tmp/host-config")
        .env("XDG_DATA_HOME", "/tmp/host-data")
        .env("XDG_STATE_HOME", "/tmp/host-state")
        .env("XDG_CACHE_HOME", "/tmp/host-cache")
        .output()
        .expect("run openclaw command with host overrides");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if confinement_env_error(&stderr) {
            eprintln!("skipping managed openclaw home assertion due host limits: {stderr}");
            return;
        }
    }
    assert!(
        output.status.success(),
        "openclaw run should succeed; stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_json_line = stdout
        .lines()
        .find(|line| line.trim_start().starts_with('{'))
        .expect("expected json output line from shim");
    let value: serde_json::Value =
        serde_json::from_str(first_json_line).expect("parse shim json output");
    let env_json = value
        .as_object()
        .expect("json object output from shim should be present");
    let expected_config = format!("{managed_home}/.config");
    let expected_data = format!("{managed_home}/.local/share");
    let expected_state = format!("{managed_home}/.local/state");
    let expected_cache = format!("{managed_home}/.cache");

    assert_eq!(
        env_json.get("openclaw_home").and_then(|item| item.as_str()),
        Some(managed_home.as_str())
    );
    assert_eq!(
        env_json.get("home").and_then(|item| item.as_str()),
        Some(managed_home.as_str())
    );
    assert_eq!(
        env_json.get("xdg_config").and_then(|item| item.as_str()),
        Some(expected_config.as_str())
    );
    assert_eq!(
        env_json.get("xdg_data").and_then(|item| item.as_str()),
        Some(expected_data.as_str())
    );
    assert_eq!(
        env_json.get("xdg_state").and_then(|item| item.as_str()),
        Some(expected_state.as_str())
    );
    assert_eq!(
        env_json.get("xdg_cache").and_then(|item| item.as_str()),
        Some(expected_cache.as_str())
    );
}

#[test]
fn claudecode_run_fails_with_login_guidance_when_managed_auth_is_missing() {
    let project = TestRuntimeDir::new("runner-structured-claude-auth-project");
    let runtime_root = TestRuntimeDir::new("runner-structured-claude-auth-runtime");
    let runtime = configure_runtime_with_runner(&project, &runtime_root, RunnerKind::Claudecode);

    write_runner_shim(
        runner_workspace(&runtime),
        "claude",
        r#"if [[ "${1:-}" == "auth" && "${2:-}" == "status" ]]; then
  echo '{"loggedIn":false,"authMethod":"none"}'
  exit 0
fi
echo "claude-run-should-not-start"
exit 0"#,
    );

    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", runner_workspace(&runtime).display(), path_env);

    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["run", "--", "claude", "-p", "reply with exactly ok"])
        .env("PATH", merged_path)
        .output()
        .expect("run claudecode command with missing auth");

    assert!(
        !output.status.success(),
        "missing managed auth should fail with guidance"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("claude auth login"),
        "expected login guidance in stderr; stderr={stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("claude-run-should-not-start"),
        "claude command should stop before launch when managed auth is missing"
    );
}

#[test]
fn non_preflight_runner_commands_still_autostart_ui() {
    let project = TestRuntimeDir::new("runner-structured-ui-autostart-project");
    let runtime_root = TestRuntimeDir::new("runner-structured-ui-autostart-runtime");
    let runtime = configure_runtime_with_runner(&project, &runtime_root, RunnerKind::Opencode);

    write_runner_shim(
        runner_workspace(&runtime),
        "opencode",
        r#"if [[ "${1:-}" == "mcp" && "${2:-}" == "list" ]]; then
  echo "mcp list ok"
  exit 0
fi
echo "unexpected opencode shim args: $*" >&2
exit 1"#,
    );

    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", runner_workspace(&runtime).display(), path_env);

    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["run", "--", "opencode", "mcp", "list"])
        .env("PATH", &merged_path)
        .output()
        .expect("run opencode non-preflight command");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if confinement_env_error(&stderr) {
            eprintln!("skipping runner auto-start assertion due host confinement limits: {stderr}");
            return;
        }
    }
    assert!(
        output.status.success(),
        "opencode mcp list should succeed; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut reachable = false;
    while Instant::now() < deadline {
        if TcpStream::connect(&runtime.config.ui_bind).is_ok() {
            reachable = true;
            break;
        }
        thread::sleep(Duration::from_millis(120));
    }
    assert!(
        reachable,
        "runner command should auto-start Agent Ruler UI at {}",
        runtime.config.ui_bind
    );

    let stop_output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["ui", "stop"])
        .env("PATH", merged_path)
        .output()
        .expect("stop auto-started ui");
    assert!(
        stop_output.status.success(),
        "ui stop should succeed after runner auto-start; stderr={}",
        String::from_utf8_lossy(&stop_output.stderr)
    );
}
