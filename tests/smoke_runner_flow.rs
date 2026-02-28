#![cfg(target_os = "linux")]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

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

#[test]
fn smoke_runner_non_interactive_prints_operator_summary() {
    let project = TestRuntimeDir::new("smoke-project");
    let runtime = TestRuntimeDir::new("smoke-runtime");

    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime.path())
        .args(["smoke", "--non-interactive"])
        .output()
        .expect("run smoke command");

    assert!(
        output.status.success(),
        "smoke should complete successfully"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[SMOKE] Agent Ruler one-shot checks"));
    assert!(stdout.contains("[PASS] init --force"));
    assert!(stdout.contains("[INFO] non-interactive summary:"));
    assert!(stdout.contains("[INFO] interactive checks skipped (--non-interactive)"));
}
