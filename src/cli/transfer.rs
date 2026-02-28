use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;

use agent_ruler::approvals::ApprovalStore;
use agent_ruler::config::RuntimeState;
use agent_ruler::export_gate::{build_export_plan, commit_export, ExportPlan};
use agent_ruler::model::{
    ActionKind, ActionRequest, Decision, ProcessContext, ReasonCode, Verdict,
};
use agent_ruler::policy::PolicyEngine;
use agent_ruler::receipts::ReceiptStore;
use agent_ruler::runner::append_receipt;
use agent_ruler::staged_exports::{StagedExportRecord, StagedExportState, StagedExportStore};

const BYPASS_ACK_HINT: &str =
    "set --i-understand-bypass-risk to acknowledge policy bypass and reduced audit guarantees";

#[allow(clippy::too_many_arguments)]
pub fn run_export(
    runtime: &RuntimeState,
    src: &Path,
    dst: &Path,
    preview_only: bool,
    force: bool,
    bypass: bool,
    bypass_ack: bool,
    actor: &str,
) -> Result<()> {
    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let engine = PolicyEngine::new(runtime.policy.clone(), runtime.config.workspace.clone());
    let staged_store = StagedExportStore::new(&runtime.config.staged_exports_file);

    let src = normalize_workspace_path(runtime, src);
    let staged_dst = normalize_stage_path(runtime, dst)?;
    let plan = build_export_plan(&src, &staged_dst)?;

    println!(
        "stage preview {} -> {}",
        src.display(),
        staged_dst.display()
    );
    print_plan_summary(&plan);

    let stage_id = uuid::Uuid::new_v4().to_string();
    let base_record = StagedExportRecord {
        id: stage_id.clone(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        src_workspace: src.to_string_lossy().to_string(),
        staged_path: staged_dst.to_string_lossy().to_string(),
        state: StagedExportState::Failed,
        stage_approval_id: None,
        delivery_approval_id: None,
        delivery_destination: None,
        delivered_to: None,
        delivered_at: None,
        last_message: None,
    };

    if preview_only {
        return Ok(());
    }

    let action = build_export_action(&src, &staged_dst, actor, Some(stage_id.clone()));

    if bypass {
        ensure_bypass_ack(bypass_ack)?;
        commit_export(&plan)?;
        staged_store.upsert(StagedExportRecord {
            state: StagedExportState::Staged,
            last_message: Some("staged with unsafe bypass".to_string()),
            ..base_record
        })?;
        append_receipt(
            &receipts,
            runtime,
            action,
            Decision {
                verdict: Verdict::Allow,
                reason: ReasonCode::AllowedByPolicy,
                detail: "unsafe bypass enabled: stage copied without policy evaluation".to_string(),
                approval_ttl_seconds: None,
            },
            None,
            Some(plan.summary),
            "export-stage-bypass",
        )?;
        println!(
            "staged to {} with bypass enabled (policy checks skipped)",
            staged_dst.display()
        );
        return Ok(());
    }

    let (decision, zone) = engine.evaluate(&action);
    match decision.verdict {
        Verdict::Allow => {
            commit_export(&plan)?;
            staged_store.upsert(StagedExportRecord {
                state: StagedExportState::Staged,
                last_message: Some("staged and ready for delivery".to_string()),
                ..base_record
            })?;
            append_receipt(
                &receipts,
                runtime,
                action,
                Decision {
                    verdict: Verdict::Allow,
                    reason: ReasonCode::AllowedByPolicy,
                    detail: "export staged in shared-zone".to_string(),
                    approval_ttl_seconds: None,
                },
                zone,
                Some(plan.summary),
                "export-stage",
            )?;
            println!("staged export {} (id: {})", staged_dst.display(), stage_id);
        }
        Verdict::RequireApproval => {
            if force || approvals.has_active_approval_for(&action)? {
                commit_export(&plan)?;
                staged_store.upsert(StagedExportRecord {
                    state: StagedExportState::Staged,
                    last_message: Some("staged via explicit override/approval".to_string()),
                    ..base_record
                })?;
                append_receipt(
                    &receipts,
                    runtime,
                    action,
                    Decision {
                        verdict: Verdict::Allow,
                        reason: ReasonCode::AllowedByPolicy,
                        detail: "export staged via explicit override/approval".to_string(),
                        approval_ttl_seconds: None,
                    },
                    zone,
                    Some(plan.summary),
                    "export-stage-approved",
                )?;
                println!(
                    "staged export {} with approval scope (id: {})",
                    staged_dst.display(),
                    stage_id
                );
            } else {
                let approval = approvals.create_pending(
                    &action,
                    &decision,
                    "stage export requires approval",
                )?;
                staged_store.upsert(StagedExportRecord {
                    state: StagedExportState::PendingStageApproval,
                    stage_approval_id: Some(approval.id.clone()),
                    last_message: Some("awaiting stage approval".to_string()),
                    ..base_record
                })?;
                append_receipt(
                    &receipts,
                    runtime,
                    action,
                    decision,
                    zone,
                    Some(plan.summary),
                    "export-stage-pending",
                )?;
                println!(
                    "stage export requires approval, pending id: {} (stage id: {})",
                    approval.id, stage_id
                );
            }
        }
        Verdict::Deny | Verdict::Quarantine => {
            staged_store.upsert(StagedExportRecord {
                state: StagedExportState::Failed,
                last_message: Some(decision.detail.clone()),
                ..base_record
            })?;
            append_receipt(
                &receipts,
                runtime,
                action,
                decision.clone(),
                zone,
                Some(plan.summary),
                "export-stage-denied",
            )?;
            return Err(anyhow!("export stage blocked: {}", decision.detail));
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn run_delivery(
    runtime: &RuntimeState,
    staged: &Path,
    destination: Option<&Path>,
    preview_only: bool,
    force: bool,
    move_artifact: bool,
    bypass: bool,
    bypass_ack: bool,
    actor: &str,
) -> Result<()> {
    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let engine = PolicyEngine::new(runtime.policy.clone(), runtime.config.workspace.clone());
    let staged_store = StagedExportStore::new(&runtime.config.staged_exports_file);

    let (stage_id, staged_src) = resolve_staged_reference(runtime, &staged_store, staged)?;

    let destination = match destination {
        Some(path) => normalize_delivery_path(runtime, path),
        None => default_delivery_target(runtime, &staged_src),
    };

    let plan = build_export_plan(&staged_src, &destination)?;
    println!(
        "deliver preview {} -> {}",
        staged_src.display(),
        destination.display()
    );
    print_plan_summary(&plan);
    if preview_only {
        return Ok(());
    }

    let action = build_delivery_action(
        &staged_src,
        &destination,
        actor,
        stage_id.clone(),
        move_artifact,
    );

    if bypass {
        ensure_bypass_ack(bypass_ack)?;
        apply_plan_with_mode(&plan, move_artifact)?;
        if let Some(id) = stage_id.as_deref() {
            let _ = staged_store.mark_delivered(
                id,
                &destination,
                "delivered with unsafe bypass (policy checks skipped)",
            );
        }
        append_receipt(
            &receipts,
            runtime,
            action,
            Decision {
                verdict: Verdict::Allow,
                reason: ReasonCode::AllowedByPolicy,
                detail: format!(
                    "unsafe bypass enabled: delivered directly to {}",
                    destination.display()
                ),
                approval_ttl_seconds: None,
            },
            None,
            Some(plan.summary),
            "delivery-bypass",
        )?;
        println!("Delivered to {}", destination.display());
        return Ok(());
    }

    let (decision, zone) = engine.evaluate(&action);
    match decision.verdict {
        Verdict::Allow => {
            apply_plan_with_mode(&plan, move_artifact)?;
            if let Some(id) = stage_id.as_deref() {
                let _ = staged_store.mark_delivered(id, &destination, "delivered");
            }
            append_receipt(
                &receipts,
                runtime,
                action,
                Decision {
                    verdict: Verdict::Allow,
                    reason: ReasonCode::AllowedByPolicy,
                    detail: format!("delivered to {}", destination.display()),
                    approval_ttl_seconds: None,
                },
                zone,
                Some(plan.summary),
                "delivery-commit",
            )?;
            println!("Delivered to {}", destination.display());
        }
        Verdict::RequireApproval => {
            if force || approvals.has_active_approval_for(&action)? {
                apply_plan_with_mode(&plan, move_artifact)?;
                if let Some(id) = stage_id.as_deref() {
                    let _ = staged_store.mark_delivered(
                        id,
                        &destination,
                        "delivered via explicit override/approval",
                    );
                }
                append_receipt(
                    &receipts,
                    runtime,
                    action,
                    Decision {
                        verdict: Verdict::Allow,
                        reason: ReasonCode::AllowedByPolicy,
                        detail: format!(
                            "delivered to {} via explicit override/approval",
                            destination.display()
                        ),
                        approval_ttl_seconds: None,
                    },
                    zone,
                    Some(plan.summary),
                    "delivery-approved",
                )?;
                println!("Delivered to {}", destination.display());
            } else {
                let approval =
                    approvals.create_pending(&action, &decision, "delivery requires approval")?;
                if let Some(id) = stage_id.as_deref() {
                    let _ = staged_store.mark_delivery_pending(
                        id,
                        Some(approval.id.clone()),
                        &destination,
                        "awaiting delivery approval",
                    );
                }
                append_receipt(
                    &receipts,
                    runtime,
                    action,
                    decision,
                    zone,
                    Some(plan.summary),
                    "delivery-pending",
                )?;
                println!("delivery requires approval, pending id: {}", approval.id);
            }
        }
        Verdict::Deny | Verdict::Quarantine => {
            if let Some(id) = stage_id.as_deref() {
                let _ = staged_store.mark_failed(id, decision.detail.clone());
            }
            append_receipt(
                &receipts,
                runtime,
                action,
                decision.clone(),
                zone,
                Some(plan.summary),
                "delivery-denied",
            )?;
            return Err(anyhow!("delivery blocked: {}", decision.detail));
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn run_import(
    runtime: &RuntimeState,
    src: &Path,
    dst: Option<&Path>,
    preview_only: bool,
    force: bool,
    bypass: bool,
    bypass_ack: bool,
    actor: &str,
) -> Result<()> {
    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let engine = PolicyEngine::new(runtime.policy.clone(), runtime.config.workspace.clone());

    let src = normalize_import_src(runtime, src);
    let dst = normalize_import_dst(runtime, dst, &src)?;

    let plan = build_export_plan(&src, &dst)?;
    println!("import preview {} -> {}", src.display(), dst.display());
    print_plan_summary(&plan);

    if preview_only {
        return Ok(());
    }

    let action = build_import_action(&src, &dst, actor);

    if bypass {
        ensure_bypass_ack(bypass_ack)?;
        commit_export(&plan)?;
        append_receipt(
            &receipts,
            runtime,
            action,
            Decision {
                verdict: Verdict::Allow,
                reason: ReasonCode::AllowedByPolicy,
                detail: "unsafe bypass enabled: import copied without policy evaluation"
                    .to_string(),
                approval_ttl_seconds: None,
            },
            None,
            Some(plan.summary),
            "import-bypass",
        )?;
        println!("imported to {} with bypass enabled", dst.display());
        return Ok(());
    }

    let (decision, zone) = engine.evaluate(&action);
    match decision.verdict {
        Verdict::Allow => {
            commit_export(&plan)?;
            append_receipt(
                &receipts,
                runtime,
                action,
                Decision {
                    verdict: Verdict::Allow,
                    reason: ReasonCode::AllowedByPolicy,
                    detail: format!("imported into workspace at {}", dst.display()),
                    approval_ttl_seconds: None,
                },
                zone,
                Some(plan.summary),
                "import-commit",
            )?;
            println!("imported to {}", dst.display());
        }
        Verdict::RequireApproval => {
            if force || approvals.has_active_approval_for(&action)? {
                commit_export(&plan)?;
                append_receipt(
                    &receipts,
                    runtime,
                    action,
                    Decision {
                        verdict: Verdict::Allow,
                        reason: ReasonCode::AllowedByPolicy,
                        detail: format!(
                            "imported into workspace at {} via explicit override/approval",
                            dst.display()
                        ),
                        approval_ttl_seconds: None,
                    },
                    zone,
                    Some(plan.summary),
                    "import-approved",
                )?;
                println!("imported to {}", dst.display());
            } else {
                let approval =
                    approvals.create_pending(&action, &decision, "import requires approval")?;
                append_receipt(
                    &receipts,
                    runtime,
                    action,
                    decision,
                    zone,
                    Some(plan.summary),
                    "import-pending",
                )?;
                println!("import requires approval, pending id: {}", approval.id);
            }
        }
        Verdict::Deny | Verdict::Quarantine => {
            append_receipt(
                &receipts,
                runtime,
                action,
                decision.clone(),
                zone,
                Some(plan.summary),
                "import-denied",
            )?;
            return Err(anyhow!("import blocked: {}", decision.detail));
        }
    }

    Ok(())
}

fn build_export_action(
    src: &Path,
    dst: &Path,
    actor: &str,
    stage_id: Option<String>,
) -> ActionRequest {
    ActionRequest {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        kind: ActionKind::ExportCommit,
        operation: "export_commit".to_string(),
        path: Some(dst.to_path_buf()),
        secondary_path: Some(src.to_path_buf()),
        host: None,
        metadata: {
            let mut meta = BTreeMap::new();
            meta.insert("export_src".to_string(), src.to_string_lossy().to_string());
            meta.insert("export_dst".to_string(), dst.to_string_lossy().to_string());
            if let Some(stage_id) = stage_id {
                meta.insert("stage_id".to_string(), stage_id);
            }
            meta
        },
        process: ProcessContext {
            pid: std::process::id(),
            ppid: None,
            command: actor.to_string(),
            process_tree: vec![std::process::id()],
        },
    }
}

fn build_delivery_action(
    src: &Path,
    dst: &Path,
    actor: &str,
    stage_id: Option<String>,
    move_artifact: bool,
) -> ActionRequest {
    let mut action = build_export_action(src, dst, actor, stage_id);
    action.operation = "deliver_commit".to_string();
    action
        .metadata
        .insert("move_artifact".to_string(), move_artifact.to_string());
    action
}

fn build_import_action(src: &Path, dst: &Path, actor: &str) -> ActionRequest {
    ActionRequest {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        kind: ActionKind::FileWrite,
        operation: "import_copy".to_string(),
        path: Some(src.to_path_buf()),
        secondary_path: Some(dst.to_path_buf()),
        host: None,
        metadata: {
            let mut meta = BTreeMap::new();
            meta.insert("import_src".to_string(), src.to_string_lossy().to_string());
            meta.insert("import_dst".to_string(), dst.to_string_lossy().to_string());
            meta
        },
        process: ProcessContext {
            pid: std::process::id(),
            ppid: None,
            command: actor.to_string(),
            process_tree: vec![std::process::id()],
        },
    }
}

fn normalize_workspace_path(runtime: &RuntimeState, input: &Path) -> PathBuf {
    if input.is_absolute() {
        input.to_path_buf()
    } else {
        runtime.config.workspace.join(input)
    }
}

fn normalize_stage_path(runtime: &RuntimeState, input: &Path) -> Result<PathBuf> {
    if input.is_absolute() {
        if has_parent_component(input) || !input.starts_with(&runtime.config.shared_zone_dir) {
            return Err(anyhow!(
                "stage destination must stay within shared zone: {}",
                runtime.config.shared_zone_dir.display()
            ));
        }
        Ok(input.to_path_buf())
    } else {
        ensure_relative_subpath(input, "stage destination")?;
        Ok(runtime.config.shared_zone_dir.join(input))
    }
}

fn normalize_delivery_path(runtime: &RuntimeState, input: &Path) -> PathBuf {
    if input.is_absolute() {
        input.to_path_buf()
    } else {
        runtime.config.default_delivery_dir.join(input)
    }
}

fn normalize_import_src(runtime: &RuntimeState, input: &Path) -> PathBuf {
    if input.is_absolute() {
        input.to_path_buf()
    } else {
        runtime.config.ruler_root.join(input)
    }
}

fn normalize_import_dst(runtime: &RuntimeState, dst: Option<&Path>, src: &Path) -> Result<PathBuf> {
    let dst = match dst {
        Some(dst) => {
            if dst.is_absolute() {
                dst.to_path_buf()
            } else {
                runtime.config.workspace.join(dst)
            }
        }
        None => {
            let name = src
                .file_name()
                .ok_or_else(|| anyhow!("import source has no file name"))?;
            runtime.config.workspace.join(name)
        }
    };

    if !dst.starts_with(&runtime.config.workspace) {
        return Err(anyhow!(
            "import destination must stay within workspace: {}",
            dst.display()
        ));
    }

    Ok(dst)
}

fn default_delivery_target(runtime: &RuntimeState, staged_src: &Path) -> PathBuf {
    let file_name = staged_src
        .file_name()
        .map(|f| f.to_os_string())
        .unwrap_or_else(|| "artifact.bin".into());
    runtime.config.default_delivery_dir.join(file_name)
}

fn resolve_staged_reference(
    runtime: &RuntimeState,
    staged_store: &StagedExportStore,
    staged: &Path,
) -> Result<(Option<String>, PathBuf)> {
    let staged_input = staged.to_string_lossy().to_string();

    if let Some(record) = staged_store.get(&staged_input)? {
        return Ok((Some(record.id), PathBuf::from(record.staged_path)));
    }

    if !staged.is_absolute() {
        ensure_relative_subpath(staged, "stage reference")?;
    }

    let staged_path = if staged.is_absolute() {
        staged.to_path_buf()
    } else {
        runtime.config.shared_zone_dir.join(staged)
    };

    if let Some(record) = staged_store.find_by_staged_path(&staged_path)? {
        return Ok((Some(record.id), staged_path));
    }

    Ok((None, staged_path))
}

fn apply_plan_with_mode(plan: &ExportPlan, move_artifact: bool) -> Result<()> {
    commit_export(plan)?;
    if !move_artifact {
        return Ok(());
    }

    if plan.src.is_file() {
        if plan.src.exists() {
            fs::remove_file(&plan.src)
                .with_context(|| format!("remove staged file {}", plan.src.display()))?;
        }
        return Ok(());
    }

    if plan.src.exists() {
        fs::remove_dir_all(&plan.src)
            .with_context(|| format!("remove staged directory {}", plan.src.display()))?;
    }
    Ok(())
}

fn ensure_bypass_ack(ack: bool) -> Result<()> {
    if ack {
        return Ok(());
    }
    Err(anyhow!("bypass refused: {}", BYPASS_ACK_HINT))
}

fn ensure_relative_subpath(path: &Path, label: &str) -> Result<()> {
    if path.as_os_str().is_empty() {
        return Err(anyhow!("{label} must not be empty"));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(anyhow!(
            "{label} must be a relative path without traversal segments"
        ));
    }
    Ok(())
}

fn has_parent_component(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn print_plan_summary(plan: &ExportPlan) {
    println!(
        "summary: +{} -{} ~{} bytes(+{} -{})",
        plan.summary.files_added,
        plan.summary.files_removed,
        plan.summary.files_changed,
        plan.summary.bytes_added,
        plan.summary.bytes_removed
    );
    if !plan.diff_preview.is_empty() {
        println!("{}", plan.diff_preview);
    }
}
