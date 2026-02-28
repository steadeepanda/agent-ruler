use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Context, Result};

pub fn run_manual_smoke(
    ruler_root: &Path,
    runtime_dir: Option<&Path>,
    non_interactive: bool,
) -> Result<()> {
    println!("[SMOKE] Agent Ruler one-shot checks");

    let mut pass = 0usize;
    let mut fail = 0usize;
    let mut skip = 0usize;

    let init = run_self(ruler_root, runtime_dir, &["init", "--force"])?;
    if init.status.success() {
        pass += 1;
        println!("[PASS] init --force");
    } else {
        fail += 1;
        println!("[FAIL] init --force");
        print_hint(
            &init,
            "run `agent-ruler init --force` manually and inspect stderr",
        );
    }

    let write_ok = run_self(
        ruler_root,
        runtime_dir,
        &[
            "run",
            "--",
            "bash",
            "-lc",
            "echo smoke-ok > smoke-normal.txt",
        ],
    )?;
    if write_ok.status.success() {
        pass += 1;
        println!("[PASS] confined workspace write");
    } else if is_confinement_env_error(&write_ok.stderr) {
        skip += 1;
        println!("[SKIP] confined workspace write (host blocks user namespaces)");
        print_hint(
            &write_ok,
            "enable unprivileged user namespaces or run in host/VM profile that permits bubblewrap",
        );
    } else {
        fail += 1;
        println!("[FAIL] confined workspace write");
        print_hint(&write_ok, "expected success writing inside workspace");
    }

    let rm_block = run_self(ruler_root, runtime_dir, &["run", "--", "rm", "/etc/passwd"])?;
    let rm_tail = run_self(ruler_root, runtime_dir, &["tail", "25"])?;
    if !rm_block.status.success()
        && rm_tail
            .stdout
            .contains("\"reason\": \"deny_system_critical\"")
    {
        pass += 1;
        println!("[PASS] system delete denied with reason code");
    } else {
        fail += 1;
        println!("[FAIL] system delete guard");
        print_hint(
            &rm_block,
            "expected non-zero exit and deny_system_critical receipt",
        );
        print_hint(
            &rm_tail,
            "tail receipts should include deny_system_critical",
        );
    }

    let dropper =
        std::env::temp_dir().join(format!("ar-smoke-dropper-{}.sh", uuid::Uuid::new_v4()));
    fs::write(&dropper, "#!/usr/bin/env bash\necho should-not-run\n")
        .context("write smoke dropper")?;
    let chmod = Command::new("chmod")
        .arg("+x")
        .arg(&dropper)
        .output()
        .context("chmod dropper")?;
    if !chmod.status.success() {
        return Err(anyhow!("chmod failed for smoke dropper"));
    }

    let exec_block = run_self(
        ruler_root,
        runtime_dir,
        &["run", "--", dropper.to_string_lossy().as_ref()],
    )?;
    let exec_tail = run_self(ruler_root, runtime_dir, &["tail", "25"])?;
    if !exec_block.status.success() && exec_tail.stdout.contains("deny_execution_from_temp") {
        pass += 1;
        println!("[PASS] download/temp execution blocked");
    } else {
        fail += 1;
        println!("[FAIL] execution guard");
        print_hint(
            &exec_block,
            "expected non-zero exit and deny_execution_from_temp receipt",
        );
    }

    println!("[INFO] non-interactive summary: pass={pass} fail={fail} skip={skip}");

    if !non_interactive {
        println!("[STEP] interactive checks (placed last):");
        println!("1) ./target/release/agent-ruler export report.txt report.txt");
        println!("2) ./target/release/agent-ruler approve --decision list");
        println!("3) ./target/release/agent-ruler approve --decision approve --id <pending-id>");
        println!("4) ./target/release/agent-ruler deliver report.txt");
    } else {
        println!("[INFO] interactive checks skipped (--non-interactive)");
    }

    if fail > 0 {
        return Err(anyhow!("smoke checks reported {fail} failures"));
    }

    Ok(())
}

struct SelfRunOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

fn run_self(ruler_root: &Path, runtime_dir: Option<&Path>, args: &[&str]) -> Result<SelfRunOutput> {
    let exe = std::env::current_exe().context("resolve current executable")?;
    let mut cmd = Command::new(exe);
    cmd.current_dir(ruler_root);
    if let Some(runtime_dir) = runtime_dir {
        cmd.arg("--runtime-dir").arg(runtime_dir);
    }
    cmd.args(args);

    let output = cmd.output().context("run smoke subprocess")?;
    Ok(SelfRunOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn print_hint(out: &SelfRunOutput, hint: &str) {
    println!("  hint: {hint}");
    if !out.stderr.trim().is_empty() {
        println!("  stderr: {}", out.stderr.trim());
    }
}

fn is_confinement_env_error(stderr: &str) -> bool {
    stderr.contains("Operation not permitted")
        || stderr.contains("Failed RTM_NEWADDR")
        || stderr.contains("setting up uid map")
        || stderr.contains("uid map")
        || stderr.contains("bubblewrap")
        || stderr.contains("setns")
}
