use crate::helpers::approvals::{approval_diff_summary, approval_paths};
use crate::helpers::ui::payloads::ApprovalView;
use crate::model::{ApprovalRecord, ReasonCode};

pub fn approval_to_view(approval: ApprovalRecord) -> ApprovalView {
    let (resolved_src, resolved_dst) = approval_paths(&approval);
    let diff_summary = approval_diff_summary(resolved_src.as_deref(), resolved_dst.as_deref());

    ApprovalView {
        why: format!("{} | {}", reason_help(approval.reason), approval.note),
        resolved_src: resolved_src.map(|p| p.to_string_lossy().to_string()),
        resolved_dst: resolved_dst.map(|p| p.to_string_lossy().to_string()),
        diff_summary,
        approval,
    }
}

pub fn reason_help(reason: ReasonCode) -> &'static str {
    match reason {
        ReasonCode::AllowedByPolicy => "Action matched an allow rule",

        ReasonCode::ApprovalRequiredExport => "Export or delivery crosses guarded boundary",
        ReasonCode::ApprovalRequiredZone2 => "Target is in shared/system-adjacent zone",
        ReasonCode::ApprovalRequiredMassDelete => "Operation exceeded mass-delete safeguard",
        ReasonCode::ApprovalRequiredNetworkUpload => {
            "Upload-style outbound request requires explicit approval"
        }
        ReasonCode::ApprovalRequiredLargeOverwrite => {
            "Large file overwrite requires explicit approval"
        }
        ReasonCode::ApprovalRequiredSuspiciousPattern => {
            "Suspicious pattern detected, requires approval"
        }
        ReasonCode::ApprovalRequiredElevation => {
            "Elevation requires operator approval and host authentication"
        }
        ReasonCode::ApprovalRequiredPersistence => {
            "Persistence boundary operation requires explicit approval"
        }

        ReasonCode::DenySystemCritical => "System-critical zone is deny-by-default",
        ReasonCode::DenySecrets => "Secrets zone is deny-by-default",
        ReasonCode::DenyUserDataWrite => "User data writes require stage/deliver workflow",
        ReasonCode::DenyPersistence => "Persistence path is blocked by policy",
        ReasonCode::DenyInvalidRequest => "Request failed deterministic validation",
        ReasonCode::DenyUnsupportedPlatform => "Feature unsupported on this platform",
        ReasonCode::DenyConfinementToolMissing => "Confinement utility is unavailable",
        ReasonCode::DenyElevationUnsupported => {
            "Only approved mediated elevation verbs are allowed"
        }
        ReasonCode::DenyElevationPackageNotAllowlisted => {
            "Requested package is not allowlisted for elevation"
        }
        ReasonCode::DenyElevationAuthFailed => "Host elevation authentication failed",
        ReasonCode::DenyElevationExecutionFailed => "Elevated helper execution failed",
        ReasonCode::DenyElevationReplay => "Elevation request token was already used",

        ReasonCode::DenyNetworkDefault => "Outbound network is default-deny",
        ReasonCode::DenyNetworkNotAllowlisted => "Outbound host is not allowlisted",
        ReasonCode::DenyDownloadSizeExceeded => "Download size exceeds policy limit",
        ReasonCode::DenyNetworkMethodNotAllowed => "HTTP method not allowed by policy",

        ReasonCode::DenyExecutionFromWorkspace => "Execution from workspace is blocked",
        ReasonCode::DenyExecutionFromTemp => "Execution from temporary path is blocked",
        ReasonCode::DenyInterpreterStreamExec => "Interpreter stream execution (curl|bash) blocked",
        ReasonCode::DenyExecutionDownloaded => "Execution of downloaded file blocked",

        ReasonCode::QuarantineDownloadExecChain => "Download-to-exec chain triggered quarantine",
        ReasonCode::QuarantineSuspiciousExfil => "Suspicious exfiltration pattern quarantined",
        ReasonCode::QuarantineInterpreterDownload => "Downloaded script execution quarantined",
        ReasonCode::QuarantineHighRiskPattern => "High-risk pattern quarantined",
    }
}
