//! OWASP Prompt Injection Scenario Tests
//!
//! These tests validate that Agent Ruler's deterministic controls properly defend
//! against OWASP-documented prompt injection attack scenarios.
//!
//! Reference: https://cheatsheetseries.owasp.org/cheatsheets/LLM_Prompt_Injection_Prevention_Cheat_Sheet.html

mod common;

use agent_ruler::config::{
    ApprovalConfig, ElevationRules, ExecutionRules, NetworkRules, PersistenceRules, Policy,
    RuleDisposition, RulesConfig, Safeguards, ZoneRuleMatrix, ZonesConfig,
};
use agent_ruler::model::{ActionKind, ActionRequest, ProcessContext, ReasonCode, Verdict, Zone};
use agent_ruler::policy::PolicyEngine;
use chrono::Utc;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn make_request(kind: ActionKind, path: &str, metadata: BTreeMap<String, String>) -> ActionRequest {
    ActionRequest {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        kind,
        operation: format!("{:?}", kind),
        path: Some(PathBuf::from(path)),
        secondary_path: None,
        host: None,
        metadata,
        process: ProcessContext {
            pid: 12345,
            ppid: Some(12344),
            command: "test-agent".to_string(),
            process_tree: vec![12345, 12344],
        },
    }
}

fn make_strict_policy() -> Policy {
    Policy {
        version: "1.0.0".to_string(),
        profile: "strict".to_string(),
        zones: ZonesConfig {
            workspace_paths: vec![],
            user_data_paths: vec![],
            shared_paths: vec!["/srv".to_string(), "/opt/shared".to_string()],
            system_critical_paths: vec![
                "/usr".to_string(),
                "/bin".to_string(),
                "/sbin".to_string(),
                "/etc".to_string(),
                "/lib".to_string(),
                "/lib64".to_string(),
                "/boot".to_string(),
                "/dev".to_string(),
                "/proc".to_string(),
                "/sys".to_string(),
            ],
            secrets_paths: vec![
                "/root/.ssh".to_string(),
                "/etc/shadow".to_string(),
                "/etc/ssh".to_string(),
                "*.pem".to_string(),
                "*.key".to_string(),
            ],
        },
        rules: RulesConfig {
            filesystem: ZoneRuleMatrix {
                workspace: RuleDisposition::Allow,
                user_data: RuleDisposition::Allow,
                shared: RuleDisposition::Approval,
                system_critical: RuleDisposition::Deny,
                secrets: RuleDisposition::Deny,
            },
            network: NetworkRules {
                default_deny: true,
                allowlist_hosts: vec!["api.example.com".to_string()],
                require_approval_for_post: true,
                denylist_hosts: vec![],
                invert_allowlist: false,
                invert_denylist: false,
            },
            execution: ExecutionRules {
                deny_workspace_exec: true,
                deny_tmp_exec: true,
                quarantine_on_download_exec_chain: true,
                allowed_exec_prefixes: vec!["/usr/bin".to_string()],
            },
            persistence: PersistenceRules {
                deny_autostart: true,
                approval_paths: vec![],
                deny_paths: vec![
                    "/etc/systemd".to_string(),
                    "/etc/cron".to_string(),
                    "/etc/init.d".to_string(),
                ],
            },
            elevation: ElevationRules::default(),
        },
        safeguards: Safeguards {
            mass_delete_threshold: 10,
        },
        approvals: ApprovalConfig { ttl_seconds: 3600 },
    }
}

fn make_engine(workspace: &Path) -> PolicyEngine {
    let policy = make_strict_policy();
    PolicyEngine::new(policy, workspace.to_path_buf())
}

// ============================================
// Scenario 1: Indirect Injection via Web Content
// ============================================

/// Test that downloaded content is treated as untrusted and cannot
/// directly cause high-risk actions like execution.
#[test]
fn test_indirect_injection_web_content() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    // Simulate a file that was downloaded from the web
    let mut metadata = BTreeMap::new();
    metadata.insert("downloaded".to_string(), "true".to_string());
    metadata.insert(
        "download_source".to_string(),
        "https://malicious.example.com/payload.sh".to_string(),
    );

    // Attempt to execute the downloaded file
    let request = make_request(
        ActionKind::Execute,
        workspace.path().join("payload.sh").to_str().unwrap(),
        metadata,
    );

    let (decision, _zone) = engine.evaluate(&request);

    // Should be quarantined or denied
    assert!(
        matches!(decision.verdict, Verdict::Quarantine | Verdict::Deny),
        "Download→exec should be blocked: {:?}",
        decision
    );

    // Reason should indicate download-exec chain
    assert!(
        matches!(
            decision.reason,
            ReasonCode::QuarantineDownloadExecChain
                | ReasonCode::DenyExecutionFromWorkspace
                | ReasonCode::DenyExecutionDownloaded
        ),
        "Wrong reason code: {:?}",
        decision.reason
    );
}

