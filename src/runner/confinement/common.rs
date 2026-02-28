//! Common confinement utilities shared across platforms.
//!
//! This module provides:
//! - The [`ConfinementBackend`] enum for identifying sandbox types
//! - Unconfined execution fallback for degraded mode
//! - Error detection helpers

/// Supported confinement backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ConfinementBackend {
    /// Linux bubblewrap (bwrap) namespace isolation
    Bubblewrap,
    /// Windows job objects and restricted tokens (planned)
    JobObject,
    /// macOS sandbox-exec / Seatbelt (planned)
    SandboxExec,
}

impl ConfinementBackend {
    /// Get a human-readable name for this backend.
    #[allow(dead_code)]
    pub fn name(&self) -> &'static str {
        match self {
            ConfinementBackend::Bubblewrap => "linux-bwrap",
            ConfinementBackend::JobObject => "windows-job-object",
            ConfinementBackend::SandboxExec => "macos-sandbox-exec",
        }
    }
}

/// Check if a stderr output indicates a confinement environment error.
///
/// These errors typically occur when:
/// - Namespace creation fails (Linux)
/// - Privilege escalation is blocked
/// - Required kernel features are unavailable
///
/// When such errors are detected, the system may fall back to degraded mode.
pub fn is_confinement_env_error(stderr: &str) -> bool {
    stderr.contains("Operation not permitted")
        || stderr.contains("Failed RTM_NEWADDR")
        || stderr.contains("setting up uid map")
        || stderr.contains("uid map")
        || stderr.contains("bubblewrap")
        || stderr.contains("setns")
}
