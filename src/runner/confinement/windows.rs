//! Windows confinement using job objects and restricted tokens.
//!
//! **Status**: Stub only. Full implementation planned.
//!
//! # Planned Implementation
//!
//! Windows confinement will use:
//! - Job objects to group and limit child processes
//! - Restricted tokens to limit privileges
//! - Mandatory Integrity Control to run at lower integrity levels
//! - File system virtualization for workspace isolation
//!
//! # Current Behavior
//!
//! On Windows, [`is_available`] always returns `false` and no confinement
//! is applied. Commands run in degraded mode if allowed by configuration.

/// Check if Windows confinement is available.
///
/// **Current**: Always returns `false` (not yet implemented).
#[allow(dead_code)]
pub fn is_available() -> bool {
    // TODO: Implement Windows job object confinement
    // This requires:
    // 1. CreateJobObject with JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    // 2. SetInformationJobObject with limit flags
    // 3. CreateRestrictedToken for privilege reduction
    // 4. Assign process to job object
    false
}

/// Build a confined command for Windows.
///
/// **Current**: Not implemented. Returns an error.
#[allow(dead_code)]
pub fn build_confined_command(
    _cmd: &[String],
    _runtime: &crate::config::RuntimeState,
    _engine: &crate::policy::PolicyEngine,
) -> anyhow::Result<std::process::Command> {
    anyhow::bail!(
        "Windows confinement not yet implemented. \
         Enable 'allow_degraded_confinement' to run without sandboxing."
    )
}
