use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Utc;

use crate::config::RuntimeState;
use crate::export_gate::build_export_plan;
use crate::model::{
    ActionRequest, ApprovalRecord, ApprovalStatus, Decision, DiffSummary, ReasonCode, Verdict, Zone,
};
use crate::receipts::ReceiptStore;
use crate::runner::append_receipt;

/// Resolve source/target path tuple for an approval across operation types.
pub fn approval_paths(approval: &ApprovalRecord) -> (Option<PathBuf>, Option<PathBuf>) {
    let metadata = &approval.action.metadata;

    if let (Some(src), Some(dst)) = (metadata.get("export_src"), metadata.get("export_dst")) {
        return (Some(PathBuf::from(src)), Some(PathBuf::from(dst)));
    }
    if let (Some(src), Some(dst)) = (metadata.get("import_src"), metadata.get("import_dst")) {
        return (Some(PathBuf::from(src)), Some(PathBuf::from(dst)));
    }

    (
        approval.action.path.clone(),
        approval.action.secondary_path.clone(),
    )
}

/// Build a diff summary when approval involves explicit source + destination paths.
pub fn approval_diff_summary(src: Option<&Path>, dst: Option<&Path>) -> Option<DiffSummary> {
    let (Some(src), Some(dst)) = (src, dst) else {
        return None;
    };

    build_export_plan(src, dst).ok().map(|plan| plan.summary)
}

/// Append a receipt that records final operator approval resolution.
///
/// This keeps timeline semantics consistent between CLI and WebUI actions.
pub fn append_approval_resolution_receipt(
    receipts: &ReceiptStore,
    runtime: &RuntimeState,
    approval: &ApprovalRecord,
    actor: &str,
) -> Result<()> {
    let subject = approval_subject_name(approval);
    let (src, dst) = approval_paths(approval);
    let (verdict, reason, detail) = match approval.status {
        ApprovalStatus::Approved => (
            Verdict::Allow,
            ReasonCode::AllowedByPolicy,
            approval_resolution_detail(
                "approved",
                approval,
                &subject,
                src.as_deref(),
                dst.as_deref(),
            ),
        ),
        ApprovalStatus::Denied => (
            Verdict::Deny,
            approval.reason,
            approval_resolution_detail(
                "denied",
                approval,
                &subject,
                src.as_deref(),
                dst.as_deref(),
            ),
        ),
        ApprovalStatus::Expired => (
            Verdict::Deny,
            approval.reason,
            approval_resolution_detail(
                "expired before decision",
                approval,
                &subject,
                src.as_deref(),
                dst.as_deref(),
            ),
        ),
        ApprovalStatus::Pending => return Ok(()),
    };

    let diff_summary = approval_diff_summary(src.as_deref(), dst.as_deref());

    append_receipt(
        receipts,
        runtime,
        approval.action.clone(),
        Decision {
            verdict,
            reason,
            detail,
            approval_ttl_seconds: None,
        },
        None,
        diff_summary,
        actor,
    )
}

