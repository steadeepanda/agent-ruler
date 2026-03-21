#![cfg(target_os = "linux")]

mod common;

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use agent_ruler::config::{init_layout, load_runtime, save_config, save_policy, CONFIG_FILE_NAME};
use agent_ruler::runners::{RunnerAssociation, RunnerKind, RunnerMissingState};
use serde_json::Value;

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
    // conflicts when runner web preflight auto-starts Agent Ruler UI.
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

fn ss_listener_line_for_port(port: u16) -> Option<String> {
    let output = Command::new("ss").args(["-ltnp"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let needle = format!(":{port}");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|line| line.contains(&needle))
        .map(|line| line.to_string())
}

fn process_exists(pid: u64) -> bool {
    PathBuf::from(format!("/proc/{pid}")).exists()
}

fn confinement_env_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("setting up uid map")
        || lower.contains("unprivileged_userns_clone")
        || lower.contains("permission denied")
        || lower.contains("bubblewrap (bwrap) is required")
        || lower.contains("confinement unavailable")
}

#[test]
fn opencode_web_refuses_unmanaged_listener_on_requested_port() {
    let project = TestRuntimeDir::new("runner-web-opencode-project");
    let runtime_root = TestRuntimeDir::new("runner-web-opencode-runtime");
    let runtime = configure_runtime_with_runner(&project, &runtime_root, RunnerKind::Opencode);

    let marker = runtime
        .config
        .workspace
        .join("opencode-web-should-not-run.txt");
    write_runner_shim(
        &runtime.config.workspace,
        "opencode",
        &format!("echo launched > '{}'\nexit 0", marker.display()),
    );

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind unmanaged listener");
    let port = listener
        .local_addr()
        .expect("listener addr")
        .port()
        .to_string();

    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", runtime.config.workspace.display(), path_env);

    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["run", "--", "opencode", "web", "--port"])
        .arg(&port)
        .env("PATH", merged_path)
        .output()
        .expect("run opencode web command");

    assert!(
        !output.status.success(),
        "opencode web should refuse unmanaged listener conflicts"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("port") && stderr.contains("already in use"),
        "expected explicit port-conflict guidance\nstderr={stderr}"
    );
    assert!(
        !marker.exists(),
        "opencode shim should not execute when the port is already occupied"
    );

    drop(listener);
}

#[test]
fn opencode_web_records_listener_pid_and_stop_terminates_listener() {
    let project = TestRuntimeDir::new("runner-web-opencode-managed-project");
    let runtime_root = TestRuntimeDir::new("runner-web-opencode-managed-runtime");
    let runtime = configure_runtime_with_runner(&project, &runtime_root, RunnerKind::Opencode);

    write_runner_shim(
        &runtime.config.workspace,
        "opencode",
        r#"if [[ "${1:-}" != "web" ]]; then
  echo "unsupported test shim mode: ${1:-}" >&2
  exit 1
fi
PORT="0"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --port)
      shift
      PORT="${1:-0}"
      ;;
    --port=*)
      PORT="${1#--port=}"
      ;;
  esac
  shift || true
done
if [[ "$PORT" == "0" ]]; then
  echo "missing --port for test shim" >&2
  exit 1
fi
nohup python3 -m http.server "$PORT" --bind 127.0.0.1 >/dev/null 2>&1 &
echo "OpenCode Web UI available at http://127.0.0.1:$PORT/"
sleep 0.2
exit 0"#,
    );

    let test_listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral listener");
    let port = test_listener.local_addr().expect("listener addr").port();
    drop(test_listener);

    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", runtime.config.workspace.display(), path_env);

    let launch_output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args([
            "run",
            "--",
            "opencode",
            "web",
            "--hostname",
            "127.0.0.1",
            "--port",
        ])
        .arg(port.to_string())
        .env("PATH", &merged_path)
        .output()
        .expect("launch managed opencode web command");

    if !launch_output.status.success() {
        let stderr = String::from_utf8_lossy(&launch_output.stderr);
        if confinement_env_error(&stderr) {
            eprintln!(
                "skipping managed opencode web lifecycle assertion due host limits: {stderr}"
            );
            return;
        }
    }
    assert!(
        launch_output.status.success(),
        "opencode web launch should succeed; stderr={}",
        String::from_utf8_lossy(&launch_output.stderr)
    );

    let pid_record_path = runtime
        .config
        .runtime_root
        .join("user_data")
        .join("logs")
        .join("opencode-web.pid.json");
    let pid_record_raw = fs::read_to_string(&pid_record_path).expect("read managed pid record");
    let pid_record: Value =
        serde_json::from_str(&pid_record_raw).expect("parse managed pid record");
    let recorded_pid = pid_record
        .get("pid")
        .and_then(Value::as_u64)
        .expect("managed pid record should include pid");

    let stable_deadline = Instant::now() + Duration::from_secs(5);
    let mut observed_line = None;
    while Instant::now() < stable_deadline {
        if let Some(line) = ss_listener_line_for_port(port) {
            if line.contains(&format!("pid={recorded_pid}")) {
                observed_line = Some(line);
                break;
            }
        }
        thread::sleep(Duration::from_millis(120));
    }
    assert!(
        observed_line.is_some(),
        "managed pid record should track active listener owner on port {port}; recorded_pid={recorded_pid}"
    );

    let stop_output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["run", "--", "opencode", "web", "stop"])
        .env("PATH", &merged_path)
        .output()
        .expect("stop managed opencode web command");

    assert!(
        stop_output.status.success(),
        "opencode web stop should succeed; stderr={}",
        String::from_utf8_lossy(&stop_output.stderr)
    );

    let stop_deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < stop_deadline {
        if ss_listener_line_for_port(port).is_none() && !process_exists(recorded_pid) {
            break;
        }
        thread::sleep(Duration::from_millis(120));
    }

    assert!(
        ss_listener_line_for_port(port).is_none(),
        "listener on {port} should be stopped by managed stop path"
    );
    assert!(
        !process_exists(recorded_pid),
        "managed listener pid {recorded_pid} should not remain alive after stop"
    );
    assert!(
        !pid_record_path.exists(),
        "managed pid record should be removed after successful stop"
    );
}

#[test]
fn claudecode_web_alias_fails_fast_with_native_command_guidance() {
    let project = TestRuntimeDir::new("runner-web-claude-alias-project");
    let runtime_root = TestRuntimeDir::new("runner-web-claude-alias-runtime");
    let runtime = configure_runtime_with_runner(&project, &runtime_root, RunnerKind::Claudecode);

    let marker = runtime
        .config
        .workspace
        .join("claude-web-alias-should-not-run.txt");
    write_runner_shim(
        &runtime.config.workspace,
        "claude",
        &format!("echo invoked > '{}'\nexit 0", marker.display()),
    );

    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", runtime.config.workspace.display(), path_env);

    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["run", "--", "claude", "web"])
        .env("PATH", merged_path)
        .output()
        .expect("run claudecode legacy web command");

    assert!(
        !output.status.success(),
        "legacy claude web alias should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("claude remote-control"),
        "expected native-command guidance for legacy alias\nstderr={stderr}"
    );
    assert!(
        !marker.exists(),
        "legacy alias should fail before runner command execution"
    );
}