// ============================================
// Scenario 2: Indirect Injection via Documents
// ============================================

/// Test that document content cannot bypass policy - all file writes
/// are subject to zone-based controls regardless of content source.
#[test]
fn test_indirect_injection_document() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    // Simulate processing a document that contains "instructions" to write to system
    let request = make_request(
        ActionKind::FileWrite,
        "/etc/cron.d/malicious",
        BTreeMap::new(),
    );

    let (decision, zone) = engine.evaluate(&request);

    // System paths should be denied
    assert!(
        matches!(zone, Some(Zone::SystemCritical) | Some(Zone::Secrets)),
        "System path should be classified as critical: {:?}",
        zone
    );

    assert!(
        matches!(decision.verdict, Verdict::Deny),
        "System write should be denied: {:?}",
        decision
    );

    assert!(
        matches!(decision.reason, ReasonCode::DenySystemCritical),
        "Wrong reason code: {:?}",
        decision.reason
    );
}

// ============================================
// Scenario 3: Data Exfiltration via URL Query Parameters
// ============================================

/// Test that network egress is controlled and suspicious patterns
/// are detected.
#[test]
fn test_data_exfiltration_url_params() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    // Attempt to exfiltrate data via URL
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "url".to_string(),
        "https://attacker.example.com/collect?data=SENSITIVE".to_string(),
    );
    metadata.insert("method".to_string(), "POST".to_string());

    let mut request = make_request(
        ActionKind::NetworkEgress,
        workspace.path().to_str().unwrap(),
        metadata,
    );
    request.host = Some("attacker.example.com".to_string());

    let (decision, _zone) = engine.evaluate(&request);

    // Should be denied because domain is not allowlisted
    assert!(
        matches!(decision.verdict, Verdict::Deny),
        "Non-allowlisted domain should be denied: {:?}",
        decision
    );

    assert!(
        matches!(
            decision.reason,
            ReasonCode::DenyNetworkDefault | ReasonCode::DenyNetworkNotAllowlisted
        ),
        "Wrong reason code: {:?}",
        decision.reason
    );
}

/// Test that upload-style network requests require approval
#[test]
fn test_data_exfiltration_upload_approval() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    // Attempt to upload data to an allowlisted domain
    let mut metadata = BTreeMap::new();
    metadata.insert("method".to_string(), "POST".to_string());
    metadata.insert("body_size".to_string(), "1024".to_string());
    metadata.insert("upload_pattern".to_string(), "true".to_string());

    let mut request = make_request(
        ActionKind::NetworkEgress,
        workspace.path().to_str().unwrap(),
        metadata,
    );
    request.host = Some("api.example.com".to_string());

    let (decision, _zone) = engine.evaluate(&request);

    // Upload to allowlisted domain should require approval
    assert!(
        matches!(decision.verdict, Verdict::RequireApproval),
        "Upload should require approval: {:?}",
        decision
    );

    assert!(
        matches!(decision.reason, ReasonCode::ApprovalRequiredNetworkUpload),
        "Wrong reason code: {:?}",
        decision.reason
    );
}

// ============================================
// Scenario 4: Download → Exec Chain
// ============================================

/// Test that the download→exec chain is properly quarantined
#[test]
fn test_download_exec_chain() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    // Simulate a downloaded executable
    let mut metadata = BTreeMap::new();
    metadata.insert("downloaded".to_string(), "true".to_string());

    let request = make_request(
        ActionKind::Execute,
        workspace
            .path()
            .join("downloaded_malware")
            .to_str()
            .unwrap(),
        metadata,
    );

    let (decision, _zone) = engine.evaluate(&request);

    assert!(
        matches!(decision.verdict, Verdict::Quarantine | Verdict::Deny),
        "Download→exec should be quarantined: {:?}",
        decision
    );
}

// ============================================
// Scenario 5: Tool Misuse / Capability Abuse
// ============================================