fn approval_subject_name(approval: &ApprovalRecord) -> String {
    let metadata = &approval.action.metadata;
    if let Some(label) = metadata.get("display_name").map(String::as_str) {
        let trimmed = label.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    match approval.action.operation.as_str() {
        "export_commit" => {
            if let Some(dst) = metadata.get("export_dst").map(PathBuf::from) {
                if let Some(name) = dst.file_name().and_then(|name| name.to_str()) {
                    return name.to_string();
                }
            }
            "export request".to_string()
        }
        "deliver_commit" => {
            if let Some(dst) = metadata.get("export_dst").map(PathBuf::from) {
                if let Some(name) = dst.file_name().and_then(|name| name.to_str()) {
                    return name.to_string();
                }
            }
            "delivery request".to_string()
        }
        "import_copy" => {
            if let Some(dst) = metadata.get("import_dst").map(PathBuf::from) {
                if let Some(name) = dst.file_name().and_then(|name| name.to_str()) {
                    return name.to_string();
                }
            }
            "import request".to_string()
        }
        "elevation_install_packages" => {
            if let Some(packages) = metadata.get("elevation_packages") {
                let trimmed = packages.trim();
                if !trimmed.is_empty() {
                    return format!("install_packages [{}]", trimmed);
                }
            }
            "install_packages".to_string()
        }
        _ => {
            if let Some(path) = approval
                .action
                .path
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
            {
                return path.to_string();
            }
            approval.action.operation.replace('_', " ")
        }
    }
}

fn approval_resolution_detail(
    decision: &str,
    approval: &ApprovalRecord,
    subject: &str,
    src: Option<&Path>,
    dst: Option<&Path>,
) -> String {
    let mut lines = vec![format!("approval of `{subject}` {decision} by user")];
    lines.push(format!("approval_id: {}", approval.id));
    let mut targets = BTreeSet::new();
    if let Some(src) = src {
        targets.insert(src.display().to_string());
    }
    if let Some(dst) = dst {
        targets.insert(dst.display().to_string());
    }
    if let Some(path) = &approval.action.path {
        targets.insert(path.display().to_string());
    }
    if let Some(path) = &approval.action.secondary_path {
        targets.insert(path.display().to_string());
    }
    if !targets.is_empty() {
        lines.push("targets:".to_string());
        for target in targets {
            lines.push(format!("- {target}"));
        }
    }
    lines.join("\n")
}

pub fn append_bulk_approval_resolution_receipt(
    receipts: &ReceiptStore,
    runtime: &RuntimeState,
    approvals: &[ApprovalRecord],
    approved: bool,
    actor: &str,
) -> Result<()> {
    if approvals.is_empty() {
        return Ok(());
    }

    let mut approval_ids: Vec<String> = approvals.iter().map(|item| item.id.clone()).collect();
    approval_ids.sort();

    let mut targets = BTreeSet::new();
    for approval in approvals {
        let (src, dst) = approval_paths(approval);
        if let Some(path) = src {
            targets.insert(path.display().to_string());
        }
        if let Some(path) = dst {
            targets.insert(path.display().to_string());
        }
        if let Some(path) = &approval.action.path {
            targets.insert(path.display().to_string());
        }
        if let Some(path) = &approval.action.secondary_path {
            targets.insert(path.display().to_string());
        }
    }

    let verb = if approved { "approved" } else { "denied" };
    let mut detail = vec![format!(
        "bulk approval ({}) {} by user",
        approvals.len(),
        verb
    )];
    detail.push(format!("approval_ids: {}", approval_ids.join(", ")));
    if !targets.is_empty() {
        detail.push("targets:".to_string());
        for target in &targets {
            detail.push(format!("- {target}"));
        }
    }

    let first = &approvals[0];
    let mut action: ActionRequest = first.action.clone();
    action.id = format!("bulk-approval-{}", first.id);
    action.timestamp = Utc::now();
    action.operation = if approved {
        "approval_bulk_resolution_approve".to_string()
    } else {
        "approval_bulk_resolution_deny".to_string()
    };
    action.path = None;
    action.secondary_path = None;
    action
        .metadata
        .insert("bulk_approval_ids".to_string(), approval_ids.join(","));
    action
        .metadata
        .insert("bulk_target_count".to_string(), targets.len().to_string());

    let reason = if approved {
        ReasonCode::AllowedByPolicy
    } else {
        first.reason
    };
    let verdict = if approved {
        Verdict::Allow
    } else {
        Verdict::Deny
    };

    append_receipt(
        receipts,
        runtime,
        action,
        Decision {
            verdict,
            reason,
            detail: detail.join("\n"),
            approval_ttl_seconds: None,
        },
        None,
        None,
        actor,
    )
}

/// Serialize approval status to stable lower-case slug used in API payloads.
pub fn approval_status_slug(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Pending => "pending",
        ApprovalStatus::Approved => "approved",
        ApprovalStatus::Denied => "denied",
        ApprovalStatus::Expired => "expired",
    }
}

/// Convert reason code enum to user-facing slug.
pub fn reason_code_slug(reason: ReasonCode) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "unknown_reason".to_string())
}

/// Convert zone enum to stable lower-case slug.
pub fn zone_slug(zone: Zone) -> &'static str {
    match zone {
        Zone::Workspace => "workspace",
        Zone::UserData => "user_data",
        Zone::Shared => "shared_zone",
        Zone::SystemCritical => "system_critical",
        Zone::Secrets => "secrets",
    }
}
