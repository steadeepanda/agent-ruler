use std::fs;

use axum::extract::{Multipart, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;

use crate::approvals::ApprovalStore;
use crate::export_gate::{build_export_plan, commit_export};
use crate::helpers::ui::payloads::{DeliverPayload, ExportPayload, ImportPayload};
use crate::helpers::{
    apply_plan_with_mode, build_delivery_action, build_export_action, build_import_action,
    new_stage_record, resolve_delivery_dst, resolve_import_dst, resolve_import_src,
    resolve_stage_dst, resolve_stage_reference, resolve_workspace_src, sanitize_file_name,
};
use crate::model::{Decision, ReasonCode, Verdict};
use crate::policy::PolicyEngine;
use crate::receipts::ReceiptStore;
use crate::runner::append_receipt;
use crate::staged_exports::{StagedExportRecord, StagedExportState, StagedExportStore};
use crate::ui::{error_response, load_runtime_from_state, WebState};

const CONTROL_PANEL_AUTO_APPROVE_ORIGIN: &str = "control_panel_user";

fn control_panel_auto_approve_requested(enabled: Option<bool>, origin: Option<&str>) -> bool {
    if !enabled.unwrap_or(false) {
        return false;
    }
    origin
        .map(str::trim)
        .map(|value| value.eq_ignore_ascii_case(CONTROL_PANEL_AUTO_APPROVE_ORIGIN))
        .unwrap_or(false)
}

pub async fn api_staged_exports(State(state): State<WebState>) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let staged = StagedExportStore::new(&runtime.config.staged_exports_file);
    let data = staged.list().unwrap_or_default();
    (StatusCode::OK, Json(data)).into_response()
}

pub async fn api_export_preview(
    State(state): State<WebState>,
    Json(payload): Json<ExportPayload>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let src = resolve_workspace_src(&runtime, &payload.src);
    let dst = match resolve_stage_dst(&runtime, payload.dst.as_deref(), &src) {
        Ok(dst) => dst,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };
    match build_export_plan(&src, &dst) {
        Ok(plan) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "src": plan.src,
                "dst": plan.dst,
                "summary": plan.summary,
                "diff_preview": plan.diff_preview,
            })),
        )
            .into_response(),
        Err(err) => error_response(StatusCode::BAD_REQUEST, err.to_string()),
    }
}

