//! Policy evaluation engine for Agent Ruler.
//!
//! This module provides deterministic policy evaluation for all agent actions.
//! It classifies paths into security zones and applies rules based on the
//! configured policy profile.
//!
//! # Key Components
//!
//! - [`PolicyEngine`] - Main entry point for policy evaluation
//! - [`evaluation`] - Evaluation logic for different action types
//! - [`zone`] - Path-to-zone classification
//! - [`patterns`] - Compiled path patterns for matching
//!
//! # Security Invariants
//!
//! 1. **Determinism**: Given the same policy and request, the engine always
//!    returns the same decision. No random or time-based variation.
//! 2. **Default Deny**: Unknown paths or operations default to deny/approval.
//! 3. **System Critical Guard**: Paths matching system-critical patterns are
//!    always denied, regardless of profile settings.
//! 4. **No LLM Logic**: Policy evaluation uses only pattern matching and
//!    rule lookup, never AI/LLM inference.
//!
//! # Evaluation Flow
//!
//! ```text
//! ActionRequest -> classify_zone() -> lookup_rule() -> Decision
//!            \
//!             -> evaluate_network() / evaluate_filesystem() / etc.
//! ```
//!
//! # Tests
//!
//! See `/tests/integration_policy_flow.rs` and `/tests/owasp_scenarios.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::config::Policy;
use crate::model::{ActionKind, ActionRequest, Decision, ReasonCode, Verdict, Zone};

mod evaluation;
mod helpers;
mod patterns;
mod zone;

use patterns::CompiledPatterns;

/// The main policy evaluation engine.
///
/// This is the core component that evaluates action requests against the
/// configured policy. It maintains compiled patterns for efficient matching
/// and a zone cache for path classification.
///
/// # Thread Safety
///
/// The engine uses a `Mutex` for the zone cache, allowing concurrent reads
/// from multiple threads (e.g., WebUI and CLI).
///
/// # Example
///
/// ```ignore
/// let policy = Policy::from_yaml_file("policy.yaml")?;
/// let engine = PolicyEngine::new(policy, PathBuf::from("/workspace"));
/// let (decision, zone) = engine.evaluate(&request);
/// ```
#[derive(Debug)]
pub struct PolicyEngine {
    /// The loaded policy configuration
    policy: Policy,
    /// Workspace path for relative path resolution
    workspace: PathBuf,
    /// Pre-compiled patterns for efficient matching
    compiled: CompiledPatterns,
    /// Cache of path -> zone classifications
    zone_cache: Mutex<HashMap<PathBuf, Zone>>,
}

