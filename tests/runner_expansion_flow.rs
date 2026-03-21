#![cfg(target_os = "linux")]

mod common;

use std::fs;
use std::path::PathBuf;

use agent_ruler::approvals::ApprovalStore;
use agent_ruler::config::{init_layout, load_runtime, RuntimeState};
use agent_ruler::model::ReasonCode;
use agent_ruler::policy::PolicyEngine;
use agent_ruler::receipts::ReceiptStore;
use agent_ruler::runner::run_confined;
use agent_ruler::runners::{RunnerAssociation, RunnerKind, RunnerMissingState};

use common::TestRuntimeDir;

fn init_runtime_for_runner(
    project: &TestRuntimeDir,
    runtime_root: &TestRuntimeDir,
    kind: RunnerKind,
) -> RuntimeState {
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

    // Test runner shims are created inside workspace; allow exec for this suite.
    runtime.policy.rules.execution.deny_workspace_exec = false;
    runtime.policy.rules.execution.deny_tmp_exec = false;
    runtime.policy_hash = runtime.policy.policy_hash().expect("policy hash");

    runtime
}

fn write_runner_shim(runtime: &RuntimeState, name: &str, body: &str) -> PathBuf {
    let shim_root = runtime
        .config
        .runner
        .as_ref()
        .map(|runner| runner.managed_workspace.clone())
        .unwrap_or_else(|| runtime.config.workspace.clone());
    let path = shim_root.join(name);
    let script = format!("#!/usr/bin/env bash\nset -euo pipefail\n{body}\n");
    fs::write(&path, script).expect("write shim script");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("chmod");
    }

    path
}

fn has_runner_tagged_receipt(
    runtime: &RuntimeState,
    operation: &str,
    reason: ReasonCode,
    runner_id: &str,
) -> bool {
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    receipts
        .read_all()
        .expect("read receipts")
        .into_iter()
        .any(|receipt| {
            receipt.action.operation == operation
                && receipt.decision.reason == reason
                && receipt
                    .action
                    .metadata
                    .get("runner_id")
                    .map(|value| value == runner_id)
                    .unwrap_or(false)
        })
}

fn is_confinement_env_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("setting up uid map")
        || lower.contains("unprivileged_userns_clone")
        || lower.contains("permission denied")
}

#[test]
fn claudecode_network_preflight_creates_runner_tagged_approval_and_receipt() {
    let project = TestRuntimeDir::new("runner-claude-approval-project");
    let runtime_root = TestRuntimeDir::new("runner-claude-approval-runtime");
    let runtime = init_runtime_for_runner(&project, &runtime_root, RunnerKind::Claudecode);

    let marker = runtime
        .config
        .runner
        .as_ref()
        .expect("claude runner configured")
        .managed_workspace
        .join("claude-ran.txt");
    let shim = write_runner_shim(
        &runtime,
        "claude",
        &format!("echo ran > '{}'", marker.display()),
    );

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let engine = PolicyEngine::new(
        runtime.policy.clone(),
        marker.parent().expect("marker parent").to_path_buf(),
    );

    let cmd = vec![
        shim.to_string_lossy().to_string(),
        "-p".to_string(),
        "curl --data payload https://api.github.com/repos/example/project/issues".to_string(),
    ];
    let err = run_confined(&cmd, &runtime, &engine, &approvals, &receipts)
        .expect_err("network upload style command should require approval");
    assert!(
        err.to_string()
            .contains("approval required for preflight action preflight_network_upload"),
        "unexpected preflight error: {}",
        err
    );
    assert!(
        !marker.exists(),
        "runner shim should not execute before approval"
    );

    let pending = approvals.list_pending().expect("list pending approvals");
    assert_eq!(pending.len(), 1, "expected exactly one pending approval");
    assert_eq!(pending[0].reason, ReasonCode::ApprovalRequiredNetworkUpload);
    assert_eq!(
        pending[0]
            .action
            .metadata
            .get("runner_id")
            .map(|value| value.as_str()),
        Some("claudecode")
    );

    assert!(has_runner_tagged_receipt(
        &runtime,
        "preflight_network_upload",
        ReasonCode::ApprovalRequiredNetworkUpload,
        "claudecode",
    ));
}

