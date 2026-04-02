#![cfg(target_os = "linux")]

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

fn configure_runtime_with_openclaw(
    project: &TestRuntimeDir,
    runtime_root: &TestRuntimeDir,
) -> agent_ruler::config::RuntimeState {
    init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
    let mut runtime =
        load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");

    let runner_root = runtime
        .config
        .runtime_root
        .join("user_data")
        .join("openclaw");
    let managed_home = runner_root.join("home");
    let managed_workspace = runner_root.join("workspace");
    fs::create_dir_all(&managed_home).expect("create managed home");
    fs::create_dir_all(&managed_workspace).expect("create managed workspace");

    runtime.config.runner = Some(RunnerAssociation {
        kind: RunnerKind::Openclaw,
        managed_home,
        managed_workspace,
        integrations: Vec::new(),
        missing: RunnerMissingState::default(),
    });

    runtime.policy.rules.execution.deny_workspace_exec = false;
    runtime.policy.rules.execution.deny_tmp_exec = false;

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
fn openclaw_passthrough_preserves_full_downstream_tail() {
    let project = TestRuntimeDir::new("runner-passthrough-project");
    let runtime_root = TestRuntimeDir::new("runner-passthrough-runtime");
    let runtime = configure_runtime_with_openclaw(&project, &runtime_root);

    let capture_path = runner_workspace(&runtime).join("captured-argv.txt");
    write_runner_shim(
        runner_workspace(&runtime),
        "openclaw",
        "printf '%s\\n' \"$@\" > \"$AR_CAPTURE\"",
    );

    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", runner_workspace(&runtime).display(), path_env);

    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args([
            "run",
            "--",
            "openclaw",
            "sessions",
            "cleanup",
            "alpha",
            "beta",
            "--dry-run",
            "--",
            "--all",
        ])
        .env("PATH", merged_path)
        .env("AR_CAPTURE", &capture_path)
        .output()
        .expect("run openclaw command");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if confinement_env_error(&stderr) {
            eprintln!("skipping passthrough assertion due host limits: {stderr}");
            return;
        }
    }
    assert!(
        output.status.success(),
        "openclaw run should succeed; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let captured = fs::read_to_string(&capture_path).expect("read captured argv");
    let got: Vec<String> = captured.lines().map(|line| line.to_string()).collect();
    assert_eq!(
        got,
        vec![
            "sessions".to_string(),
            "cleanup".to_string(),
            "alpha".to_string(),
            "beta".to_string(),
            "--dry-run".to_string(),
            "--".to_string(),
            "--all".to_string(),
        ]
    );
}

#[test]
fn openclaw_passthrough_keeps_tokens_that_look_like_agent_flags() {
    let project = TestRuntimeDir::new("runner-passthrough-flags-project");
    let runtime_root = TestRuntimeDir::new("runner-passthrough-flags-runtime");
    let runtime = configure_runtime_with_openclaw(&project, &runtime_root);

    let capture_path = runner_workspace(&runtime).join("captured-flags.txt");
    write_runner_shim(
        runner_workspace(&runtime),
        "openclaw",
        "printf '%s\\n' \"$@\" > \"$AR_CAPTURE\"",
    );

    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", runner_workspace(&runtime).display(), path_env);

    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args([
            "run",
            "--",
            "openclaw",
            "sessions",
            "cleanup",
            "--background",
            "--foreground",
            "--runtime-dir",
            "nested",
        ])
        .env("PATH", merged_path)
        .env("AR_CAPTURE", &capture_path)
        .output()
        .expect("run openclaw command with downstream flags");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if confinement_env_error(&stderr) {
            eprintln!("skipping passthrough assertion due host limits: {stderr}");
            return;
        }
    }
    assert!(
        output.status.success(),
        "openclaw run should succeed; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let captured = fs::read_to_string(&capture_path).expect("read captured argv");
    let got: Vec<String> = captured.lines().map(|line| line.to_string()).collect();
    assert_eq!(
        got,
        vec![
            "sessions".to_string(),
            "cleanup".to_string(),
            "--background".to_string(),
            "--foreground".to_string(),
            "--runtime-dir".to_string(),
            "nested".to_string(),
        ]
    );
}
