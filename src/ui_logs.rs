//! Append-only UI/operator event logs for Control Panel visibility.
//!
//! This store is intentionally separate from policy receipts:
//! - receipts capture governed action decisions,
//! - UI logs capture operator-facing status/error/warning events.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiLogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: String,
    pub source: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<JsonValue>,
}

#[derive(Debug, Clone)]
pub struct UiLogStore {
    path: PathBuf,
}

impl UiLogStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn append_event(
        &self,
        level: impl Into<String>,
        source: impl Into<String>,
        message: impl Into<String>,
        details: Option<JsonValue>,
    ) -> Result<()> {
        let entry = UiLogEntry {
            timestamp: Utc::now(),
            level: level.into(),
            source: source.into(),
            message: message.into(),
            details,
        };
        self.append(&entry)
    }

    pub fn append(&self, entry: &UiLogEntry) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create ui-log parent {}", parent.display()))?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("open ui-log file {}", self.path.display()))?;

        let serialized = serde_json::to_string(entry).context("serialize ui-log entry")?;
        writeln!(file, "{}", serialized).context("append ui-log line")?;
        Ok(())
    }

    pub fn read_all(&self) -> Result<Vec<UiLogEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file = OpenOptions::new()
            .read(true)
            .open(&self.path)
            .with_context(|| format!("read ui-log file {}", self.path.display()))?;
        let reader = BufReader::new(file);
        let mut output = Vec::new();
        for line in reader.lines() {
            let line = line.context("read ui-log line")?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<UiLogEntry>(&line) {
                output.push(entry);
            }
        }
        Ok(output)
    }
}
