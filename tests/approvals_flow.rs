mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use agent_ruler::approvals::ApprovalStore;
use agent_ruler::model::{
    ActionKind, ActionRequest, Decision, ProcessContext, ReasonCode, Verdict,
};
use chrono::Utc;

use common::TestRuntimeDir;

#[test]
fn approvals_grant_scope_access() {
    let temp = TestRuntimeDir::new("approvals-scope");
    let file = temp.path().join("approvals.json");
    fs::write(&file, "[]").expect("seed approvals file");

    let store = ApprovalStore::new(&file);
    let action = ActionRequest {
        id: "approval-test".to_string(),
        timestamp: Utc::now(),
        kind: ActionKind::ExportCommit,
        operation: "export".to_string(),
        path: Some(PathBuf::from("/opt/demo")),
        secondary_path: Some(PathBuf::from("/tmp/work/report.txt")),
        host: None,
        metadata: BTreeMap::new(),
        process: ProcessContext {
            pid: 99,
            ppid: Some(1),
            command: "test".to_string(),
            process_tree: vec![99],
        },
    };

    let decision = Decision {
        verdict: Verdict::RequireApproval,
        reason: ReasonCode::ApprovalRequiredExport,
        detail: "need approval".to_string(),
        approval_ttl_seconds: Some(3600),
    };

    let pending = store
        .create_pending(&action, &decision, "integration test")
        .expect("create pending");
    assert_eq!(pending.status, agent_ruler::model::ApprovalStatus::Pending);
    assert!(!store
        .has_active_approval_for(&action)
        .expect("has approval before approve"));

    store.approve(&pending.id).expect("approve");
    assert!(store
        .has_active_approval_for(&action)
        .expect("has approval after approve"));
}

#[test]
fn create_pending_reuses_existing_pending_for_same_scope() {
    let temp = TestRuntimeDir::new("approvals-dedupe");
    let file = temp.path().join("approvals.json");
    fs::write(&file, "[]").expect("seed approvals file");

    let store = ApprovalStore::new(&file);
    let action = ActionRequest {
        id: "approval-dedupe".to_string(),
        timestamp: Utc::now(),
        kind: ActionKind::ExportCommit,
        operation: "export_commit".to_string(),
        path: Some(PathBuf::from("/tmp/shared/report.txt")),
        secondary_path: Some(PathBuf::from("/tmp/workspace/report.txt")),
        host: None,
        metadata: BTreeMap::from([
            ("stage_id".to_string(), "first".to_string()),
            (
                "export_src".to_string(),
                "/tmp/workspace/report.txt".to_string(),
            ),
            (
                "export_dst".to_string(),
                "/tmp/shared/report.txt".to_string(),
            ),
        ]),
        process: ProcessContext {
            pid: 99,
            ppid: Some(1),
            command: "test".to_string(),
            process_tree: vec![99],
        },
    };
    let mut action_retry = action.clone();
    action_retry
        .metadata
        .insert("stage_id".to_string(), "second".to_string());

    let decision = Decision {
        verdict: Verdict::RequireApproval,
        reason: ReasonCode::ApprovalRequiredExport,
        detail: "need approval".to_string(),
        approval_ttl_seconds: Some(3600),
    };

    let first = store
        .create_pending(&action, &decision, "first request")
        .expect("create first pending");
    let second = store
        .create_pending(&action_retry, &decision, "retry request")
        .expect("create second pending");

    assert_eq!(
        first.id, second.id,
        "retry with same operation/path should reuse pending approval"
    );

    let all = store.list_all().expect("list all approvals");
    assert_eq!(all.len(), 1, "dedupe should keep one pending record");
}
