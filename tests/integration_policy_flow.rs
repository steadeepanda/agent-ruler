mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;

use agent_ruler::config::Policy;
use agent_ruler::model::{ActionKind, ActionRequest, ProcessContext, ReasonCode, Verdict};
use agent_ruler::policy::PolicyEngine;
use chrono::Utc;

use common::TestRuntimeDir;

fn test_request(kind: ActionKind, path: PathBuf) -> ActionRequest {
    ActionRequest {
        id: "integration-request".to_string(),
        timestamp: Utc::now(),
        kind,
        operation: "integration-test".to_string(),
        path: Some(path),
        secondary_path: None,
        host: None,
        metadata: BTreeMap::new(),
        process: ProcessContext {
            pid: 200,
            ppid: Some(1),
            command: "test".to_string(),
            process_tree: vec![200],
        },
    }
}

#[test]
fn workspace_writes_are_allowed() {
    let temp = TestRuntimeDir::new("workspace-writes");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let policy: Policy =
        serde_yaml::from_str(include_str!("../assets/default-policy.yaml")).expect("policy parse");
    let policy = policy.expanded(&workspace);
    let engine = PolicyEngine::new(policy, workspace.clone());

    let req = test_request(ActionKind::FileWrite, workspace.join("result.txt"));
    let (decision, _) = engine.evaluate(&req);

    assert_eq!(decision.verdict, Verdict::Allow);
    assert_eq!(decision.reason, ReasonCode::AllowedByPolicy);
}

#[test]
fn system_delete_is_denied_with_reason_code() {
    let temp = TestRuntimeDir::new("system-delete");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let policy: Policy =
        serde_yaml::from_str(include_str!("../assets/default-policy.yaml")).expect("policy parse");
    let policy = policy.expanded(&workspace);
    let engine = PolicyEngine::new(policy, workspace);

    let req = test_request(ActionKind::FileDelete, PathBuf::from("/etc/passwd"));
    let (decision, _) = engine.evaluate(&req);

    assert_eq!(decision.verdict, Verdict::Deny);
    assert_eq!(decision.reason, ReasonCode::DenySystemCritical);
}

#[test]
fn download_exec_chain_is_quarantined() {
    let temp = TestRuntimeDir::new("download-exec-quarantine");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let policy: Policy =
        serde_yaml::from_str(include_str!("../assets/default-policy.yaml")).expect("policy parse");
    let policy = policy.expanded(&workspace);
    let engine = PolicyEngine::new(policy, workspace);

    let mut req = test_request(ActionKind::Execute, PathBuf::from("/tmp/dropper.sh"));
    req.metadata
        .insert("downloaded".to_string(), "true".to_string());
    let (decision, _) = engine.evaluate(&req);

    assert_eq!(decision.verdict, Verdict::Quarantine);
    assert_eq!(decision.reason, ReasonCode::QuarantineDownloadExecChain);
}

#[test]
fn export_to_shared_zone_requires_approval() {
    let temp = TestRuntimeDir::new("export-shared-approval");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let policy: Policy =
        serde_yaml::from_str(include_str!("../assets/default-policy.yaml")).expect("policy parse");
    let policy = policy.expanded(&workspace);
    let engine = PolicyEngine::new(policy, workspace.clone());

    let mut req = test_request(ActionKind::ExportCommit, PathBuf::from("/opt/agent-output"));
    req.secondary_path = Some(workspace.join("report.txt"));
    let (decision, _) = engine.evaluate(&req);

    assert_eq!(decision.verdict, Verdict::RequireApproval);
    assert_eq!(decision.reason, ReasonCode::ApprovalRequiredExport);
}
