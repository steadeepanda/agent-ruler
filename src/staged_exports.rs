use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StagedExportState {
    PendingStageApproval,
    Staged,
    PendingDeliveryApproval,
    Delivered,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedExportRecord {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub src_workspace: String,
    pub staged_path: String,
    pub state: StagedExportState,
    pub stage_approval_id: Option<String>,
    pub delivery_approval_id: Option<String>,
    pub delivery_destination: Option<String>,
    pub delivered_to: Option<String>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub last_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StagedExportStore {
    path: PathBuf,
}

impl StagedExportStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn list(&self) -> Result<Vec<StagedExportRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let raw = fs::read_to_string(&self.path)
            .with_context(|| format!("read staged exports file {}", self.path.display()))?;
        let mut records: Vec<StagedExportRecord> = serde_json::from_str(raw.trim())
            .or_else(|_| serde_json::from_str("[]"))
            .context("parse staged exports json")?;

        records.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(records)
    }

    pub fn upsert(&self, record: StagedExportRecord) -> Result<()> {
        let mut records = self.list()?;
        let mut replaced = false;
        for item in &mut records {
            if item.id == record.id {
                *item = record.clone();
                replaced = true;
                break;
            }
        }
        if !replaced {
            records.push(record);
        }
        self.persist(&records)
    }

    pub fn get(&self, id: &str) -> Result<Option<StagedExportRecord>> {
        let records = self.list()?;
        Ok(records.into_iter().find(|r| r.id == id))
    }

    pub fn find_by_staged_path(&self, path: &Path) -> Result<Option<StagedExportRecord>> {
        let target = path.to_string_lossy().to_string();
        let records = self.list()?;
        Ok(records.into_iter().find(|r| r.staged_path == target))
    }

    pub fn mark_stage_pending(
        &self,
        id: &str,
        approval_id: Option<String>,
        message: impl Into<String>,
    ) -> Result<()> {
        self.mutate(id, |item| {
            item.state = StagedExportState::PendingStageApproval;
            item.stage_approval_id = approval_id;
            item.last_message = Some(message.into());
        })
    }

    pub fn mark_staged(&self, id: &str, message: impl Into<String>) -> Result<()> {
        self.mutate(id, |item| {
            item.state = StagedExportState::Staged;
            item.stage_approval_id = None;
            item.last_message = Some(message.into());
        })
    }

    pub fn mark_delivery_pending(
        &self,
        id: &str,
        approval_id: Option<String>,
        destination: &Path,
        message: impl Into<String>,
    ) -> Result<()> {
        let destination = destination.to_string_lossy().to_string();
        self.mutate(id, |item| {
            item.state = StagedExportState::PendingDeliveryApproval;
            item.delivery_approval_id = approval_id;
            item.delivery_destination = Some(destination.clone());
            item.last_message = Some(message.into());
        })
    }

    pub fn mark_delivered(
        &self,
        id: &str,
        destination: &Path,
        message: impl Into<String>,
    ) -> Result<()> {
        let now = Utc::now();
        let destination = destination.to_string_lossy().to_string();
        self.mutate(id, |item| {
            item.state = StagedExportState::Delivered;
            item.delivery_approval_id = None;
            item.delivery_destination = Some(destination.clone());
            item.delivered_to = Some(destination.clone());
            item.delivered_at = Some(now);
            item.last_message = Some(message.into());
        })
    }

    pub fn mark_failed(&self, id: &str, message: impl Into<String>) -> Result<()> {
        self.mutate(id, |item| {
            item.state = StagedExportState::Failed;
            item.last_message = Some(message.into());
        })
    }

    fn mutate<F>(&self, id: &str, mutator: F) -> Result<()>
    where
        F: FnOnce(&mut StagedExportRecord),
    {
        let mut records = self.list()?;
        for item in &mut records {
            if item.id == id {
                mutator(item);
                item.updated_at = Utc::now();
                return self.persist(&records);
            }
        }
        Ok(())
    }

    fn persist(&self, records: &[StagedExportRecord]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create staged export parent {}", parent.display()))?;
        }

        let payload = serde_json::to_string_pretty(records).context("serialize staged exports")?;
        let temp = self.path.with_extension("json.tmp");
        fs::write(&temp, payload).with_context(|| format!("write {}", temp.display()))?;
        fs::rename(&temp, &self.path)
            .with_context(|| format!("replace {}", self.path.display()))?;
        Ok(())
    }
}
