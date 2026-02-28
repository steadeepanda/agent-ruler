//! Platform-specific confinement for command execution.
//!
//! This module provides sandboxing for executed commands using platform-specific
//! isolation mechanisms. Each platform has its own submodule with a conforming
//! implementation.
//!
//! # Platform Support
//!
//! - **Linux**: Full support via bubblewrap (bwrap). See [`linux`] module.
//! - **Windows**: Stub only. Planned support via job objects and restricted tokens.
//! - **macOS**: Stub only. Planned support via sandbox-exec or Seatbelt.
//!
//! # Security Model
//!
//! All confinement implementations should provide:
//! - Filesystem isolation (limited read/write access)
//! - Network isolation (when policy requires)
//! - Secret masking (prevent credential access)
//! - Process isolation (prevent access to host processes)
//!
//! # Degraded Mode
//!
//! If confinement is unavailable or fails, commands can run in "degraded" mode
//! without sandboxing. This is controlled by `allow_degraded_confinement` in
//! the configuration. Degraded mode is logged prominently as a security warning.
//!
//! # Adding New Platforms
//!
//! To add a new platform:
//! 1. Create a new submodule (e.g., `windows.rs` or `macos.rs`)
//! 2. Implement the platform-specific confinement functions
//! 3. Add conditional compilation in this module
//! 4. Update the platform detection in [`platform_confinement_available`]

mod common;

// Keep platform modules in the tree so rust-analyzer can index them even when
// compiling on a different host OS.
mod linux;

mod windows;

mod macos;

pub use common::{is_confinement_env_error, ConfinementBackend};

#[cfg(target_os = "linux")]
pub use linux::{
    build_confined_command as build_bwrap_command, is_available as is_linux_available,
};

#[cfg(target_os = "windows")]
pub use windows::is_available as is_windows_available;

#[cfg(target_os = "macos")]
pub use macos::is_available as is_macos_available;

/// Check if confinement is available on the current platform.
///
/// Returns the backend type if available, or None if confinement is not supported
/// or the required tools are not installed.
#[allow(dead_code)]
pub fn platform_confinement_available() -> Option<ConfinementBackend> {
    #[cfg(target_os = "linux")]
    {
        if is_linux_available() {
            return Some(ConfinementBackend::Bubblewrap);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if is_windows_available() {
            return Some(ConfinementBackend::JobObject);
        }
    }

    #[cfg(target_os = "macos")]
    {
        if is_macos_available() {
            return Some(ConfinementBackend::SandboxExec);
        }
    }

    None
}

/// Get the name of the current platform's confinement backend.
#[allow(dead_code)]
pub fn backend_name() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        if is_linux_available() {
            return "linux-bwrap";
        }
    }

    #[cfg(target_os = "windows")]
    {
        if is_windows_available() {
            return "windows-job-object";
        }
    }

    #[cfg(target_os = "macos")]
    {
        if is_macos_available() {
            return "macos-sandbox-exec";
        }
    }

    "degraded"
}
