mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use agent_ruler::approvals::ApprovalStore;
use agent_ruler::model::{
    ActionKind, ActionRequest, ApprovalStatus, Decision, ProcessContext, ReasonCode, Verdict,
};
use chrono::Utc;

use common::TestRuntimeDir;

fn sample_action() -> ActionRequest {
    ActionRequest {
        id: "approval-expiry".to_string(),
        timestamp: Utc::now(),
        kind: ActionKind::ExportCommit,
        operation: "export".to_string(),
        path: Some(PathBuf::from("/opt/out.txt")),
        secondary_path: Some(PathBuf::from("/tmp/workspace/out.txt")),
        host: None,
        metadata: BTreeMap::new(),
        process: ProcessContext {
            pid: 55,
            ppid: Some(1),
            command: "test".to_string(),
            process_tree: vec![55],
        },
    }
}

#[test]
fn pending_approval_expires_when_ttl_is_elapsed() {
    let temp = TestRuntimeDir::new("approvals-expiry");
    let file = temp.path().join("approvals.json");
    fs::write(&file, "[]\n").expect("seed approvals file");
    let store = ApprovalStore::new(&file);

    let decision = Decision {
        verdict: Verdict::RequireApproval,
        reason: ReasonCode::ApprovalRequiredExport,
        detail: "approval needed".to_string(),
        approval_ttl_seconds: Some(0),
    };

    let created = store
        .create_pending(&sample_action(), &decision, "expiry test")
        .expect("create pending approval");
    assert_eq!(created.status, ApprovalStatus::Pending);

    let _expired_count = store.expire_now().expect("expire approvals");
    let refreshed = store
        .get(&created.id)
        .expect("load approval")
        .expect("approval should exist");

    assert_eq!(refreshed.status, ApprovalStatus::Expired);
    assert!(refreshed.decided_at.is_some());
}
