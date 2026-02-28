//! Append-only audit log for Agent Ruler.
//!
//! This module provides the receipt store that records all policy decisions.
//! Receipts are written in JSONL format (one JSON object per line) for durability
//! and easy parsing.
//!
//! # Key Properties
//!
//! - **Append-Only**: Receipts are never modified after writing
//! - **Immutable**: No deletion or update operations provided
//! - **Ordered**: Receipts are stored in chronological order
//! - **Complete**: Every policy decision generates a receipt
//!
//! # Receipt Contents
//!
//! Each receipt contains:
//! - The original action request
//! - The policy decision (verdict, reason, detail)
//! - Zone classification
//! - Policy version and hash
//! - Optional diff summary for file operations
//!
//! # Redaction
//!
//! Sensitive fields (full command lines, diff content) may be redacted in
//! agent-visible outputs. See the `helpers` module for redaction logic.
//!
//! # Tests
//!
//! See `/tests/receipts_flow.rs`.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::model::Receipt;

/// Append-only store for audit receipts.
///
/// Receipts are stored as JSONL (JSON Lines) to preserve deterministic
/// event history ordering. Each line is a complete JSON receipt object.
#[derive(Debug, Clone)]
pub struct ReceiptStore {
    /// Path to the receipts JSONL file
    path: PathBuf,
}

impl ReceiptStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    // Append one receipt per governed action; callers choose granularity explicitly.
    pub fn append(&self, receipt: &Receipt) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create receipt parent {}", parent.display()))?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("open receipt file {}", self.path.display()))?;

        let serialized = serde_json::to_string(receipt).context("serialize receipt")?;
        writeln!(file, "{}", serialized).context("append receipt line")?;
        Ok(())
    }

    pub fn read_all(&self) -> Result<Vec<Receipt>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file = OpenOptions::new()
            .read(true)
            .open(&self.path)
            .with_context(|| format!("read receipt file {}", self.path.display()))?;
        let reader = BufReader::new(file);
        let mut output = Vec::new();
        for line in reader.lines() {
            let line = line.context("read receipt line")?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(receipt) = serde_json::from_str::<Receipt>(&line) {
                output.push(receipt);
            }
        }
        Ok(output)
    }

    pub fn tail(&self, limit: usize) -> Result<Vec<Receipt>> {
        let mut all = self.read_all()?;
        if all.len() <= limit {
            return Ok(all);
        }
        let split = all.len().saturating_sub(limit);
        Ok(all.split_off(split))
    }
}
