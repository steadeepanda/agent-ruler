//! macOS confinement using sandbox-exec or Seatbelt.
//!
//! **Status**: Stub only. Full implementation planned.
//!
//! # Planned Implementation
//!
//! macOS confinement will use:
//! - `sandbox-exec` for process sandboxing
//! - Seatbelt profiles for fine-grained access control
//! - Path-based rules for filesystem access
//! - Network rules for egress control
//!
//! # Current Behavior
//!
//! On macOS, [`is_available`] always returns `false` and no confinement
//! is applied. Commands run in degraded mode if allowed by configuration.

/// Check if macOS confinement is available.
///
/// **Current**: Always returns `false` (not yet implemented).
#[allow(dead_code)]
pub fn is_available() -> bool {
    // TODO: Implement macOS sandbox-exec confinement
    // This requires:
    // 1. Generate a Seatbelt profile from policy rules
    // 2. Execute via `sandbox-exec -p <profile> -- <command>`
    // 3. Handle profile syntax for:
    //    - (allow file-read* ...) for readable paths
    //    - (allow file-write* ...) for writable paths
    //    - (deny network) for network isolation
    false
}

/// Build a confined command for macOS.
///
/// **Current**: Not implemented. Returns an error.
#[allow(dead_code)]
pub fn build_confined_command(
    _cmd: &[String],
    _runtime: &crate::config::RuntimeState,
    _engine: &crate::policy::PolicyEngine,
) -> anyhow::Result<std::process::Command> {
    anyhow::bail!(
        "macOS confinement not yet implemented. \
         Enable 'allow_degraded_confinement' to run without sandboxing."
    )
}