pub async fn api_export_request(
    State(state): State<WebState>,
    Json(payload): Json<ExportPayload>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let staged_store = StagedExportStore::new(&runtime.config.staged_exports_file);
    let engine = PolicyEngine::new(runtime.policy.clone(), runtime.config.workspace.clone());

    let src = resolve_workspace_src(&runtime, &payload.src);
    let dst = match resolve_stage_dst(&runtime, payload.dst.as_deref(), &src) {
        Ok(dst) => dst,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };
    let plan = match build_export_plan(&src, &dst) {
        Ok(plan) => plan,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let stage_id = uuid::Uuid::new_v4().to_string();
    let action = build_export_action(&src, &dst, "ui-export", Some(stage_id.clone()));
    let base_record = new_stage_record(&stage_id, &src, &dst);

    if payload.bypass.unwrap_or(false) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "unsafe bypass is CLI-only and unavailable from WebUI/API".to_string(),
        );
    }

    let (decision, zone) = engine.evaluate(&action);
    match decision.verdict {
        Verdict::Allow => {
            if let Err(err) = commit_export(&plan) {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
            }
            let _ = staged_store.upsert(StagedExportRecord {
                state: StagedExportState::Staged,
                last_message: Some("staged and ready for delivery".to_string()),
                ..base_record
            });
            let _ = append_receipt(
                &receipts,
                &runtime,
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
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "staged",
                    "stage_id": stage_id,
                    "stage_state": "staged",
                    "message": format!("Staged to {}", dst.display()),
                })),
            )
                .into_response()
        }
        Verdict::RequireApproval => {
            if control_panel_auto_approve_requested(
                payload.auto_approve,
                payload.auto_approve_origin.as_deref(),
            ) {
                if let Err(err) = commit_export(&plan) {
                    return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
                }
                let _ = staged_store.upsert(StagedExportRecord {
                    state: StagedExportState::Staged,
                    last_message: Some("staged by user Control Panel confirmation".to_string()),
                    ..base_record
                });
                let _ = append_receipt(
                    &receipts,
                    &runtime,
                    action,
                    Decision {
                        verdict: Verdict::Allow,
                        reason: ReasonCode::AllowedByPolicy,
                        detail: "user confirmed stage in Control Panel; approval queue skipped"
                            .to_string(),
                        approval_ttl_seconds: None,
                    },
                    zone,
                    Some(plan.summary),
                    "export-stage-user-confirmed",
                );
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "staged",
                        "stage_id": stage_id,
                        "stage_state": "staged",
                        "message": format!("Staged to {}", dst.display()),
                    })),
                )
                    .into_response()
            } else {
                match approvals.create_pending(&action, &decision, "stage export requires approval")
                {
                    Ok(approval) => {
                        let _ = staged_store.upsert(StagedExportRecord {
                            state: StagedExportState::PendingStageApproval,
                            stage_approval_id: Some(approval.id.clone()),
                            last_message: Some("awaiting stage approval".to_string()),
                            ..base_record
                        });
                        let _ = append_receipt(
                            &receipts,
                            &runtime,
                            action,
                            decision,
                            zone,
                            Some(plan.summary),
                            "export-stage-pending",
                        );
                        (
                            StatusCode::ACCEPTED,
                            Json(serde_json::json!({
                                "status": "pending_approval",
                                "approval_id": approval.id,
                                "stage_id": stage_id,
                                "stage_state": "pending_stage_approval",
                                "message": "Export staged request queued; awaiting approval",
                            })),
                        )
                            .into_response()
                    }
                    Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
                }
            }
        }
        Verdict::Deny | Verdict::Quarantine => {
            let _ = staged_store.upsert(StagedExportRecord {
                state: StagedExportState::Failed,
                last_message: Some(decision.detail.clone()),
                ..base_record
            });
            let _ = append_receipt(
                &receipts,
                &runtime,
                action,
                decision.clone(),
                zone,
                Some(plan.summary),
                "export-stage-denied",
            );
            (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "status": "blocked",
                    "reason": decision.reason,
                    "detail": decision.detail,
                })),
            )
                .into_response()
        }
    }
}

pub async fn api_deliver_preview(
    State(state): State<WebState>,
    Json(payload): Json<DeliverPayload>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let staged_store = StagedExportStore::new(&runtime.config.staged_exports_file);
    let (stage_id, staged_src) =
        match resolve_stage_reference(&runtime, &staged_store, &payload.stage_ref) {
            Ok(value) => value,
            Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
        };

    let dst = resolve_delivery_dst(&runtime, payload.dst.as_deref(), &staged_src);
    match build_export_plan(&staged_src, &dst) {
        Ok(plan) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "stage_id": stage_id,
                "src": plan.src,
                "dst": plan.dst,
                "summary": plan.summary,
                "diff_preview": plan.diff_preview,
            })),
        )
            .into_response(),
        Err(err) => error_response(StatusCode::BAD_REQUEST, err.to_string()),
    }
}

