//! Core data models for Agent Ruler policy engine.
//!
//! This module defines the fundamental types used throughout the policy evaluation
//! and enforcement system. All types are serializable for persistence and API use.
//!
//! # Key Types
//!
//! - [`ActionRequest`] - An incoming action to be evaluated
//! - [`Decision`] - The policy decision (allow/deny/approval/quarantine)
//! - [`Zone`] - Security zone classification for paths
//! - [`Verdict`] - The outcome of policy evaluation
//! - [`ReasonCode`] - Stable identifiers explaining why a decision was made
//! - [`Receipt`] - Audit record of an action and its decision
//!
//! # Security Invariants
//!
//! - All decisions are deterministic: same inputs always produce same outputs
//! - Reason codes are stable and should not change (used in tests and logs)
//! - Receipts are append-only and must not be modified after creation
//!
//! # Tests
//!
//! See `/tests/` directory for integration tests covering policy decisions.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Security zones for path classification.
///
/// Zones determine what rules apply to a given path. Classification is based on
/// path prefixes defined in the policy configuration.
///
/// # Zone Hierarchy
///
/// - `Workspace` (Zone 0): Agent working directory, typically permissive
/// - `UserData` (Zone 1): User documents and config, controlled access
/// - `Shared` (Zone 2): Export staging area, requires approval for writes
/// - `SystemCritical` (Zone 3): System binaries and config, always denied
/// - `Secrets` (Zone 4): Credentials and keys, always denied
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Zone {
    /// Zone 0: Agent working directory with full read/write access
    Workspace,
    /// Zone 1: User documents and application config
    UserData,
    /// Zone 2: Export staging and approval boundary
    Shared,
    /// Zone 3: System binaries and host-critical configuration (always denied)
    SystemCritical,
    /// Zone 4: Credentials, keys, and sensitive material (always denied)
    Secrets,
}

/// Types of actions that can be evaluated by the policy engine.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// File write/create operation
    FileWrite,
    /// File delete operation
    FileDelete,
    /// File rename/move operation
    FileRename,
    /// Command execution
    Execute,
    /// Network egress (HTTP request)
    NetworkEgress,
    /// Persistence mechanism (autostart, cron, etc.)
    Persistence,
    /// Reading secrets/credentials
    SecretsRead,
    /// Committing an export to delivery
    ExportCommit,
    /// Download operation
    Download,
}

/// Process context for an action request.
///
/// Captures the process tree information for audit and security analysis.
/// Used to detect suspicious patterns like curl|bash execution chains.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessContext {
    /// Process ID of the action initiator
    pub pid: u32,
    /// Parent process ID if available
    pub ppid: Option<u32>,
    /// Full command line
    pub command: String,
    /// Ancestor PIDs for process tree analysis
    pub process_tree: Vec<u32>,
}

/// A request to perform an action, evaluated by the policy engine.
///
/// This is the primary input to [`PolicyEngine::evaluate`](crate::policy::PolicyEngine::evaluate).
/// Each request captures what action is being attempted, by whom, and with what context.
///
/// # Metadata Keys
///
/// Standard metadata keys used by the policy engine:
/// - `downloaded`: "true" if this action originated from downloaded content
/// - `download_source`: URL of the download source
/// - `interpreter`: "true" if this is an interpreter execution (python, bash)
/// - `stream_exec`: "true" if this is a pipe-style execution (curl | bash)
/// - `method`: HTTP method for network requests (GET, POST, etc.)
/// - `argv`: Full command line for execution requests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRequest {
    /// Unique identifier for this request
    pub id: String,
    /// When this request was created
    pub timestamp: DateTime<Utc>,
    /// Type of action being requested
    pub kind: ActionKind,
    /// Human-readable operation description
    pub operation: String,
    /// Primary path for file operations
    pub path: Option<PathBuf>,
    /// Secondary path for rename/copy operations
    pub secondary_path: Option<PathBuf>,
    /// Target host for network operations
    pub host: Option<String>,
    /// Additional context for policy evaluation
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    /// Process context for security analysis
    pub process: ProcessContext,
}

impl ActionRequest {
    /// Check if this request originated from downloaded/untrusted content
    pub fn is_from_download(&self) -> bool {
        self.metadata
            .get("downloaded")
            .map(|v| v == "true")
            .unwrap_or(false)
    }

    /// Check if this is an interpreter execution (script via python, bash, etc.)
    pub fn is_interpreter_exec(&self) -> bool {
        self.metadata
            .get("interpreter")
            .map(|v| v == "true")
            .unwrap_or(false)
    }

