mod common;

use std::collections::BTreeMap;
use std::fs;

use agent_ruler::model::{
    ActionKind, ActionRequest, Decision, ProcessContext, ReasonCode, Receipt, Verdict,
};
use agent_ruler::receipts::ReceiptStore;
use chrono::Utc;

use common::TestRuntimeDir;

fn sample_receipt(id: &str) -> Receipt {
    Receipt {
        id: id.to_string(),
        timestamp: Utc::now(),
        action: ActionRequest {
            id: format!("action-{id}"),
            timestamp: Utc::now(),
            kind: ActionKind::FileWrite,
            operation: "write".to_string(),
            path: Some("/tmp/file.txt".into()),
            secondary_path: None,
            host: None,
            metadata: BTreeMap::new(),
            process: ProcessContext {
                pid: 77,
                ppid: Some(1),
                command: "test".to_string(),
                process_tree: vec![77],
            },
        },
        decision: Decision {
            verdict: Verdict::Allow,
            reason: ReasonCode::AllowedByPolicy,
            detail: "ok".to_string(),
            approval_ttl_seconds: None,
        },
        zone: Some(agent_ruler::model::Zone::Workspace),
        policy_version: "1".to_string(),
        policy_hash: "hash".to_string(),
        diff_summary: None,
        confinement: "test".to_string(),
    }
}

#[test]
fn receipt_store_appends_and_tails() {
    let temp = TestRuntimeDir::new("receipts-store");
    let file = temp.path().join("receipts.jsonl");
    fs::write(&file, "").expect("seed file");
    let store = ReceiptStore::new(&file);

    store.append(&sample_receipt("r1")).expect("append r1");
    store.append(&sample_receipt("r2")).expect("append r2");

    let tail = store.tail(1).expect("tail");
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].id, "r2");

    let all = store.read_all().expect("read all");
    assert_eq!(all.len(), 2);
}