pub async fn api_deliver_request(
    State(state): State<WebState>,
    Json(payload): Json<DeliverPayload>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let staged_store = StagedExportStore::new(&runtime.config.staged_exports_file);
    let engine = PolicyEngine::new(runtime.policy.clone(), runtime.config.workspace.clone());

    let (stage_id, staged_src) =
        match resolve_stage_reference(&runtime, &staged_store, &payload.stage_ref) {
            Ok(value) => value,
            Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
        };
    let dst = resolve_delivery_dst(&runtime, payload.dst.as_deref(), &staged_src);

    let plan = match build_export_plan(&staged_src, &dst) {
        Ok(plan) => plan,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let move_artifact = payload.move_artifact.unwrap_or(false);
    let action = build_delivery_action(
        &staged_src,
        &dst,
        "ui-deliver",
        stage_id.clone(),
        move_artifact,
    );

    if payload.bypass.unwrap_or(false) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "unsafe bypass is CLI-only and unavailable from WebUI/API".to_string(),
        );
    }

    let (decision, zone) = engine.evaluate(&action);
    match decision.verdict {
        Verdict::Allow => {
            if let Err(err) = apply_plan_with_mode(&plan, move_artifact) {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
            }
            if let Some(stage_id) = stage_id.as_deref() {
                let _ = staged_store.mark_delivered(stage_id, &dst, "delivered");
            }
            let _ = append_receipt(
                &receipts,
                &runtime,
                action,
                Decision {
                    verdict: Verdict::Allow,
                    reason: ReasonCode::AllowedByPolicy,
                    detail: format!("Delivered to {}", dst.display()),
                    approval_ttl_seconds: None,
                },
                zone,
                Some(plan.summary),
                "delivery-commit",
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "delivered",
                    "stage_id": stage_id,
                    "message": format!("Delivered to {}", dst.display()),
                })),
            )
                .into_response()
        }
        Verdict::RequireApproval => {
            if control_panel_auto_approve_requested(
                payload.auto_approve,
                payload.auto_approve_origin.as_deref(),
            ) {
                if let Err(err) = apply_plan_with_mode(&plan, move_artifact) {
                    return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
                }
                if let Some(stage_id) = stage_id.as_deref() {
                    let _ = staged_store.mark_delivered(
                        stage_id,
                        &dst,
                        "delivered by user Control Panel confirmation",
                    );
                }
                let _ = append_receipt(
                    &receipts,
                    &runtime,
                    action,
                    Decision {
                        verdict: Verdict::Allow,
                        reason: ReasonCode::AllowedByPolicy,
                        detail: format!(
                            "user confirmed delivery in Control Panel; delivered to {}",
                            dst.display()
                        ),
                        approval_ttl_seconds: None,
                    },
                    zone,
                    Some(plan.summary),
                    "delivery-user-confirmed",
                );
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "delivered",
                        "stage_id": stage_id,
                        "message": format!("Delivered to {}", dst.display()),
                    })),
                )
                    .into_response()
            } else {
                match approvals.create_pending(&action, &decision, "delivery requires approval") {
                    Ok(approval) => {
                        if let Some(stage_id) = stage_id.as_deref() {
                            let _ = staged_store.mark_delivery_pending(
                                stage_id,
                                Some(approval.id.clone()),
                                &dst,
                                "awaiting delivery approval",
                            );
                        }
                        let _ = append_receipt(
                            &receipts,
                            &runtime,
                            action,
                            decision,
                            zone,
                            Some(plan.summary),
                            "delivery-pending",
                        );
                        (
                            StatusCode::ACCEPTED,
                            Json(serde_json::json!({
                                "status": "pending_approval",
                                "approval_id": approval.id,
                                "stage_id": stage_id,
                                "message": "Delivery request queued; awaiting approval",
                            })),
                        )
                            .into_response()
                    }
                    Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
                }
            }
        }
        Verdict::Deny | Verdict::Quarantine => {
            if let Some(stage_id) = stage_id.as_deref() {
                let _ = staged_store.mark_failed(stage_id, decision.detail.clone());
            }
            let _ = append_receipt(
                &receipts,
                &runtime,
                action,
                decision.clone(),
                zone,
                Some(plan.summary),
                "delivery-denied",
            );
            (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "status": "blocked",
                    "reason": decision.reason,
                    "detail": decision.detail,
                })),
            )
                .into_response()
        }
    }
}

pub async fn api_import_preview(
    State(state): State<WebState>,
    Json(payload): Json<ImportPayload>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let src = resolve_import_src(&runtime, &payload.src);
    let dst = match resolve_import_dst(&runtime, payload.dst.as_deref(), &src) {
        Ok(dst) => dst,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    match build_export_plan(&src, &dst) {
        Ok(plan) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "src": plan.src,
                "dst": plan.dst,
                "summary": plan.summary,
                "diff_preview": plan.diff_preview,
            })),
        )
            .into_response(),
        Err(err) => error_response(StatusCode::BAD_REQUEST, err.to_string()),
    }
}