    /// Check if this is a stream-style execution (curl | bash pattern)
    pub fn is_stream_exec(&self) -> bool {
        self.metadata
            .get("stream_exec")
            .map(|v| v == "true")
            .unwrap_or(false)
    }

    /// Get the download source URL if this originated from a download
    pub fn download_source(&self) -> Option<&str> {
        self.metadata.get("download_source").map(|s| s.as_str())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Allow,
    Deny,
    RequireApproval,
    Quarantine,
}

/// Reason codes for policy decisions.
///
/// These are stable, deterministic identifiers that explain why a decision was made.
/// They are used in receipts, UI explanations, and testing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    // === Allow ===
    /// Action permitted by policy
    AllowedByPolicy,

    // === Approval Required ===
    /// Shared zone (Zone 2) requires approval
    ApprovalRequiredZone2,
    /// Export/deliver operation requires approval
    ApprovalRequiredExport,
    /// Mass delete operation exceeds threshold
    ApprovalRequiredMassDelete,
    /// Network upload-style request requires approval
    ApprovalRequiredNetworkUpload,
    /// Large overwrite operation requires approval
    ApprovalRequiredLargeOverwrite,
    /// Suspicious pattern detected, requires approval
    ApprovalRequiredSuspiciousPattern,
    /// Elevated host action requires approval + operator auth
    ApprovalRequiredElevation,
    /// Persistence boundary operation requires approval
    ApprovalRequiredPersistence,

    // === Deny - System ===
    /// System-critical path access denied
    DenySystemCritical,
    /// Secrets path access denied
    DenySecrets,
    /// User data write denied (reads allowed, writes require stage/deliver)
    DenyUserDataWrite,
    /// Persistence attempt denied
    DenyPersistence,
    /// Invalid request format
    DenyInvalidRequest,
    /// Platform not supported
    DenyUnsupportedPlatform,
    /// Confinement tool (bubblewrap) not available
    DenyConfinementToolMissing,
    /// Sudo/elevation command is not supported by mediated helper
    DenyElevationUnsupported,
    /// Requested package is not in elevation allowlist
    DenyElevationPackageNotAllowlisted,
    /// Operator elevation authentication failed
    DenyElevationAuthFailed,
    /// Elevated helper execution failed
    DenyElevationExecutionFailed,
    /// One-time elevation nonce replay detected
    DenyElevationReplay,

    // === Deny - Network ===
    /// Network access denied by default policy
    DenyNetworkDefault,
    /// Domain not in allowlist
    DenyNetworkNotAllowlisted,
    /// Download size exceeds limit
    DenyDownloadSizeExceeded,
    /// HTTP method not allowed
    DenyNetworkMethodNotAllowed,

    // === Deny - Execution ===
    /// Execution from workspace denied
    DenyExecutionFromWorkspace,
    /// Execution from temp directory denied
    DenyExecutionFromTemp,
    /// Interpreter stream execution denied (curl | bash pattern)
    DenyInterpreterStreamExec,
    /// Execution of downloaded file denied
    DenyExecutionDownloaded,

    // === Quarantine ===
    /// Download→exec chain quarantined
    QuarantineDownloadExecChain,
    /// Suspicious exfiltration pattern quarantined
    QuarantineSuspiciousExfil,
    /// Interpreter execution of downloaded script quarantined
    QuarantineInterpreterDownload,
    /// High-risk pattern quarantined
    QuarantineHighRiskPattern,
}

