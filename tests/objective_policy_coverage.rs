mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;

use agent_ruler::config::Policy;
use agent_ruler::model::{ActionKind, ActionRequest, ProcessContext, ReasonCode, Verdict};
use agent_ruler::policy::PolicyEngine;
use chrono::Utc;

use common::TestRuntimeDir;

fn base_request(kind: ActionKind) -> ActionRequest {
    ActionRequest {
        id: "objective-request".to_string(),
        timestamp: Utc::now(),
        kind,
        operation: "objective-test".to_string(),
        path: None,
        secondary_path: None,
        host: None,
        metadata: BTreeMap::new(),
        process: ProcessContext {
            pid: 321,
            ppid: Some(1),
            command: "test".to_string(),
            process_tree: vec![321],
        },
    }
}

fn default_engine(workspace: PathBuf) -> PolicyEngine {
    let policy: Policy =
        serde_yaml::from_str(include_str!("../assets/default-policy.yaml")).expect("policy parse");
    PolicyEngine::new(policy.expanded(&workspace), workspace)
}

#[test]
fn network_egress_denied_by_default() {
    let temp = TestRuntimeDir::new("network-default-deny");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let engine = default_engine(workspace);

    let mut req = base_request(ActionKind::NetworkEgress);
    req.host = Some("example.com".to_string());

    let (decision, zone) = engine.evaluate(&req);
    assert!(zone.is_none());
    assert_eq!(decision.verdict, Verdict::Deny);
    assert_eq!(decision.reason, ReasonCode::DenyNetworkNotAllowlisted);
}

#[test]
fn network_allowlist_allows_explicit_host() {
    let temp = TestRuntimeDir::new("network-allowlist");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let mut policy: Policy =
        serde_yaml::from_str(include_str!("../assets/default-policy.yaml")).expect("policy parse");
    policy.rules.network.allowlist_hosts = vec!["api.example.org".to_string()];
    let engine = PolicyEngine::new(policy.expanded(&workspace), workspace);

    let mut req = base_request(ActionKind::NetworkEgress);
    req.host = Some("api.example.org".to_string());

    let (decision, _) = engine.evaluate(&req);
    assert_eq!(decision.verdict, Verdict::Allow);
    assert_eq!(decision.reason, ReasonCode::AllowedByPolicy);
}

#[test]
fn secrets_path_is_denied_for_filesystem_write() {
    let temp = TestRuntimeDir::new("secrets-path-deny");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let engine = default_engine(workspace.clone());

    let mut req = base_request(ActionKind::FileWrite);
    req.path = Some(workspace.join(".env"));

    let (decision, _) = engine.evaluate(&req);
    assert_eq!(decision.verdict, Verdict::Deny);
    assert_eq!(decision.reason, ReasonCode::DenySecrets);
}

#[test]
fn persistence_user_local_autostart_is_allowed_by_default() {
    let temp = TestRuntimeDir::new("persistence-user-allow");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let engine = default_engine(workspace.clone());

    let mut req = base_request(ActionKind::Persistence);
    req.path = Some(workspace.join(".config/autostart/agent-ruler.desktop"));

    let (decision, _) = engine.evaluate(&req);
    assert_eq!(decision.verdict, Verdict::Allow);
    assert_eq!(decision.reason, ReasonCode::AllowedByPolicy);
}

#[test]
fn persistence_system_scope_requires_approval() {
    let temp = TestRuntimeDir::new("persistence-system-approval");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let engine = default_engine(workspace);

    let mut req = base_request(ActionKind::Persistence);
    req.path = Some(PathBuf::from("/etc/systemd/system/agent-ruler.service"));

    let (decision, _) = engine.evaluate(&req);
    assert_eq!(decision.verdict, Verdict::RequireApproval);
    assert_eq!(decision.reason, ReasonCode::ApprovalRequiredPersistence);
}

#[test]
fn persistence_suspicious_chain_is_quarantined() {
    let temp = TestRuntimeDir::new("persistence-suspicious-chain");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let engine = default_engine(workspace);

    let mut req = base_request(ActionKind::Persistence);
    req.path = Some(PathBuf::from("/etc/systemd/system/agent-ruler.service"));
    req.metadata
        .insert("suspicious_chain".to_string(), "true".to_string());

    let (decision, _) = engine.evaluate(&req);
    assert_eq!(decision.verdict, Verdict::Quarantine);
    assert_eq!(decision.reason, ReasonCode::QuarantineHighRiskPattern);
}

#[test]
fn mass_delete_guard_requires_approval() {
    let temp = TestRuntimeDir::new("mass-delete-approval");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let engine = default_engine(workspace.clone());

    let mut req = base_request(ActionKind::FileDelete);
    req.path = Some(workspace.join("project"));
    req.metadata
        .insert("delete_count".to_string(), "40".to_string());

    let (decision, _) = engine.evaluate(&req);
    assert_eq!(decision.verdict, Verdict::RequireApproval);
    assert_eq!(decision.reason, ReasonCode::ApprovalRequiredMassDelete);
}

