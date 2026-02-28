use crate::helpers::approvals::{approval_status_slug, reason_code_slug, zone_slug};
use crate::helpers::ui::payloads::RedactedStatusEvent;
use crate::model::{ActionKind, ApprovalRecord, ApprovalStatus, ReasonCode};
use crate::policy::PolicyEngine;

/// Build a redacted status event for agent polling/UI feeds.
///
/// The payload intentionally excludes raw paths/commands so agents can monitor
/// approval progress without receiving sensitive runtime details.
pub fn redacted_status_event(
    engine: &PolicyEngine,
    approval: &ApprovalRecord,
) -> RedactedStatusEvent {
    let updated_at = approval
        .decided_at
        .unwrap_or(approval.created_at)
        .to_rfc3339();

    RedactedStatusEvent {
        approval_id: approval.id.clone(),
        verdict: approval_status_slug(approval.status).to_string(),
        reason_code: reason_code_slug(approval.reason),
        category: approval_category(approval).to_string(),
        session_hint: approval_session_hint(approval),
        target_classification: approval_target_classification(engine, approval),
        guidance: approval_guidance(approval),
        open_in_webui: format!("/approvals/{}", approval.id),
        updated_at,
    }
}

fn approval_category(approval: &ApprovalRecord) -> &'static str {
    match approval.reason {
        ReasonCode::ApprovalRequiredNetworkUpload => "network_upload",
        ReasonCode::ApprovalRequiredExport => {
            if approval.action.operation == "deliver_commit" {
                "deliver"
            } else {
                "shared_zone_stage"
            }
        }
        ReasonCode::ApprovalRequiredZone2 => "shared_zone_write",
        ReasonCode::ApprovalRequiredMassDelete => "mass_delete",
        ReasonCode::ApprovalRequiredLargeOverwrite => "large_overwrite",
        ReasonCode::ApprovalRequiredSuspiciousPattern => "suspicious_pattern",
        ReasonCode::ApprovalRequiredElevation => "elevation",
        ReasonCode::ApprovalRequiredPersistence => "persistence",
        _ => "approval_required",
    }
}

fn approval_target_classification(engine: &PolicyEngine, approval: &ApprovalRecord) -> String {
    if matches!(
        approval.action.kind,
        ActionKind::NetworkEgress | ActionKind::Download
    ) {
        return "network_boundary".to_string();
    }

    if approval.action.operation == "deliver_commit" {
        return "user_destination".to_string();
    }
    if approval.action.operation == "elevation_install_packages" {
        return "system_management".to_string();
    }

    if let Some(path) = approval
        .action
        .path
        .as_deref()
        .or(approval.action.secondary_path.as_deref())
    {
        return zone_slug(engine.classify_zone(path, approval.action.kind)).to_string();
    }

    if approval.action.operation == "export_commit" {
        return "shared_zone".to_string();
    }
    if approval.action.operation == "import_copy" {
        return "workspace".to_string();
    }

    "unknown".to_string()
}

fn approval_guidance(approval: &ApprovalRecord) -> String {
    match approval.status {
        ApprovalStatus::Pending => format!(
            "waiting for approval; open /approvals/{} in WebUI",
            approval.id
        ),
        ApprovalStatus::Approved => "approved; resume blocked operation".to_string(),
        ApprovalStatus::Denied => {
            "denied; keep operation blocked or request a new approval".to_string()
        }
        ApprovalStatus::Expired => {
            "expired; resubmit request if operation is still required".to_string()
        }
    }
}

fn approval_session_hint(approval: &ApprovalRecord) -> Option<String> {
    let agent = approval
        .action
        .metadata
        .get("agent_id")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let session = approval
        .action
        .metadata
        .get("session_key")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match (agent, session) {
        (Some(agent_id), Some(session_key)) => {
            let compact = truncate_session_hint(session_key, 36);
            Some(format!("agent={agent_id} session={compact}"))
        }
        (Some(agent_id), None) => Some(format!("agent={agent_id}")),
        (None, Some(session_key)) => {
            let compact = truncate_session_hint(session_key, 36);
            Some(format!("session={compact}"))
        }
        (None, None) => None,
    }
}

fn truncate_session_hint(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        return value.to_string();
    }
    format!("{}...", &value[..max_len])
}