impl ReasonCode {
    /// Get a human-readable description of this reason code
    pub fn description(&self) -> &'static str {
        match self {
            ReasonCode::AllowedByPolicy => "Action permitted by policy",
            ReasonCode::ApprovalRequiredZone2 => "Shared zone operation requires approval",
            ReasonCode::ApprovalRequiredExport => "Export/deliver operation requires approval",
            ReasonCode::ApprovalRequiredMassDelete => "Mass delete exceeds safety threshold",
            ReasonCode::ApprovalRequiredNetworkUpload => "Network upload requires approval",
            ReasonCode::ApprovalRequiredLargeOverwrite => "Large file overwrite requires approval",
            ReasonCode::ApprovalRequiredSuspiciousPattern => "Suspicious pattern requires approval",
            ReasonCode::ApprovalRequiredElevation => "Elevation request requires approval",
            ReasonCode::ApprovalRequiredPersistence => {
                "Persistence boundary operation requires approval"
            }
            ReasonCode::DenySystemCritical => "System-critical path access denied",
            ReasonCode::DenySecrets => "Secrets path access denied",
            ReasonCode::DenyUserDataWrite => "User data write denied (use stage/deliver workflow)",
            ReasonCode::DenyPersistence => "Persistence attempt denied",
            ReasonCode::DenyInvalidRequest => "Invalid request format",
            ReasonCode::DenyUnsupportedPlatform => "Platform not supported",
            ReasonCode::DenyConfinementToolMissing => "Confinement tool not available",
            ReasonCode::DenyElevationUnsupported => "Unsupported elevation request",
            ReasonCode::DenyElevationPackageNotAllowlisted => "Elevation package not in allowlist",
            ReasonCode::DenyElevationAuthFailed => "Elevation authentication failed",
            ReasonCode::DenyElevationExecutionFailed => "Elevation helper execution failed",
            ReasonCode::DenyElevationReplay => "Elevation nonce replay detected",
            ReasonCode::DenyNetworkDefault => "Network access denied by default",
            ReasonCode::DenyNetworkNotAllowlisted => "Domain not in allowlist",
            ReasonCode::DenyDownloadSizeExceeded => "Download size exceeds limit",
            ReasonCode::DenyNetworkMethodNotAllowed => "HTTP method not allowed",
            ReasonCode::DenyExecutionFromWorkspace => "Execution from workspace denied",
            ReasonCode::DenyExecutionFromTemp => "Execution from temp directory denied",
            ReasonCode::DenyInterpreterStreamExec => "Interpreter stream execution denied",
            ReasonCode::DenyExecutionDownloaded => "Execution of downloaded file denied",
            ReasonCode::QuarantineDownloadExecChain => "Download→exec chain quarantined",
            ReasonCode::QuarantineSuspiciousExfil => "Suspicious exfiltration pattern quarantined",
            ReasonCode::QuarantineInterpreterDownload => "Downloaded script execution quarantined",
            ReasonCode::QuarantineHighRiskPattern => "High-risk pattern quarantined",
        }
    }

    /// Check if this reason code indicates a quarantine verdict
    pub fn is_quarantine(&self) -> bool {
        matches!(
            self,
            ReasonCode::QuarantineDownloadExecChain
                | ReasonCode::QuarantineSuspiciousExfil
                | ReasonCode::QuarantineInterpreterDownload
                | ReasonCode::QuarantineHighRiskPattern
        )
    }

    /// Check if this reason code requires approval
    pub fn requires_approval(&self) -> bool {
        matches!(
            self,
            ReasonCode::ApprovalRequiredZone2
                | ReasonCode::ApprovalRequiredExport
                | ReasonCode::ApprovalRequiredMassDelete
                | ReasonCode::ApprovalRequiredNetworkUpload
                | ReasonCode::ApprovalRequiredLargeOverwrite
                | ReasonCode::ApprovalRequiredSuspiciousPattern
                | ReasonCode::ApprovalRequiredElevation
                | ReasonCode::ApprovalRequiredPersistence
        )
    }

    /// Get the OWASP category this reason code relates to
    pub fn owasp_category(&self) -> Option<&'static str> {
        match self {
            // Input Validation
            ReasonCode::DenyNetworkNotAllowlisted
            | ReasonCode::DenyDownloadSizeExceeded
            | ReasonCode::DenyNetworkMethodNotAllowed => Some("Input Validation"),

            // Access Control
            ReasonCode::DenySystemCritical
            | ReasonCode::DenySecrets
            | ReasonCode::DenyUserDataWrite
            | ReasonCode::DenyPersistence
            | ReasonCode::DenyExecutionFromWorkspace
            | ReasonCode::DenyExecutionFromTemp
            | ReasonCode::DenyExecutionDownloaded
            | ReasonCode::DenyInterpreterStreamExec
            | ReasonCode::DenyElevationUnsupported
            | ReasonCode::DenyElevationPackageNotAllowlisted
            | ReasonCode::DenyElevationAuthFailed
            | ReasonCode::DenyElevationExecutionFailed
            | ReasonCode::DenyElevationReplay => Some("Access Control"),

            // Sandboxing
            ReasonCode::DenyConfinementToolMissing | ReasonCode::DenyUnsupportedPlatform => {
                Some("Sandboxing")
            }

            // Data Exfiltration
            ReasonCode::DenyNetworkDefault
            | ReasonCode::ApprovalRequiredNetworkUpload
            | ReasonCode::QuarantineSuspiciousExfil => Some("Data Exfiltration"),

            // Download→Exec
            ReasonCode::QuarantineDownloadExecChain | ReasonCode::QuarantineInterpreterDownload => {
                Some("Download-Exec Chain")
            }

            // Human-in-Loop
            ReasonCode::ApprovalRequiredZone2
            | ReasonCode::ApprovalRequiredExport
            | ReasonCode::ApprovalRequiredMassDelete
            | ReasonCode::ApprovalRequiredLargeOverwrite
            | ReasonCode::ApprovalRequiredSuspiciousPattern
            | ReasonCode::ApprovalRequiredElevation
            | ReasonCode::ApprovalRequiredPersistence => Some("Human-in-Loop"),

            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub verdict: Verdict,
    pub reason: ReasonCode,
    pub detail: String,
    pub approval_ttl_seconds: Option<u64>,
}

