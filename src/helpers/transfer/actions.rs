use std::collections::BTreeMap;
use std::path::Path;

use chrono::Utc;

use crate::model::{ActionKind, ActionRequest, ProcessContext};
use crate::staged_exports::{StagedExportRecord, StagedExportState};

pub fn build_export_action(
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

pub fn build_delivery_action(
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

pub fn build_import_action(src: &Path, dst: &Path, actor: &str) -> ActionRequest {
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

pub fn new_stage_record(stage_id: &str, src: &Path, dst: &Path) -> StagedExportRecord {
    StagedExportRecord {
        id: stage_id.to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        src_workspace: src.to_string_lossy().to_string(),
        staged_path: dst.to_string_lossy().to_string(),
        state: StagedExportState::Failed,
        stage_approval_id: None,
        delivery_approval_id: None,
        delivery_destination: None,
        delivered_to: None,
        delivered_at: None,
        last_message: None,
    }
}
