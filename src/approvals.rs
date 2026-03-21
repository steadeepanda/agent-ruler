//! Approval queue management for Agent Ruler.
//!
//! This module provides the approval store that manages pending, approved, denied,
//! and expired approval requests. Approvals are persisted to disk as JSON for
//! durability across restarts.
//!
//! # Key Concepts
//!
//! - **Pending**: Awaiting operator decision (via WebUI or CLI)
//! - **Approved**: Operator has approved, action can proceed
//! - **Denied**: Operator has denied, action is blocked
//! - **Expired**: TTL has passed without a decision
//!
//! # TTL (Time-To-Live)
//!
//! Each approval has a TTL (default 30 minutes). After expiration, pending
//! approvals automatically transition to expired status.
//!
//! # Scope Keys
//!
//! Approvals are scoped by a deterministic key derived from the action request.
//! This allows checking if an equivalent action has already been approved.
//!
//! # Persistence
//!
//! Approvals are stored in `approvals.json` using atomic write-rename to prevent
//! corruption if the process exits during a write.
//!
//! # Tests
//!
//! See `/tests/approvals_flow.rs` and `/tests/approvals_expiry.rs`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::{Duration, Utc};

use crate::model::{ActionRequest, ApprovalRecord, ApprovalStatus, Decision};
use crate::utils::make_scope_key;

