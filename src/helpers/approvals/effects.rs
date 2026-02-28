use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::config::RuntimeState;
use crate::export_gate::{build_export_plan, commit_export};
use crate::model::{ApprovalRecord, Decision, ReasonCode, Verdict};
use crate::receipts::ReceiptStore;
use crate::runner::append_receipt;
use crate::staged_exports::StagedExportStore;

use crate::helpers::runtime::apply_plan_with_mode;

pub fn maybe_apply_approval_effect(
    runtime: &RuntimeState,
    approval: &ApprovalRecord,
    receipts: &ReceiptStore,
) -> Result<()> {
    let op = approval.action.operation.as_str();
    let staged_store = StagedExportStore::new(&runtime.config.staged_exports_file);

    match op {
        "export_commit" => {
            let src = approval
                .action
                .metadata
                .get("export_src")
                .ok_or_else(|| anyhow!("approved export missing export_src metadata"))?;
            let dst = approval
                .action
                .metadata
                .get("export_dst")
                .ok_or_else(|| anyhow!("approved export missing export_dst metadata"))?;

            let src = PathBuf::from(src);
            let dst = PathBuf::from(dst);
            let plan = build_export_plan(&src, &dst)?;
            commit_export(&plan)?;

            if let Some(stage_id) = approval.action.metadata.get("stage_id") {
                let _ = staged_store
                    .mark_staged(stage_id, format!("staged after approval {}", approval.id));
            }

            append_receipt(
                receipts,
                runtime,
                approval.action.clone(),
                Decision {
                    verdict: Verdict::Allow,
                    reason: ReasonCode::AllowedByPolicy,
                    detail: format!("export staged after approval {}", approval.id),
                    approval_ttl_seconds: None,
                },
                None,
                Some(plan.summary),
                "approval-effect-stage",
            )?;
        }
        "deliver_commit" => {
            let src = approval
                .action
                .metadata
                .get("export_src")
                .ok_or_else(|| anyhow!("approved delivery missing export_src metadata"))?;
            let dst = approval
                .action
                .metadata
                .get("export_dst")
                .ok_or_else(|| anyhow!("approved delivery missing export_dst metadata"))?;
            let move_artifact = approval
                .action
                .metadata
                .get("move_artifact")
                .map(|v| v == "true")
                .unwrap_or(false);

            let src = PathBuf::from(src);
            let dst = PathBuf::from(dst);
            let plan = build_export_plan(&src, &dst)?;
            apply_plan_with_mode(&plan, move_artifact)?;

            if let Some(stage_id) = approval.action.metadata.get("stage_id") {
                let _ = staged_store.mark_delivered(
                    stage_id,
                    &dst,
                    format!("delivered after approval {}", approval.id),
                );
            }

            append_receipt(
                receipts,
                runtime,
                approval.action.clone(),
                Decision {
                    verdict: Verdict::Allow,
                    reason: ReasonCode::AllowedByPolicy,
                    detail: format!("Delivered to {}", dst.display()),
                    approval_ttl_seconds: None,
                },
                None,
                Some(plan.summary),
                "approval-effect-delivery",
            )?;
        }
        "import_copy" => {
            let src = approval
                .action
                .metadata
                .get("import_src")
                .ok_or_else(|| anyhow!("approved import missing import_src metadata"))?;
            let dst = approval
                .action
                .metadata
                .get("import_dst")
                .ok_or_else(|| anyhow!("approved import missing import_dst metadata"))?;

            let src = PathBuf::from(src);
            let dst = PathBuf::from(dst);
            let plan = build_export_plan(&src, &dst)?;
            commit_export(&plan)?;

            append_receipt(
                receipts,
                runtime,
                approval.action.clone(),
                Decision {
                    verdict: Verdict::Allow,
                    reason: ReasonCode::AllowedByPolicy,
                    detail: format!("import committed after approval {}", approval.id),
                    approval_ttl_seconds: None,
                },
                None,
                Some(plan.summary),
                "approval-effect-import",
            )?;
        }
        "elevation_install_packages" => match apply_elevation_install_packages(runtime, approval) {
            Ok(detail) => {
                append_receipt(
                    receipts,
                    runtime,
                    approval.action.clone(),
                    Decision {
                        verdict: Verdict::Allow,
                        reason: ReasonCode::AllowedByPolicy,
                        detail,
                        approval_ttl_seconds: None,
                    },
                    None,
                    None,
                    "approval-effect-elevation",
                )?;
            }
            Err(err) => {
                append_receipt(
                    receipts,
                    runtime,
                    approval.action.clone(),
                    Decision {
                        verdict: Verdict::Deny,
                        reason: err.reason,
                        detail: err.detail.clone(),
                        approval_ttl_seconds: None,
                    },
                    None,
                    None,
                    "approval-effect-elevation-deny",
                )?;
                return Err(anyhow!(err.detail));
            }
        },
        _ => {}
    }

    Ok(())
}

