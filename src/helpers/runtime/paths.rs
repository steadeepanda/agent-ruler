use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::config::RuntimeState;
use crate::export_gate::{commit_export, ExportPlan};
use crate::staged_exports::StagedExportStore;

const BYPASS_ACK_HINT: &str =
    "bypass refused: include bypass_ack=true to acknowledge policy bypass and reduced audit";

pub fn resolve_ui_path_update(base: &Path, raw: &str, absolute: bool) -> PathBuf {
    let path = PathBuf::from(raw.trim());
    if absolute || path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

pub fn resolve_workspace_src(runtime: &RuntimeState, src: &str) -> PathBuf {
    let path = PathBuf::from(src);
    if path.is_absolute() {
        path
    } else {
        runtime.config.workspace.join(path)
    }
}

pub fn resolve_stage_dst(runtime: &RuntimeState, dst: Option<&str>, src: &Path) -> Result<PathBuf> {
    match dst {
        Some(dst) => {
            let path = PathBuf::from(dst);
            if path.is_absolute() {
                if has_parent_component(&path) || !path.starts_with(&runtime.config.shared_zone_dir)
                {
                    return Err(anyhow!(
                        "stage destination must stay within shared zone: {}",
                        runtime.config.shared_zone_dir.display()
                    ));
                }
                Ok(path)
            } else {
                ensure_relative_subpath(&path, "stage destination")?;
                Ok(runtime.config.shared_zone_dir.join(path))
            }
        }
        None => {
            let file_name = src
                .file_name()
                .map(|f| f.to_os_string())
                .unwrap_or_else(|| "artifact.bin".into());
            Ok(runtime.config.shared_zone_dir.join(file_name))
        }
    }
}

pub fn resolve_stage_reference(
    runtime: &RuntimeState,
    staged_store: &StagedExportStore,
    stage_ref: &str,
) -> Result<(Option<String>, PathBuf)> {
    if stage_ref.trim().is_empty() {
        return Err(anyhow!("stage_ref must not be empty"));
    }

    if let Some(record) = staged_store.get(stage_ref)? {
        return Ok((Some(record.id), PathBuf::from(record.staged_path)));
    }

    let input = PathBuf::from(stage_ref);
    if !input.is_absolute() {
        ensure_relative_subpath(&input, "stage reference")?;
    }
    let staged_path = if input.is_absolute() {
        input
    } else {
        runtime.config.shared_zone_dir.join(input)
    };

    if let Some(record) = staged_store.find_by_staged_path(&staged_path)? {
        return Ok((Some(record.id), staged_path));
    }

    Ok((None, staged_path))
}

pub fn resolve_delivery_dst(
    runtime: &RuntimeState,
    dst: Option<&str>,
    staged_src: &Path,
) -> PathBuf {
    match dst {
        Some(dst) => {
            let path = PathBuf::from(dst);
            if path.is_absolute() {
                path
            } else {
                runtime.config.default_delivery_dir.join(path)
            }
        }
        None => {
            let file_name = staged_src
                .file_name()
                .map(|f| f.to_os_string())
                .unwrap_or_else(|| "artifact.bin".into());
            runtime.config.default_delivery_dir.join(file_name)
        }
    }
}

pub fn resolve_import_src(runtime: &RuntimeState, src: &str) -> PathBuf {
    let path = PathBuf::from(src);
    if path.is_absolute() {
        path
    } else {
        runtime.config.ruler_root.join(path)
    }
}

pub fn resolve_import_dst(
    runtime: &RuntimeState,
    dst: Option<&str>,
    src: &Path,
) -> Result<PathBuf> {
    let dst = match dst {
        Some(dst) => {
            let path = PathBuf::from(dst);
            if path.is_absolute() {
                path
            } else {
                runtime.config.workspace.join(path)
            }
        }
        None => {
            let file_name = src
                .file_name()
                .ok_or_else(|| anyhow!("import source has no file name"))?;
            runtime.config.workspace.join(file_name)
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

pub fn apply_plan_with_mode(plan: &ExportPlan, move_artifact: bool) -> Result<()> {
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

pub fn ensure_bypass_ack(ack: bool) -> Result<()> {
    if ack {
        return Ok(());
    }

    Err(anyhow!(BYPASS_ACK_HINT))
}

pub fn sanitize_file_name(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }

    out.trim_matches('-').to_string()
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
