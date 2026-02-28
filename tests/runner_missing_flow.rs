#![cfg(target_os = "linux")]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

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

fn configure_runner(project_root: &Path, runtime_dir: &Path) {
    let setup_missing_path = TestRuntimeDir::new("runner-missing-setup-path");
    let init = Command::new(bin_path())
        .current_dir(project_root)
        .arg("--runtime-dir")
        .arg(runtime_dir)
        .args(["init", "--force"])
        .output()
        .expect("run init");
    assert!(
        init.status.success(),
        "init failed\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&init.stdout),
        String::from_utf8_lossy(&init.stderr)
    );

    let setup = Command::new(bin_path())
        .current_dir(project_root)
        .arg("--runtime-dir")
        .arg(runtime_dir)
        .arg("setup")
        .env("AGENT_RULER_TEST_ALLOW_MISSING_RUNNER", "1")
        .env("PATH", setup_missing_path.path())
        .output()
        .expect("run setup");
    assert!(
        setup.status.success(),
        "setup failed\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&setup.stdout),
        String::from_utf8_lossy(&setup.stderr)
    );

    let status = Command::new(bin_path())
        .current_dir(project_root)
        .arg("--runtime-dir")
        .arg(runtime_dir)
        .args(["status", "--json"])
        .env("PATH", setup_missing_path.path())
        .output()
        .expect("run status --json after setup");
    assert!(
        status.status.success(),
        "status --json should succeed after setup in test missing-runner mode\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );
    let parsed: Value =
        serde_json::from_slice(&status.stdout).expect("status stdout should be valid json");
    assert_eq!(
        parsed["runner"]["missing_executable"],
        Value::Bool(true),
        "setup test-mode should mark selected runner as missing/unavailable"
    );
}

#[test]
fn status_json_remains_valid_when_runner_missing_non_interactive() {
    let project = TestRuntimeDir::new("runner-missing-json-project");
    let runtime = TestRuntimeDir::new("runner-missing-json-runtime");
    let missing_path_dir = TestRuntimeDir::new("runner-missing-path");

    configure_runner(project.path(), runtime.path());

    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime.path())
        .args(["status", "--json"])
        .env("PATH", missing_path_dir.path())
        .output()
        .expect("run status --json");

    assert!(
        output.status.success(),
        "status --json should succeed\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    serde_json::from_slice::<Value>(&output.stdout).expect("status stdout should be valid json");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("runner check: project is configured for OpenClaw")
            || stderr.contains("runner reminder: `openclaw` is still missing"),
        "missing runner warning should be emitted to stderr in json mode"
    );
    assert!(
        stderr.contains("agent-ruler setup"),
        "stderr should include setup resolution command"
    );
    assert!(
        stderr.contains("agent-ruler runner remove openclaw"),
        "stderr should include cleanup resolution command"
    );
}

#[test]
fn ui_triggers_missing_runner_warning_before_server_loop() {
    let project = TestRuntimeDir::new("runner-missing-ui-project");
    let runtime = TestRuntimeDir::new("runner-missing-ui-runtime");
    let missing_path_dir = TestRuntimeDir::new("runner-missing-ui-path");

    configure_runner(project.path(), runtime.path());

    let mut child = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime.path())
        .args(["ui", "--bind", "127.0.0.1:0"])
        .env("PATH", missing_path_dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ui command");

    thread::sleep(Duration::from_millis(700));
    let _ = child.kill();
    let output = child.wait_with_output().expect("collect ui output");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");
    assert!(
        combined.contains("runner check: project is configured for OpenClaw")
            || combined.contains("runner reminder: `openclaw` is still missing"),
        "ui command should run missing-runner preflight warning\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        combined.contains("agent-ruler setup"),
        "ui warning should include setup command\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        combined.contains("agent-ruler runner remove openclaw"),
        "ui warning should include runner remove command\nstdout={stdout}\nstderr={stderr}"
    );
}

#[test]
fn run_fails_only_when_missing_runner_is_required() {
    let project = TestRuntimeDir::new("runner-missing-required-project");
    let runtime = TestRuntimeDir::new("runner-missing-required-runtime");
    let missing_path_dir = TestRuntimeDir::new("runner-missing-required-path");

    configure_runner(project.path(), runtime.path());

    let status = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime.path())
        .arg("status")
        .env("PATH", missing_path_dir.path())
        .output()
        .expect("run status");
    assert!(
        status.status.success(),
        "status should still succeed even when runner executable is missing"
    );

    let run = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime.path())
        .args(["run", "--", "openclaw", "--help"])
        .env("PATH", missing_path_dir.path())
        .output()
        .expect("run openclaw through runner");
    assert!(
        !run.status.success(),
        "run should fail when configured runner executable is required but missing"
    );
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        stderr.contains("runner `openclaw` is not available"),
        "missing-runner error should be explicit\nstderr={stderr}"
    );
}