pub async fn api_import_upload(
    State(state): State<WebState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let upload_dir = runtime.config.state_dir.join("import-uploads");
    if let Err(err) = fs::create_dir_all(&upload_dir) {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("create import upload dir failed: {err}"),
        );
    }

    let field = match multipart.next_field().await {
        Ok(Some(field)) => field,
        Ok(None) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "multipart payload must include one file field".to_string(),
            )
        }
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("multipart read failed: {err}"),
            )
        }
    };

    let original_name = field
        .file_name()
        .map(sanitize_file_name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "imported-file.bin".to_string());

    let bytes = match field.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("read uploaded field failed: {err}"),
            )
        }
    };

    let unique_name = format!(
        "{}-{}",
        Utc::now().timestamp_millis(),
        sanitize_file_name(&original_name)
    );
    let upload_path = upload_dir.join(unique_name);

    if let Err(err) = fs::write(&upload_path, &bytes) {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("persist upload failed: {err}"),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "uploaded_src": upload_path.to_string_lossy(),
            "suggested_dst": original_name,
            "bytes": bytes.len(),
        })),
    )
        .into_response()
}

pub async fn api_import_request(
    State(state): State<WebState>,
    Json(payload): Json<ImportPayload>,
) -> impl IntoResponse {
    let runtime = match load_runtime_from_state(&state) {
        Ok(runtime) => runtime,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let approvals = ApprovalStore::new(&runtime.config.approvals_file);
    let receipts = ReceiptStore::new(&runtime.config.receipts_file);
    let engine = PolicyEngine::new(runtime.policy.clone(), runtime.config.workspace.clone());

    let src = resolve_import_src(&runtime, &payload.src);
    let dst = match resolve_import_dst(&runtime, payload.dst.as_deref(), &src) {
        Ok(dst) => dst,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let plan = match build_export_plan(&src, &dst) {
        Ok(plan) => plan,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let action = build_import_action(&src, &dst, "ui-import");

    if payload.bypass.unwrap_or(false) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "unsafe bypass is CLI-only and unavailable from WebUI/API".to_string(),
        );
    }

    let (decision, zone) = engine.evaluate(&action);
    match decision.verdict {
        Verdict::Allow => {
            if let Err(err) = commit_export(&plan) {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
            }
            let _ = append_receipt(
                &receipts,
                &runtime,
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
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "completed",
                    "message": format!("Imported to {}", dst.display()),
                })),
            )
                .into_response()
        }
        Verdict::RequireApproval => {
            if control_panel_auto_approve_requested(
                payload.auto_approve,
                payload.auto_approve_origin.as_deref(),
            ) {
                if let Err(err) = commit_export(&plan) {
                    return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
                }
                let _ = append_receipt(
                    &receipts,
                    &runtime,
                    action,
                    Decision {
                        verdict: Verdict::Allow,
                        reason: ReasonCode::AllowedByPolicy,
                        detail: format!(
                            "user confirmed import in Control Panel; imported to {}",
                            dst.display()
                        ),
                        approval_ttl_seconds: None,
                    },
                    zone,
                    Some(plan.summary),
                    "import-user-confirmed",
                );
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "completed",
                        "message": format!("Imported to {}", dst.display()),
                    })),
                )
                    .into_response()
            } else {
                match approvals.create_pending(&action, &decision, "import requires approval") {
                    Ok(approval) => {
                        let _ = append_receipt(
                            &receipts,
                            &runtime,
                            action,
                            decision,
                            zone,
                            Some(plan.summary),
                            "import-pending",
                        );
                        (
                            StatusCode::ACCEPTED,
                            Json(serde_json::json!({
                                "status": "pending_approval",
                                "approval_id": approval.id,
                                "message": "Import request queued; awaiting approval",
                            })),
                        )
                            .into_response()
                    }
                    Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
                }
            }
        }
        Verdict::Deny | Verdict::Quarantine => {
            let _ = append_receipt(
                &receipts,
                &runtime,
                action,
                decision.clone(),
                zone,
                Some(plan.summary),
                "import-denied",
            );
            (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "status": "blocked",
                    "reason": decision.reason,
                    "detail": decision.detail,
                })),
            )
                .into_response()
        }
    }
}
