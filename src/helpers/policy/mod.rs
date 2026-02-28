pub mod profiles;

pub use profiles::{
    apply_profile_preset, canonical_profile_id, enforce_minimum_safety_guards,
    enforce_system_critical_guard, is_supported_profile, normalize_profile_for_display,
    policy_profiles, profile_allows_rule_customization, profile_permissions, ProfilePermissions,
};