impl PolicyEngine {
    /// Create a new policy engine with the given policy and workspace.
    pub fn new(policy: Policy, workspace: PathBuf) -> Self {
        let compiled = CompiledPatterns::from_policy(&policy);
        Self {
            policy,
            workspace,
            compiled,
            zone_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Get a reference to the current policy.
    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Classify a path into a security zone.
    ///
    /// Uses cached results when available for performance.
    /// The `kind` parameter is used for context but doesn't affect classification.
    pub fn classify_zone(&self, path: &Path, kind: ActionKind) -> Zone {
        let normalized = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        if let Ok(cache) = self.zone_cache.lock() {
            if let Some(zone) = cache.get(&normalized) {
                return *zone;
            }
        }

        let zone = self.classify_uncached(&normalized, kind);
        if let Ok(mut cache) = self.zone_cache.lock() {
            cache.insert(normalized, zone);
        }
        zone
    }

    /// Evaluate an action request against the policy.
    ///
    /// Returns a tuple of (Decision, Option<Zone>). The zone is `None` for
    /// network operations which don't have a filesystem path.
    ///
    /// # Determinism
    ///
    /// This method is fully deterministic: the same inputs always produce
    /// the same outputs. No LLM or probabilistic logic is used.
    pub fn evaluate(&self, request: &ActionRequest) -> (Decision, Option<Zone>) {
        match request.kind {
            ActionKind::NetworkEgress => (self.evaluate_network(request), None),
            ActionKind::Download => (self.evaluate_download(request), None),
            _ => {
                let path = request
                    .path
                    .as_ref()
                    .or(request.secondary_path.as_ref())
                    .cloned();

                let Some(path) = path else {
                    return (
                        Decision {
                            verdict: Verdict::Deny,
                            reason: ReasonCode::DenyInvalidRequest,
                            detail: "request missing target path".to_string(),
                            approval_ttl_seconds: None,
                        },
                        None,
                    );
                };

                let zone = self.classify_zone(&path, request.kind);
                let decision = match request.kind {
                    ActionKind::FileWrite | ActionKind::FileDelete | ActionKind::FileRename => {
                        self.evaluate_filesystem(request, zone)
                    }
                    ActionKind::Execute => self.evaluate_execution(request, zone),
                    ActionKind::Persistence => self.evaluate_persistence(request, zone),
                    ActionKind::SecretsRead => Decision {
                        verdict: Verdict::Deny,
                        reason: ReasonCode::DenySecrets,
                        detail: "secrets access denied by default".to_string(),
                        approval_ttl_seconds: None,
                    },
                    ActionKind::ExportCommit => self.evaluate_export(request, zone),
                    ActionKind::NetworkEgress | ActionKind::Download => unreachable!(),
                };
                (decision, Some(zone))
            }
        }
    }

    /// Get the current policy profile name.
    pub fn policy_profile(&self) -> &str {
        &self.policy.profile
    }

    /// Change the active policy profile.
    pub fn toggle_profile(&mut self, profile: String) {
        self.policy.profile = profile;
    }

    /// Check if a path matches any of the given patterns.
    fn matches_any(&self, path: &Path, patterns: &[patterns::PathPattern]) -> bool {
        patterns.iter().any(|p| p.matches(path))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use chrono::Utc;

    use crate::config::{Policy, RuleDisposition};
    use crate::model::{ActionKind, ActionRequest, ProcessContext, ReasonCode, Verdict, Zone};

    use super::PolicyEngine;

    fn base_policy() -> Policy {
        serde_yaml::from_str(include_str!("../../assets/default-policy.yaml"))
            .expect("default policy should parse")
    }

    fn request(kind: ActionKind, path: Option<PathBuf>) -> ActionRequest {
        ActionRequest {
            id: "req-1".to_string(),
            timestamp: Utc::now(),
            kind,
            operation: "test".to_string(),
            path,
            secondary_path: None,
            host: None,
            metadata: BTreeMap::new(),
            process: ProcessContext {
                pid: 111,
                ppid: Some(1),
                command: "test".to_string(),
                process_tree: vec![111],
            },
        }
    }

    #[test]
    fn classifies_system_critical() {
        let policy = base_policy().expanded(PathBuf::from("/tmp/work").as_path());
        let engine = PolicyEngine::new(policy, PathBuf::from("/tmp/work"));

        let zone = engine.classify_zone(
            PathBuf::from("/etc/passwd").as_path(),
            ActionKind::FileWrite,
        );
        assert_eq!(zone, Zone::SystemCritical);
    }

    #[test]
    fn denies_workspace_exec() {
        let policy = base_policy().expanded(PathBuf::from("/tmp/work").as_path());
        let engine = PolicyEngine::new(policy, PathBuf::from("/tmp/work"));

        let req = request(
            ActionKind::Execute,
            Some(PathBuf::from("/tmp/work/build.sh")),
        );
        let (decision, _) = engine.evaluate(&req);
        assert_eq!(decision.verdict, Verdict::Deny);
        assert_eq!(decision.reason, ReasonCode::DenyExecutionFromWorkspace);
    }

    #[test]
    fn requires_approval_for_shared_exports() {
        let policy = base_policy().expanded(PathBuf::from("/tmp/work").as_path());
        let engine = PolicyEngine::new(policy, PathBuf::from("/tmp/work"));

        let req = request(ActionKind::ExportCommit, Some(PathBuf::from("/opt/output")));
        let (decision, _) = engine.evaluate(&req);
        assert_eq!(decision.verdict, Verdict::RequireApproval);
        assert_eq!(decision.reason, ReasonCode::ApprovalRequiredExport);
    }

    #[test]
    fn user_data_write_follows_configured_zone_disposition() {
        let mut policy = base_policy().expanded(PathBuf::from("/tmp/work").as_path());
        policy.zones.user_data_paths = vec!["/tmp/managed-openclaw-home".to_string()];

        let req = request(
            ActionKind::FileWrite,
            Some(PathBuf::from(
                "/tmp/managed-openclaw-home/.openclaw/openclaw.json",
            )),
        );

        policy.rules.filesystem.user_data = RuleDisposition::Allow;
        let allow_engine = PolicyEngine::new(policy.clone(), PathBuf::from("/tmp/work"));
        let (allow_decision, allow_zone) = allow_engine.evaluate(&req);
        assert_eq!(allow_zone, Some(Zone::UserData));
        assert_eq!(allow_decision.verdict, Verdict::Allow);
        assert_eq!(allow_decision.reason, ReasonCode::AllowedByPolicy);

        policy.rules.filesystem.user_data = RuleDisposition::Approval;
        let approval_engine = PolicyEngine::new(policy.clone(), PathBuf::from("/tmp/work"));
        let (approval_decision, approval_zone) = approval_engine.evaluate(&req);
        assert_eq!(approval_zone, Some(Zone::UserData));
        assert_eq!(approval_decision.verdict, Verdict::RequireApproval);
        assert_eq!(approval_decision.reason, ReasonCode::ApprovalRequiredZone2);

        policy.rules.filesystem.user_data = RuleDisposition::Deny;
        let deny_engine = PolicyEngine::new(policy, PathBuf::from("/tmp/work"));
        let (deny_decision, deny_zone) = deny_engine.evaluate(&req);
        assert_eq!(deny_zone, Some(Zone::UserData));
        assert_eq!(deny_decision.verdict, Verdict::Deny);
        assert_eq!(deny_decision.reason, ReasonCode::DenyUserDataWrite);
    }

    #[test]
    fn denies_agent_ruler_execution_by_binary_path() {
        let policy = base_policy().expanded(PathBuf::from("/tmp/work").as_path());
        let engine = PolicyEngine::new(policy, PathBuf::from("/tmp/work"));

        let req = request(
            ActionKind::Execute,
            Some(PathBuf::from("/home/operator/.local/bin/agent-ruler")),
        );
        let (decision, _) = engine.evaluate(&req);
        assert_eq!(decision.verdict, Verdict::Deny);
        assert_eq!(decision.reason, ReasonCode::DenySystemCritical);
        assert!(
            decision.detail.contains("operator-only"),
            "expected operator-only guidance, got: {}",
            decision.detail
        );
    }

    #[test]
    fn denies_agent_ruler_execution_when_embedded_in_shell_argv() {
        let policy = base_policy().expanded(PathBuf::from("/tmp/work").as_path());
        let engine = PolicyEngine::new(policy, PathBuf::from("/tmp/work"));

        let mut req = request(ActionKind::Execute, Some(PathBuf::from("/bin/bash")));
        req.metadata.insert(
            "argv".to_string(),
            "bash -lc 'agent-ruler status --json'".to_string(),
        );

        let (decision, _) = engine.evaluate(&req);
        assert_eq!(decision.verdict, Verdict::Deny);
        assert_eq!(decision.reason, ReasonCode::DenySystemCritical);
        assert!(
            decision.detail.contains("API endpoints/tools"),
            "expected API guidance in deny detail, got: {}",
            decision.detail
        );
    }

    #[test]
    fn requires_approval_for_post_when_host_is_explicitly_allowed() {
        let mut policy = base_policy().expanded(PathBuf::from("/tmp/work").as_path());
        policy.rules.network.default_deny = true;
        policy.rules.network.allowlist_hosts = vec!["api.github.com".to_string()];
        policy.rules.network.invert_allowlist = false;
        policy.rules.network.invert_denylist = false;
        policy.rules.network.require_approval_for_post = true;
        let engine = PolicyEngine::new(policy, PathBuf::from("/tmp/work"));

        let mut req = request(ActionKind::NetworkEgress, None);
        req.host = Some("api.github.com".to_string());
        req.metadata
            .insert("method".to_string(), "POST".to_string());

        let (decision, zone) = engine.evaluate(&req);
        assert!(zone.is_none());
        assert_eq!(decision.verdict, Verdict::RequireApproval);
        assert_eq!(decision.reason, ReasonCode::ApprovalRequiredNetworkUpload);
    }
}
