use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use similar::TextDiff;
use walkdir::WalkDir;

use crate::model::DiffSummary;

#[derive(Debug, Clone)]
pub struct ExportPlan {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub summary: DiffSummary,
    pub diff_preview: String,
}

// Build a lightweight diff summary so export approvals can be decided without scanning unrelated files.
pub fn build_export_plan(src: &Path, dst: &Path) -> Result<ExportPlan> {
    if !src.exists() {
        return Err(anyhow!("export source does not exist: {}", src.display()));
    }

    let (summary, preview) = if src.is_file() {
        let src_body = fs::read_to_string(src).unwrap_or_else(|_| String::new());
        let dst_body = if dst.exists() {
            fs::read_to_string(dst).unwrap_or_else(|_| String::new())
        } else {
            String::new()
        };

        let diff = TextDiff::from_lines(&dst_body, &src_body);
        let mut preview = String::new();
        let mut summary = DiffSummary::default();

        if dst.exists() {
            summary.files_changed = 1;
        } else {
            summary.files_added = 1;
        }

        for change in diff.iter_all_changes() {
            match change.tag() {
                similar::ChangeTag::Delete => {
                    summary.bytes_removed += change.value().len() as u64;
                }
                similar::ChangeTag::Insert => {
                    summary.bytes_added += change.value().len() as u64;
                }
                similar::ChangeTag::Equal => {}
            }

            preview.push_str(match change.tag() {
                similar::ChangeTag::Delete => "-",
                similar::ChangeTag::Insert => "+",
                similar::ChangeTag::Equal => " ",
            });
            preview.push_str(change.value());
        }

        (summary, preview)
    } else {
        let mut summary = DiffSummary::default();
        let mut preview = String::new();

        for entry in WalkDir::new(src) {
            let entry = entry.context("walk source tree")?;
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(src)
                .context("strip source prefix")?;
            let dst_file = dst.join(rel);
            let src_len = entry.metadata().map(|m| m.len()).unwrap_or(0);

            if !dst_file.exists() {
                summary.files_added += 1;
                summary.bytes_added += src_len;
                preview.push_str(&format!("+ {}\n", rel.display()));
            } else {
                let dst_len = dst_file.metadata().map(|m| m.len()).unwrap_or(0);
                if dst_len != src_len {
                    summary.files_changed += 1;
                    if src_len >= dst_len {
                        summary.bytes_added += src_len - dst_len;
                    } else {
                        summary.bytes_removed += dst_len - src_len;
                    }
                    preview.push_str(&format!("~ {}\n", rel.display()));
                }
            }
        }

        if dst.exists() {
            for entry in WalkDir::new(dst) {
                let entry = entry.context("walk destination tree")?;
                if !entry.file_type().is_file() {
                    continue;
                }
                let rel = entry
                    .path()
                    .strip_prefix(dst)
                    .context("strip destination prefix")?;
                if !src.join(rel).exists() {
                    summary.files_removed += 1;
                    summary.bytes_removed += entry.metadata().map(|m| m.len()).unwrap_or(0);
                    preview.push_str(&format!("- {}\n", rel.display()));
                }
            }
        }

        (summary, preview)
    };

    Ok(ExportPlan {
        src: src.to_path_buf(),
        dst: dst.to_path_buf(),
        summary,
        diff_preview: preview,
    })
}

// Commit is intentionally separate from planning so approval flows can persist and replay the exact operation.
pub fn commit_export(plan: &ExportPlan) -> Result<()> {
    if plan.src.is_file() {
        if let Some(parent) = plan.dst.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create export parent {}", parent.display()))?;
        }
        fs::copy(&plan.src, &plan.dst).with_context(|| {
            format!(
                "copy export file {} -> {}",
                plan.src.display(),
                plan.dst.display()
            )
        })?;
        return Ok(());
    }

    copy_dir_recursive(&plan.src, &plan.dst)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("create destination {}", dst.display()))?;

    for entry in WalkDir::new(src) {
        let entry = entry.context("walk src for copy")?;
        let rel = entry
            .path()
            .strip_prefix(src)
            .context("strip source prefix")?;
        let out_path = dst.join(rel);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&out_path)
                .with_context(|| format!("create directory {}", out_path.display()))?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create parent {}", parent.display()))?;
        }

        fs::copy(entry.path(), &out_path).with_context(|| {
            format!("copy {} -> {}", entry.path().display(), out_path.display())
        })?;
    }

    Ok(())
}
