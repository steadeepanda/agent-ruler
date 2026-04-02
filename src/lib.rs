//! Agent Ruler - Security Policy Engine for AI Agents
//!
//! This crate provides a deterministic policy enforcement layer for AI agents,
//! controlling file system access, network egress, command execution, and data exports.
//!
//! # Architecture Overview
//!
//! The crate is organized into several key modules:
//!
//! - [`model`] - Core data types: `ActionRequest`, `Decision`, `Zone`, `Verdict`, `Receipt`
//! - [`config`] - Configuration loading, policy structures, runtime state
//! - [`policy`] - Policy evaluation engine (deterministic, no LLM logic)
//! - [`approvals`] - Pending approval queue with TTL support
//! - [`receipts`] - Append-only audit log
//! - [`runner`] - Command execution with confinement (Linux bubblewrap)
//! - [`export_gate`] - Staged export flow with approval gates
//! - [`staged_exports`] - Tracking of pending exports
//! - [`ui`] - Web UI server and API endpoints
//! - [`helpers`] - Shared utilities for UI, CLI, and policy
//! - [`utils`] - Common helper functions
//!
//! # Security Invariants
//!
//! 1. **Determinism**: All policy decisions are deterministic based on request
//!    inputs and policy configuration. No LLM or probabilistic logic in enforcement.
//! 2. **Default Deny**: Unknown operations default to deny or require approval.
//! 3. **System Critical Guard**: Paths like `/etc`, `/usr`, `/bin` are always denied.
//! 4. **Approval Gates**: Sensitive operations require explicit operator approval.
//! 5. **Audit Trail**: All decisions are logged to append-only receipts.
//!
//! # Key Flows
//!
//! ```text
//! ActionRequest -> PolicyEngine::evaluate() -> Decision
//!            \
//!             -> Zone classification -> Rule lookup -> Verdict
//! ```
//!
//! Tests are located in `/tests/` and cover policy evaluation, approval flows,
//! confinement, and OWASP scenarios.

pub mod approvals;
pub mod claudecode_bridge;
pub mod config;
pub mod doctor;
pub mod embedded_bridge;
pub mod export_gate;
pub mod helpers;
pub mod model;
pub mod openclaw_bridge;
pub mod opencode_bridge;
pub mod policy;
pub mod receipts;
pub mod runner;
pub mod runners;
pub mod sessions;
pub mod staged_exports;
pub mod ui;
pub mod ui_logs;
pub mod utils;