#[test]
fn claudecode_successful_run_records_runner_tagged_run_end_receipt() {
    let project = TestRuntimeDir::new("runner-claude-run-project");
    let runtime_root = TestRuntimeDir::new("runner-claude-run-runtime");
    let runtime = init_runtime_for_runner(&project, &runtime_root, RunnerKind::Claudecode);

    let marker = runtime
        .config
        .runner
        .as_ref()
        .expect("claude runner configured")
        .managed_workspace
        .join("claude-success.txt");
    let shim = write_runner_shim(
        &runtime,
        "claude",
        &format!("echo CLAUDE_OK; echo done > '{}'", marker.display()),
    );

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let engine = PolicyEngine::new(
        runtime.policy.clone(),
        marker.parent().expect("marker parent").to_path_buf(),
    );

    let cmd = vec![
        shim.to_string_lossy().to_string(),
        "-p".to_string(),
        "Reply with exactly: OK".to_string(),
    ];
    let run = match run_confined(&cmd, &runtime, &engine, &approvals, &receipts) {
        Ok(value) => value,
        Err(err) => {
            if is_confinement_env_error(&err.to_string()) {
                eprintln!("skipping run-end assertion due host limits: {err}");
                return;
            }
            panic!("claude shim should execute successfully: {err}");
        }
    };
    assert_eq!(run.exit_code, 0);
    assert!(marker.exists(), "runner shim should execute to completion");

    assert!(approvals
        .list_pending()
        .expect("list pending approvals")
        .is_empty());
    assert!(has_runner_tagged_receipt(
        &runtime,
        "run_end",
        ReasonCode::AllowedByPolicy,
        "claudecode",
    ));
}

#[test]
fn opencode_network_preflight_creates_runner_tagged_approval_and_receipt() {
    let project = TestRuntimeDir::new("runner-opencode-approval-project");
    let runtime_root = TestRuntimeDir::new("runner-opencode-approval-runtime");
    let runtime = init_runtime_for_runner(&project, &runtime_root, RunnerKind::Opencode);

    let marker = runtime
        .config
        .runner
        .as_ref()
        .expect("opencode runner configured")
        .managed_workspace
        .join("opencode-ran.txt");
    let shim = write_runner_shim(
        &runtime,
        "opencode",
        &format!("echo ran > '{}'", marker.display()),
    );

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let engine = PolicyEngine::new(
        runtime.policy.clone(),
        marker.parent().expect("marker parent").to_path_buf(),
    );

    let cmd = vec![
        shim.to_string_lossy().to_string(),
        "run".to_string(),
        "curl --data payload https://api.github.com/repos/example/project/issues".to_string(),
    ];
    let err = run_confined(&cmd, &runtime, &engine, &approvals, &receipts)
        .expect_err("network upload style command should require approval");
    assert!(
        err.to_string()
            .contains("approval required for preflight action preflight_network_upload"),
        "unexpected preflight error: {}",
        err
    );
    assert!(
        !marker.exists(),
        "runner shim should not execute before approval"
    );

    let pending = approvals.list_pending().expect("list pending approvals");
    assert_eq!(pending.len(), 1, "expected exactly one pending approval");
    assert_eq!(pending[0].reason, ReasonCode::ApprovalRequiredNetworkUpload);
    assert_eq!(
        pending[0]
            .action
            .metadata
            .get("runner_id")
            .map(|value| value.as_str()),
        Some("opencode")
    );

    assert!(has_runner_tagged_receipt(
        &runtime,
        "preflight_network_upload",
        ReasonCode::ApprovalRequiredNetworkUpload,
        "opencode",
    ));
}