#[test]
fn tmp_exec_without_download_metadata_is_denied() {
    let temp = TestRuntimeDir::new("tmp-exec-deny");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let engine = default_engine(workspace);

    let mut req = base_request(ActionKind::Execute);
    req.path = Some(PathBuf::from("/tmp/transient.sh"));

    let (decision, _) = engine.evaluate(&req);
    assert_eq!(decision.verdict, Verdict::Deny);
    assert_eq!(decision.reason, ReasonCode::DenyExecutionFromTemp);
}

#[test]
fn network_enabled_with_allowlist_denies_unknown_host() {
    let temp = TestRuntimeDir::new("network-enabled-allowlist-deny");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let mut policy: Policy =
        serde_yaml::from_str(include_str!("../assets/default-policy.yaml")).expect("policy parse");
    policy.rules.network.default_deny = false;
    policy.rules.network.allowlist_hosts = vec!["api.example.org".to_string()];

    let engine = PolicyEngine::new(policy.expanded(&workspace), workspace);

    let mut req = base_request(ActionKind::NetworkEgress);
    req.host = Some("evil.example.net".to_string());

    let (decision, _) = engine.evaluate(&req);
    assert_eq!(decision.verdict, Verdict::Deny);
    assert_eq!(decision.reason, ReasonCode::DenyNetworkNotAllowlisted);
}

#[test]
fn network_enabled_with_denylist_blocks_matching_host() {
    let temp = TestRuntimeDir::new("network-enabled-denylist");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let mut policy: Policy =
        serde_yaml::from_str(include_str!("../assets/default-policy.yaml")).expect("policy parse");
    policy.rules.network.default_deny = false;
    policy.rules.network.denylist_hosts = vec!["blocked.example.org".to_string()];

    let engine = PolicyEngine::new(policy.expanded(&workspace), workspace);

    let mut req = base_request(ActionKind::NetworkEgress);
    req.host = Some("blocked.example.org".to_string());

    let (decision, _) = engine.evaluate(&req);
    assert_eq!(decision.verdict, Verdict::Deny);
    assert_eq!(decision.reason, ReasonCode::DenyNetworkNotAllowlisted);
}

#[test]
fn network_invert_allowlist_acts_as_denyset() {
    let temp = TestRuntimeDir::new("network-invert-allowlist");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let mut policy: Policy =
        serde_yaml::from_str(include_str!("../assets/default-policy.yaml")).expect("policy parse");
    policy.rules.network.default_deny = false;
    policy.rules.network.allowlist_hosts = vec!["blocked.example.org".to_string()];
    policy.rules.network.invert_allowlist = true;

    let engine = PolicyEngine::new(policy.expanded(&workspace), workspace);

    let mut req = base_request(ActionKind::NetworkEgress);
    req.host = Some("blocked.example.org".to_string());

    let (decision, _) = engine.evaluate(&req);
    assert_eq!(decision.verdict, Verdict::Deny);
    assert_eq!(decision.reason, ReasonCode::DenyNetworkNotAllowlisted);
}

#[test]
fn network_invert_denylist_acts_as_allowset_even_with_default_deny() {
    let temp = TestRuntimeDir::new("network-invert-denylist");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let mut policy: Policy =
        serde_yaml::from_str(include_str!("../assets/default-policy.yaml")).expect("policy parse");
    policy.rules.network.default_deny = true;
    policy.rules.network.allowlist_hosts.clear();
    policy.rules.network.denylist_hosts = vec!["api.example.org".to_string()];
    policy.rules.network.invert_denylist = true;

    let engine = PolicyEngine::new(policy.expanded(&workspace), workspace);

    let mut req = base_request(ActionKind::NetworkEgress);
    req.host = Some("api.example.org".to_string());

    let (decision, _) = engine.evaluate(&req);
    assert_eq!(decision.verdict, Verdict::Allow);
    assert_eq!(decision.reason, ReasonCode::AllowedByPolicy);
}

#[test]
fn upload_style_network_requires_approval() {
    let temp = TestRuntimeDir::new("network-upload-approval");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let mut policy: Policy =
        serde_yaml::from_str(include_str!("../assets/default-policy.yaml")).expect("policy parse");
    policy.rules.network.default_deny = false;
    policy.rules.network.allowlist_hosts = vec!["api.example.org".to_string()];

    let engine = PolicyEngine::new(policy.expanded(&workspace), workspace);

    let mut req = base_request(ActionKind::NetworkEgress);
    req.host = Some("api.example.org".to_string());
    req.metadata
        .insert("upload_pattern".to_string(), "true".to_string());

    let (decision, _) = engine.evaluate(&req);
    assert_eq!(decision.verdict, Verdict::RequireApproval);
    assert_eq!(decision.reason, ReasonCode::ApprovalRequiredNetworkUpload);
}