/// Manages the approval queue with persistent storage.
///
/// Approval records are persisted as JSON to support deterministic offline
/// review and replay. The store uses atomic write-rename to prevent corruption.
#[derive(Debug, Clone)]
pub struct ApprovalStore {
    /// Path to the approvals JSON file
    path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ApprovalStatusUpdate {
    pub approval: ApprovalRecord,
    pub changed: bool,
}

impl ApprovalStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn list_all(&self) -> Result<Vec<ApprovalRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&self.path)
            .with_context(|| format!("read approvals file {}", self.path.display()))?;
        let approvals: Vec<ApprovalRecord> = serde_json::from_str(content.trim())
            .or_else(|_| serde_json::from_str("[]"))
            .context("parse approvals json")?;
        Ok(approvals)
    }

    pub fn list_pending(&self) -> Result<Vec<ApprovalRecord>> {
        let mut all = self.list_all()?;
        if self.expire_stale(&mut all) {
            self.persist(&all)?;
        }
        Ok(all
            .into_iter()
            .filter(|a| a.status == ApprovalStatus::Pending)
            .collect())
    }

    pub fn create_pending(
        &self,
        action: &ActionRequest,
        decision: &Decision,
        note: impl Into<String>,
    ) -> Result<ApprovalRecord> {
        let mut all = self.list_all()?;
        let expired = self.expire_stale(&mut all);
        if let Some(existing) = all.iter().find(|record| {
            record.status == ApprovalStatus::Pending
                && record.reason == decision.reason
                && record.action.kind == action.kind
                && record.action.operation == action.operation
                && record.action.path == action.path
                && record.action.secondary_path == action.secondary_path
                && record.action.host == action.host
        }) {
            if expired {
                self.persist(&all)?;
            }
            return Ok(existing.clone());
        }

        let now = Utc::now();
        let ttl = decision.approval_ttl_seconds.unwrap_or(1800) as i64;
        let approval = ApprovalRecord {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: now,
            expires_at: now + Duration::seconds(ttl),
            status: ApprovalStatus::Pending,
            reason: decision.reason,
            scope_key: make_scope_key(action),
            action: action.clone(),
            note: note.into(),
            decided_at: None,
        };
        all.push(approval.clone());
        self.persist(&all)?;
        Ok(approval)
    }

    pub fn approve(&self, id: &str) -> Result<ApprovalRecord> {
        self.set_status(id, ApprovalStatus::Approved)
    }

    pub fn deny(&self, id: &str) -> Result<ApprovalRecord> {
        self.set_status(id, ApprovalStatus::Denied)
    }

    pub fn approve_idempotent(&self, id: &str) -> Result<ApprovalStatusUpdate> {
        self.set_status_idempotent(id, ApprovalStatus::Approved)
    }

    pub fn deny_idempotent(&self, id: &str) -> Result<ApprovalStatusUpdate> {
        self.set_status_idempotent(id, ApprovalStatus::Denied)
    }

    pub fn get(&self, id: &str) -> Result<Option<ApprovalRecord>> {
        let mut all = self.list_all()?;
        if self.expire_stale(&mut all) {
            self.persist(&all)?;
        }
        Ok(all.into_iter().find(|a| a.id == id))
    }

    pub fn has_active_approval_for(&self, action: &ActionRequest) -> Result<bool> {
        let scope_key = make_scope_key(action);
        let mut all = self.list_all()?;
        if self.expire_stale(&mut all) {
            self.persist(&all)?;
        }

        Ok(all.iter().any(|a| {
            a.scope_key == scope_key
                && a.status == ApprovalStatus::Approved
                && a.expires_at > Utc::now()
        }))
    }

    pub fn expire_now(&self) -> Result<usize> {
        let mut all = self.list_all()?;
        let before = all.len();
        let changed = self.expire_stale(&mut all);
        if changed {
            self.persist(&all)?;
        }
        let expired = all
            .iter()
            .filter(|a| a.status == ApprovalStatus::Expired)
            .count();
        if before == 0 {
            return Ok(0);
        }
        Ok(expired)
    }

    fn set_status(&self, id: &str, target: ApprovalStatus) -> Result<ApprovalRecord> {
        let mut all = self.list_all()?;
        let mut changed = self.expire_stale(&mut all);

        let now = Utc::now();
        let mut updated = None;
        for approval in &mut all {
            if approval.id == id {
                if approval.status != ApprovalStatus::Pending {
                    return Err(anyhow!(
                        "approval {} is not pending (status: {:?})",
                        id,
                        approval.status
                    ));
                }
                approval.status = target;
                approval.decided_at = Some(now);
                updated = Some(approval.clone());
                changed = true;
                break;
            }
        }

        if changed {
            self.persist(&all)?;
        }
        updated.ok_or_else(|| anyhow!("approval {} not found", id))
    }

    fn set_status_idempotent(
        &self,
        id: &str,
        target: ApprovalStatus,
    ) -> Result<ApprovalStatusUpdate> {
        let mut all = self.list_all()?;
        let mut changed = self.expire_stale(&mut all);

        let now = Utc::now();
        let mut updated = None;
        for approval in &mut all {
            if approval.id == id {
                if approval.status == ApprovalStatus::Pending {
                    approval.status = target;
                    approval.decided_at = Some(now);
                    updated = Some(ApprovalStatusUpdate {
                        approval: approval.clone(),
                        changed: true,
                    });
                    changed = true;
                    break;
                }
                if approval.status == target {
                    updated = Some(ApprovalStatusUpdate {
                        approval: approval.clone(),
                        changed: false,
                    });
                    break;
                }
                return Err(anyhow!(
                    "approval {} is not pending (status: {:?})",
                    id,
                    approval.status
                ));
            }
        }

        if changed {
            self.persist(&all)?;
        }
        updated.ok_or_else(|| anyhow!("approval {} not found", id))
    }

    // Atomic replace avoids truncated approval state if process exits during write.
    fn persist(&self, approvals: &[ApprovalRecord]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create approvals parent {}", parent.display()))?;
        }
        let payload = serde_json::to_string_pretty(approvals).context("serialize approvals")?;
        let temp = self
            .path
            .with_extension(format!("json.tmp.{}", uuid::Uuid::new_v4()));
        fs::write(&temp, payload).with_context(|| format!("write {}", temp.display()))?;
        fs::rename(&temp, &self.path).with_context(|| {
            format!(
                "replace approvals file {} -> {}",
                temp.display(),
                self.path.display()
            )
        })?;
        Ok(())
    }

    fn expire_stale(&self, approvals: &mut [ApprovalRecord]) -> bool {
        let now = Utc::now();
        let mut changed = false;
        for approval in approvals {
            if approval.status == ApprovalStatus::Pending && approval.expires_at <= now {
                approval.status = ApprovalStatus::Expired;
                approval.decided_at = Some(now);
                changed = true;
            }
        }
        changed
    }
}
