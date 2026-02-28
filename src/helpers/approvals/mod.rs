//! Approval workflow helper surface used by CLI and WebUI.
//!
//! These helpers keep approval side effects (receipts, redacted feed shaping,
//! post-approval operations) consistent across interfaces.

pub mod effects;
pub mod status;
pub mod utils;
pub mod views;

pub use effects::maybe_apply_approval_effect;
pub use status::redacted_status_event;
pub use utils::{
    append_approval_resolution_receipt, append_bulk_approval_resolution_receipt,
    approval_diff_summary, approval_paths, approval_status_slug, reason_code_slug, zone_slug,
};
pub use views::{approval_to_view, reason_help};