/// Test that mass delete operations require approval
#[test]
fn test_tool_misuse_mass_delete() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    // Attempt mass delete
    let mut metadata = BTreeMap::new();
    metadata.insert("delete_count".to_string(), "50".to_string());

    let request = make_request(
        ActionKind::FileDelete,
        workspace.path().to_str().unwrap(),
        metadata,
    );

    let (decision, _zone) = engine.evaluate(&request);

    assert!(
        matches!(decision.verdict, Verdict::RequireApproval),
        "Mass delete should require approval: {:?}",
        decision
    );

    assert!(
        matches!(decision.reason, ReasonCode::ApprovalRequiredMassDelete),
        "Wrong reason code: {:?}",
        decision.reason
    );
}

/// Test that system file access is denied regardless of operation type
#[test]
fn test_tool_misuse_system_access() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    // Attempt to read secrets
    let request = make_request(ActionKind::SecretsRead, "/etc/shadow", BTreeMap::new());

    let (decision, _zone) = engine.evaluate(&request);

    assert!(
        matches!(decision.verdict, Verdict::Deny),
        "Secrets access should be denied: {:?}",
        decision
    );

    assert!(
        matches!(decision.reason, ReasonCode::DenySecrets),
        "Wrong reason code: {:?}",
        decision.reason
    );
}

// ============================================
// Scenario 6: Instruction Hierarchy Violation
// ============================================

/// Test that policy enforcement is deterministic and cannot be bypassed
/// by agent "instructions" - the zone classification is based on paths,
/// not on claimed intent.
#[test]
fn test_instruction_hierarchy_enforcement() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    // Even if metadata claims "authorized" or "safe", system paths should still be denied
    let mut metadata = BTreeMap::new();
    metadata.insert("intent".to_string(), "legitimate system update".to_string());
    metadata.insert("authorized".to_string(), "true".to_string());
    metadata.insert("user_requested".to_string(), "true".to_string());

    let request = make_request(ActionKind::FileWrite, "/usr/bin/malicious", metadata);

    let (decision, zone) = engine.evaluate(&request);

    // Zone should be system critical regardless of metadata claims
    assert!(
        matches!(zone, Some(Zone::SystemCritical)),
        "Zone should be system critical: {:?}",
        zone
    );

    // Should be denied regardless of "intent" metadata
    assert!(
        matches!(decision.verdict, Verdict::Deny),
        "System write should be denied regardless of metadata: {:?}",
        decision
    );
}

// ============================================
// Scenario 7: Multi-Step Chained Attack
// ============================================

/// Test that fetch→write→exec→persist chain is broken at multiple points
#[test]
fn test_multistep_chain_fetch_write() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    // Step 1: Fetch (download) - creates a file with download marker
    let mut download_metadata = BTreeMap::new();
    download_metadata.insert("downloaded".to_string(), "true".to_string());

    // Step 2: Write to persistence location
    let persist_request = make_request(
        ActionKind::Persistence,
        "/etc/systemd/system/malicious.service",
        BTreeMap::new(),
    );

    let (decision, _zone) = engine.evaluate(&persist_request);

    // Persistence should be denied
    assert!(
        matches!(decision.verdict, Verdict::Deny),
        "Persistence should be denied: {:?}",
        decision
    );

    assert!(
        matches!(
            decision.reason,
            ReasonCode::DenyPersistence | ReasonCode::DenySystemCritical
        ),
        "Wrong reason code: {:?}",
        decision.reason
    );
}

#[test]
fn test_multistep_chain_exec_persist() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    // Attempt to execute a file from workspace
    let request = make_request(
        ActionKind::Execute,
        workspace.path().join("script.sh").to_str().unwrap(),
        BTreeMap::new(),
    );

    let (decision, _zone) = engine.evaluate(&request);

    // Workspace execution should be denied
    assert!(
        matches!(decision.verdict, Verdict::Deny),
        "Workspace execution should be denied: {:?}",
        decision
    );

    assert!(
        matches!(decision.reason, ReasonCode::DenyExecutionFromWorkspace),
        "Wrong reason code: {:?}",
        decision.reason
    );
}

// ============================================
// Scenario 8: Interpreter Execution Attack
// ============================================

