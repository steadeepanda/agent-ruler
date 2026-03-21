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
    let lowered = stderr.to_ascii_lowercase();
    lowered.contains("operation not permitted")
        || lowered.contains("failed rtm_newaddr")
        || lowered.contains("setting up uid map")
        || lowered.contains("uid map")
        || lowered.contains("bubblewrap")
        || lowered.contains("bwrap:")
        || lowered.contains("setns")
        || (lowered.contains("bwrap") && lowered.contains("permission denied"))
}

#[cfg(test)]
mod tests {
    use super::is_confinement_env_error;

    #[test]
    fn detects_uppercase_uid_map_variant() {
        assert!(is_confinement_env_error(
            "bwrap: setting up UID map: Permission denied"
        ));
    }

    #[test]
    fn detects_generic_bwrap_permission_denied_variant() {
        assert!(is_confinement_env_error(
            "bwrap: Permission denied while configuring namespace"
        ));
    }
}