#[derive(Debug)]
struct ElevationEffectError {
    reason: ReasonCode,
    detail: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct UsedElevationNonces {
    nonces: BTreeSet<String>,
}

fn apply_elevation_install_packages(
    runtime: &RuntimeState,
    approval: &ApprovalRecord,
) -> std::result::Result<String, ElevationEffectError> {
    let rules = &runtime.policy.rules.elevation;
    if !rules.enabled {
        return Err(ElevationEffectError {
            reason: ReasonCode::DenyElevationUnsupported,
            detail: "elevation helper is disabled by policy".to_string(),
        });
    }

    if approval.expires_at < Utc::now() {
        return Err(ElevationEffectError {
            reason: ReasonCode::DenyInvalidRequest,
            detail: format!("approved elevation {} has expired", approval.id),
        });
    }

    let nonce = approval
        .action
        .metadata
        .get("elevation_nonce")
        .cloned()
        .ok_or_else(|| ElevationEffectError {
            reason: ReasonCode::DenyInvalidRequest,
            detail: "approved elevation missing nonce".to_string(),
        })?;
    consume_elevation_nonce_once(runtime, &nonce)?;

    let packages = parse_elevation_packages(approval)?;
    if packages.is_empty() {
        return Err(ElevationEffectError {
            reason: ReasonCode::DenyInvalidRequest,
            detail: "approved elevation contains no package targets".to_string(),
        });
    }

    for package in &packages {
        if rules.denied_packages.iter().any(|denied| denied == package) {
            return Err(ElevationEffectError {
                reason: ReasonCode::DenyElevationPackageNotAllowlisted,
                detail: format!("approved elevation package '{}' is denylisted", package),
            });
        }
    }

    if rules.use_allowlist && rules.allowed_packages.is_empty() {
        return Err(ElevationEffectError {
            reason: ReasonCode::DenyElevationPackageNotAllowlisted,
            detail: "use_allowlist is enabled, but no packages are allowlisted".to_string(),
        });
    }

    if rules.use_allowlist {
        for package in &packages {
            if !rules
                .allowed_packages
                .iter()
                .any(|allowed| allowed == package)
            {
                return Err(ElevationEffectError {
                    reason: ReasonCode::DenyElevationPackageNotAllowlisted,
                    detail: format!(
                        "approved elevation package '{}' is not in allowlist",
                        package
                    ),
                });
            }
        }
    }

    if rules.require_operator_auth {
        ensure_operator_auth()?;
    }

    run_install_packages_helper(&packages)?;

    Ok(format!(
        "elevation helper completed install_packages for [{}]",
        packages.join(", ")
    ))
}

fn parse_elevation_packages(
    approval: &ApprovalRecord,
) -> std::result::Result<Vec<String>, ElevationEffectError> {
    let raw = approval
        .action
        .metadata
        .get("elevation_packages")
        .cloned()
        .ok_or_else(|| ElevationEffectError {
            reason: ReasonCode::DenyInvalidRequest,
            detail: "approved elevation missing package list".to_string(),
        })?;

    let mut packages = Vec::new();
    for token in raw.split(',') {
        let package = token.trim();
        if package.is_empty() {
            continue;
        }
        if !is_valid_debian_package_name(package) {
            return Err(ElevationEffectError {
                reason: ReasonCode::DenyInvalidRequest,
                detail: format!("invalid package token in approved elevation: '{}'", package),
            });
        }
        packages.push(package.to_string());
    }
    packages.sort();
    packages.dedup();
    Ok(packages)
}

fn consume_elevation_nonce_once(
    runtime: &RuntimeState,
    nonce: &str,
) -> std::result::Result<(), ElevationEffectError> {
    let file = runtime.config.state_dir.join("elevation-used-nonces.json");
    let mut state = if file.exists() {
        let raw = fs::read_to_string(&file).map_err(|err| ElevationEffectError {
            reason: ReasonCode::DenyElevationExecutionFailed,
            detail: format!("read elevation nonce state failed: {err}"),
        })?;
        serde_json::from_str::<UsedElevationNonces>(&raw).unwrap_or_default()
    } else {
        UsedElevationNonces::default()
    };

    if state.nonces.contains(nonce) {
        return Err(ElevationEffectError {
            reason: ReasonCode::DenyElevationReplay,
            detail: "elevation nonce replay detected".to_string(),
        });
    }

    state.nonces.insert(nonce.to_string());
    let temp = file.with_extension("json.tmp");
    let payload = serde_json::to_string_pretty(&state).map_err(|err| ElevationEffectError {
        reason: ReasonCode::DenyElevationExecutionFailed,
        detail: format!("serialize elevation nonce state failed: {err}"),
    })?;
    fs::write(&temp, payload).map_err(|err| ElevationEffectError {
        reason: ReasonCode::DenyElevationExecutionFailed,
        detail: format!("write elevation nonce temp file failed: {err}"),
    })?;
    fs::rename(&temp, &file).map_err(|err| ElevationEffectError {
        reason: ReasonCode::DenyElevationExecutionFailed,
        detail: format!("replace elevation nonce state failed: {err}"),
    })?;
    Ok(())
}

fn ensure_operator_auth() -> std::result::Result<(), ElevationEffectError> {
    if std::env::var("AR_ELEVATION_AUTH_MODE")
        .ok()
        .map(|value| value.eq_ignore_ascii_case("mock"))
        .unwrap_or(false)
    {
        return Ok(());
    }

    let status = Command::new("sudo")
        .arg("-v")
        .status()
        .map_err(|err| ElevationEffectError {
            reason: ReasonCode::DenyElevationAuthFailed,
            detail: format!("failed to launch sudo auth prompt: {err}"),
        })?;

    if !status.success() {
        return Err(ElevationEffectError {
            reason: ReasonCode::DenyElevationAuthFailed,
            detail: "operator authentication failed for elevated helper".to_string(),
        });
    }

    Ok(())
}

fn run_install_packages_helper(
    packages: &[String],
) -> std::result::Result<(), ElevationEffectError> {
    if std::env::var("AR_ELEVATION_HELPER_MODE")
        .ok()
        .map(|value| value.eq_ignore_ascii_case("mock"))
        .unwrap_or(false)
    {
        return Ok(());
    }

    let apt_binary = if PathBuf::from("/usr/bin/apt-get").exists() {
        "/usr/bin/apt-get"
    } else if PathBuf::from("/usr/bin/apt").exists() {
        "/usr/bin/apt"
    } else {
        return Err(ElevationEffectError {
            reason: ReasonCode::DenyElevationExecutionFailed,
            detail: "apt helper is unavailable on this host".to_string(),
        });
    };

    let mut cmd = Command::new("sudo");
    cmd.arg("--")
        .arg(apt_binary)
        .arg("install")
        .arg("-y")
        .arg("--no-install-recommends");
    for package in packages {
        cmd.arg(package);
    }

    let output = cmd.output().map_err(|err| ElevationEffectError {
        reason: ReasonCode::DenyElevationExecutionFailed,
        detail: format!("failed to execute elevated helper: {err}"),
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ElevationEffectError {
            reason: ReasonCode::DenyElevationExecutionFailed,
            detail: if stderr.is_empty() {
                "elevated helper returned non-zero status".to_string()
            } else {
                format!("elevated helper failed: {stderr}")
            },
        });
    }

    Ok(())
}

fn is_valid_debian_package_name(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(ch) if ch.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-'))
}