impl Decision {
    /// Create an allow decision
    pub fn allow(reason: ReasonCode, detail: String) -> Self {
        Self {
            verdict: Verdict::Allow,
            reason,
            detail,
            approval_ttl_seconds: None,
        }
    }

    /// Create a deny decision
    pub fn deny(reason: ReasonCode, detail: String) -> Self {
        Self {
            verdict: Verdict::Deny,
            reason,
            detail,
            approval_ttl_seconds: None,
        }
    }

    /// Create a require-approval decision
    pub fn require_approval(reason: ReasonCode, detail: String, ttl_seconds: u64) -> Self {
        Self {
            verdict: Verdict::RequireApproval,
            reason,
            detail,
            approval_ttl_seconds: Some(ttl_seconds),
        }
    }

    /// Create a quarantine decision
    pub fn quarantine(reason: ReasonCode, detail: String) -> Self {
        Self {
            verdict: Verdict::Quarantine,
            reason,
            detail,
            approval_ttl_seconds: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffSummary {
    pub files_added: usize,
    pub files_removed: usize,
    pub files_changed: usize,
    pub bytes_added: u64,
    pub bytes_removed: u64,
}

impl DiffSummary {
    pub fn is_empty(&self) -> bool {
        self.files_added == 0
            && self.files_removed == 0
            && self.files_changed == 0
            && self.bytes_added == 0
            && self.bytes_removed == 0
    }

    pub fn total_changes(&self) -> usize {
        self.files_added + self.files_removed + self.files_changed
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub action: ActionRequest,
    pub decision: Decision,
    pub zone: Option<Zone>,
    pub policy_version: String,
    pub policy_hash: String,
    pub diff_summary: Option<DiffSummary>,
    pub confinement: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub status: ApprovalStatus,
    pub reason: ReasonCode,
    pub scope_key: String,
    pub action: ActionRequest,
    pub note: String,
    pub decided_at: Option<DateTime<Utc>>,
}

impl ApprovalRecord {
    /// Check if this approval has expired
    pub fn is_expired(&self) -> bool {
        self.status == ApprovalStatus::Pending && Utc::now() > self.expires_at
    }

    /// Check if this approval is still valid for use
    pub fn is_valid(&self) -> bool {
        self.status == ApprovalStatus::Approved && Utc::now() <= self.expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reason_code_description() {
        assert!(!ReasonCode::AllowedByPolicy.description().is_empty());
        assert!(!ReasonCode::QuarantineDownloadExecChain
            .description()
            .is_empty());
    }

    #[test]
    fn test_reason_code_categories() {
        assert_eq!(
            ReasonCode::DenySystemCritical.owasp_category(),
            Some("Access Control")
        );
        assert_eq!(
            ReasonCode::QuarantineDownloadExecChain.owasp_category(),
            Some("Download-Exec Chain")
        );
    }

    #[test]
    fn test_decision_constructors() {
        let allow = Decision::allow(ReasonCode::AllowedByPolicy, "test".to_string());
        assert_eq!(allow.verdict, Verdict::Allow);

        let deny = Decision::deny(ReasonCode::DenySystemCritical, "test".to_string());
        assert_eq!(deny.verdict, Verdict::Deny);

        let approval =
            Decision::require_approval(ReasonCode::ApprovalRequiredZone2, "test".to_string(), 3600);
        assert_eq!(approval.verdict, Verdict::RequireApproval);
        assert_eq!(approval.approval_ttl_seconds, Some(3600));

        let quarantine =
            Decision::quarantine(ReasonCode::QuarantineDownloadExecChain, "test".to_string());
        assert_eq!(quarantine.verdict, Verdict::Quarantine);
    }

    #[test]
    fn test_diff_summary() {
        let empty = DiffSummary::default();
        assert!(empty.is_empty());
        assert_eq!(empty.total_changes(), 0);

        let non_empty = DiffSummary {
            files_added: 2,
            files_removed: 1,
            files_changed: 3,
            bytes_added: 100,
            bytes_removed: 50,
        };
        assert!(!non_empty.is_empty());
        assert_eq!(non_empty.total_changes(), 6);
    }
}