/// Test that interpreter-based execution of downloaded scripts is blocked
#[test]
fn test_interpreter_exec_downloaded() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    // Simulate python executing a downloaded script
    let mut metadata = BTreeMap::new();
    metadata.insert("interpreter".to_string(), "true".to_string());
    metadata.insert("downloaded".to_string(), "true".to_string());
    metadata.insert(
        "script_path".to_string(),
        workspace
            .path()
            .join("malware.py")
            .to_str()
            .unwrap()
            .to_string(),
    );

    let request = make_request(ActionKind::Execute, "/usr/bin/python", metadata);

    let (decision, _zone) = engine.evaluate(&request);

    // Should be quarantined
    assert!(
        matches!(decision.verdict, Verdict::Quarantine | Verdict::Deny),
        "Interpreter exec of downloaded script should be blocked: {:?}",
        decision
    );
}

/// Test that stream-style execution (curl | bash) is blocked
#[test]
fn test_interpreter_stream_exec() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    // Simulate bash receiving input from a pipe (stream execution)
    let mut metadata = BTreeMap::new();
    metadata.insert("stream_exec".to_string(), "true".to_string());
    metadata.insert("pipe_input".to_string(), "true".to_string());

    let request = make_request(ActionKind::Execute, "/bin/bash", metadata);

    let (decision, _zone) = engine.evaluate(&request);

    // Should be denied
    assert!(
        matches!(decision.verdict, Verdict::Deny),
        "Stream execution should be denied: {:?}",
        decision
    );

    assert!(
        matches!(decision.reason, ReasonCode::DenyInterpreterStreamExec),
        "Wrong reason code: {:?}",
        decision.reason
    );
}

// ============================================
// Zone Classification Tests
// ============================================

#[test]
fn test_zone_classification_workspace() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    let request = make_request(
        ActionKind::FileWrite,
        workspace.path().join("output.txt").to_str().unwrap(),
        BTreeMap::new(),
    );

    let (decision, zone) = engine.evaluate(&request);

    assert!(matches!(zone, Some(Zone::Workspace)));
    assert!(matches!(decision.verdict, Verdict::Allow));
}

#[test]
fn test_zone_classification_system_critical() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    let request = make_request(ActionKind::FileWrite, "/usr/bin/test", BTreeMap::new());

    let (decision, zone) = engine.evaluate(&request);

    assert!(matches!(zone, Some(Zone::SystemCritical)));
    assert!(matches!(decision.verdict, Verdict::Deny));
}

#[test]
fn test_zone_classification_secrets() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    // Zone classification should identify secrets paths
    let zone = engine.classify_zone(&PathBuf::from("/etc/shadow"), ActionKind::SecretsRead);

    assert!(matches!(zone, Zone::Secrets));
}

// ============================================
// Network Policy Tests
// ============================================

#[test]
fn test_network_default_deny() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    let mut request = make_request(
        ActionKind::NetworkEgress,
        workspace.path().to_str().unwrap(),
        BTreeMap::new(),
    );
    request.host = Some("random.example.com".to_string());

    let (decision, _zone) = engine.evaluate(&request);

    assert!(matches!(decision.verdict, Verdict::Deny));
    assert!(matches!(
        decision.reason,
        ReasonCode::DenyNetworkDefault | ReasonCode::DenyNetworkNotAllowlisted
    ));
}

// ============================================
// Receipt and Audit Tests
// ============================================

#[test]
fn test_decision_has_reason_code() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    let request = make_request(ActionKind::FileWrite, "/etc/passwd", BTreeMap::new());

    let (decision, _zone) = engine.evaluate(&request);

    // Every decision must have a reason code
    let _reason = decision.reason;

    // Every decision must have a detail string
    assert!(!decision.detail.is_empty());
}

// ============================================
// Capability Separation Tests
// ============================================

/// Test that untrusted content markers cannot be forged
#[test]
fn test_content_markers_cannot_be_forged() {
    let workspace = TempDir::new().unwrap();
    let engine = make_engine(workspace.path());

    // Even if someone tries to clear the "downloaded" marker,
    // execution from workspace should still be denied
    let mut metadata = BTreeMap::new();
    metadata.insert("downloaded".to_string(), "false".to_string());
    metadata.insert("trusted".to_string(), "true".to_string());

    let request = make_request(
        ActionKind::Execute,
        workspace.path().join("script.sh").to_str().unwrap(),
        metadata,
    );

    let (decision, _zone) = engine.evaluate(&request);

    // Should still be denied because workspace execution is always blocked
    assert!(
        matches!(decision.verdict, Verdict::Deny),
        "Workspace execution should be denied regardless of trust markers: {:?}",
        decision
    );
}