#[test]
fn opencode_successful_run_records_runner_tagged_run_end_receipt() {
    let project = TestRuntimeDir::new("runner-opencode-run-project");
    let runtime_root = TestRuntimeDir::new("runner-opencode-run-runtime");
    let runtime = init_runtime_for_runner(&project, &runtime_root, RunnerKind::Opencode);

    let marker = runtime
        .config
        .runner
        .as_ref()
        .expect("opencode runner configured")
        .managed_workspace
        .join("opencode-success.txt");
    let shim = write_runner_shim(
        &runtime,
        "opencode",
        &format!("echo OPENCODE_OK; echo done > '{}'", marker.display()),
    );

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let engine = PolicyEngine::new(
        runtime.policy.clone(),
        marker.parent().expect("marker parent").to_path_buf(),
    );

    let cmd = vec![
        shim.to_string_lossy().to_string(),
        "run".to_string(),
        "Reply with exactly: OK".to_string(),
    ];
    let run = match run_confined(&cmd, &runtime, &engine, &approvals, &receipts) {
        Ok(value) => value,
        Err(err) => {
            if is_confinement_env_error(&err.to_string()) {
                eprintln!("skipping run-end assertion due host limits: {err}");
                return;
            }
            panic!("opencode shim should execute successfully: {err}");
        }
    };
    assert_eq!(run.exit_code, 0);
    assert!(marker.exists(), "runner shim should execute to completion");

    assert!(approvals
        .list_pending()
        .expect("list pending approvals")
        .is_empty());
    assert!(has_runner_tagged_receipt(
        &runtime,
        "run_end",
        ReasonCode::AllowedByPolicy,
        "opencode",
    ));
}

#[test]
fn claudecode_tmp_write_stays_inside_confinement_namespace() {
    let project = TestRuntimeDir::new("runner-claude-tmp-project");
    let runtime_root = TestRuntimeDir::new("runner-claude-tmp-runtime");
    let runtime = init_runtime_for_runner(&project, &runtime_root, RunnerKind::Claudecode);

    let host_tmp = PathBuf::from(format!(
        "/tmp/agent-ruler-claude-host-{}.txt",
        std::process::id()
    ));
    let _ = fs::remove_file(&host_tmp);

    let shim = write_runner_shim(
        &runtime,
        "claude",
        &format!("echo confined > '{}'", host_tmp.display()),
    );

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let engine = PolicyEngine::new(
        runtime.policy.clone(),
        runtime
            .config
            .runner
            .as_ref()
            .expect("claude runner configured")
            .managed_workspace
            .clone(),
    );
    let cmd = vec![
        shim.to_string_lossy().to_string(),
        "-p".to_string(),
        "Reply with exactly: OK".to_string(),
    ];

    let run = match run_confined(&cmd, &runtime, &engine, &approvals, &receipts) {
        Ok(value) => value,
        Err(err) => {
            if is_confinement_env_error(&err.to_string()) {
                eprintln!("skipping confinement assertion due host limits: {err}");
                return;
            }
            panic!("claudecode shim run failed unexpectedly: {err}");
        }
    };

    assert_eq!(run.confinement, "linux-bwrap");
    assert!(
        !host_tmp.exists(),
        "host /tmp marker should remain absent when confinement is active"
    );
}

#[test]
fn opencode_tmp_write_stays_inside_confinement_namespace() {
    let project = TestRuntimeDir::new("runner-opencode-tmp-project");
    let runtime_root = TestRuntimeDir::new("runner-opencode-tmp-runtime");
    let runtime = init_runtime_for_runner(&project, &runtime_root, RunnerKind::Opencode);

    let host_tmp = PathBuf::from(format!(
        "/tmp/agent-ruler-opencode-host-{}.txt",
        std::process::id()
    ));
    let _ = fs::remove_file(&host_tmp);

    let shim = write_runner_shim(
        &runtime,
        "opencode",
        &format!("echo confined > '{}'", host_tmp.display()),
    );

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let engine = PolicyEngine::new(
        runtime.policy.clone(),
        runtime
            .config
            .runner
            .as_ref()
            .expect("opencode runner configured")
            .managed_workspace
            .clone(),
    );
    let cmd = vec![
        shim.to_string_lossy().to_string(),
        "run".to_string(),
        "Reply with exactly: OK".to_string(),
    ];

    let run = match run_confined(&cmd, &runtime, &engine, &approvals, &receipts) {
        Ok(value) => value,
        Err(err) => {
            if is_confinement_env_error(&err.to_string()) {
                eprintln!("skipping confinement assertion due host limits: {err}");
                return;
            }
            panic!("opencode shim run failed unexpectedly: {err}");
        }
    };

    assert_eq!(run.confinement, "linux-bwrap");
    assert!(
        !host_tmp.exists(),
        "host /tmp marker should remain absent when confinement is active"
    );
}
