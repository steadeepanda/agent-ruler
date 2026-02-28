use anyhow::{anyhow, Result};

use crate::config::{Policy, RuleDisposition};
use crate::helpers::ui::payloads::PolicyProfile;

pub const PROFILE_STRICT: &str = "strict";
pub const PROFILE_SIMPLE_USER: &str = "simple_user";
pub const PROFILE_BALANCED: &str = "balanced";
pub const PROFILE_CODING_NERD: &str = "coding_nerd";
pub const PROFILE_I_DONT_CARE: &str = "i_dont_care";
pub const PROFILE_CUSTOM: &str = "custom";
pub const PROFILE_USER_CUSTOM_LEGACY: &str = "user_custom";

const DEFAULT_MASS_DELETE_THRESHOLD: usize = 40;
const OPEN_MASS_DELETE_THRESHOLD: usize = 400;

#[derive(Debug, Clone, Copy)]
pub struct ProfilePermissions {
    pub allow_network_customization: bool,
    pub allow_domain_customization: bool,
    pub allow_elevation_customization: bool,
    pub allow_rule_customization: bool,
    pub can_create_custom_profile: bool,
}

pub fn canonical_profile_id(profile: &str) -> Option<&'static str> {
    match profile {
        PROFILE_STRICT => Some(PROFILE_STRICT),
        PROFILE_SIMPLE_USER => Some(PROFILE_SIMPLE_USER),
        PROFILE_BALANCED => Some(PROFILE_BALANCED),
        PROFILE_CODING_NERD => Some(PROFILE_CODING_NERD),
        PROFILE_I_DONT_CARE => Some(PROFILE_I_DONT_CARE),
        PROFILE_CUSTOM | PROFILE_USER_CUSTOM_LEGACY => Some(PROFILE_CUSTOM),
        _ => None,
    }
}

pub fn normalize_profile_for_display(profile: &str) -> &str {
    canonical_profile_id(profile).unwrap_or(PROFILE_STRICT)
}

pub fn is_supported_profile(profile: &str) -> bool {
    canonical_profile_id(profile).is_some()
}

pub fn profile_permissions(profile: &str) -> ProfilePermissions {
    match normalize_profile_for_display(profile) {
        PROFILE_STRICT => ProfilePermissions {
            allow_network_customization: true,
            allow_domain_customization: true,
            allow_elevation_customization: false,
            allow_rule_customization: false,
            can_create_custom_profile: true,
        },
        PROFILE_SIMPLE_USER => ProfilePermissions {
            allow_network_customization: true,
            allow_domain_customization: true,
            allow_elevation_customization: false,
            allow_rule_customization: false,
            can_create_custom_profile: true,
        },
        PROFILE_BALANCED => ProfilePermissions {
            allow_network_customization: true,
            allow_domain_customization: true,
            allow_elevation_customization: true,
            allow_rule_customization: false,
            can_create_custom_profile: true,
        },
        PROFILE_CODING_NERD => ProfilePermissions {
            allow_network_customization: true,
            allow_domain_customization: true,
            allow_elevation_customization: true,
            allow_rule_customization: true,
            can_create_custom_profile: true,
        },
        PROFILE_I_DONT_CARE => ProfilePermissions {
            allow_network_customization: true,
            allow_domain_customization: true,
            allow_elevation_customization: true,
            allow_rule_customization: true,
            can_create_custom_profile: true,
        },
        PROFILE_CUSTOM => ProfilePermissions {
            allow_network_customization: true,
            allow_domain_customization: true,
            allow_elevation_customization: true,
            allow_rule_customization: true,
            can_create_custom_profile: false,
        },
        _ => ProfilePermissions {
            allow_network_customization: true,
            allow_domain_customization: true,
            allow_elevation_customization: false,
            allow_rule_customization: false,
            can_create_custom_profile: true,
        },
    }
}

pub fn policy_profiles() -> Vec<PolicyProfile> {
    vec![
        PolicyProfile {
            id: PROFILE_STRICT.to_string(),
            label: "Strict".to_string(),
            description:
                "Maximum guardrails with low-risk defaults. Safe for untrusted tasks and shared machines."
                    .to_string(),
            details: vec![
                "Optimized for strongest deterministic protection with minimal ambiguity."
                    .to_string(),
                "Network and domain lists stay configurable; advanced filesystem/execution tuning is locked."
                    .to_string(),
                "System-critical and secrets boundaries stay deny-by-default.".to_string(),
            ],
            allow_rule_customization: profile_permissions(PROFILE_STRICT).allow_rule_customization,
            allow_elevation_customization: profile_permissions(PROFILE_STRICT)
                .allow_elevation_customization,
            allow_network_customization: profile_permissions(PROFILE_STRICT)
                .allow_network_customization,
            allow_domain_customization: profile_permissions(PROFILE_STRICT)
                .allow_domain_customization,
            can_create_custom_profile: profile_permissions(PROFILE_STRICT)
                .can_create_custom_profile,
        },
        PolicyProfile {
            id: PROFILE_SIMPLE_USER.to_string(),
            label: "Simple User".to_string(),
            description: "Safe defaults for daily use with as few decisions as possible.".to_string(),
            details: vec![
                "Locks most advanced controls so normal use stays simple and predictable.".to_string(),
                "Keeps managed export/deliver approvals and critical boundaries in place."
                    .to_string(),
                "Best option if you want safety without constantly tweaking settings.".to_string(),
            ],
            allow_rule_customization: profile_permissions(PROFILE_SIMPLE_USER)
                .allow_rule_customization,
            allow_elevation_customization: profile_permissions(PROFILE_SIMPLE_USER)
                .allow_elevation_customization,
            allow_network_customization: profile_permissions(PROFILE_SIMPLE_USER)
                .allow_network_customization,
            allow_domain_customization: profile_permissions(PROFILE_SIMPLE_USER)
                .allow_domain_customization,
            can_create_custom_profile: profile_permissions(PROFILE_SIMPLE_USER)
                .can_create_custom_profile,
        },
        PolicyProfile {
            id: PROFILE_BALANCED.to_string(),
            label: "Balanced".to_string(),
            description:
                "Ready-to-go profile: practical autonomy with strong boundary protection."
                    .to_string(),
            details: vec![
                "Good default if you want to start quickly and stay protected.".to_string(),
                "Network and elevation controls are configurable.".to_string(),
                "Advanced filesystem/execution/persistence tuning stays locked for consistency."
                    .to_string(),
            ],
            allow_rule_customization: profile_permissions(PROFILE_BALANCED)
                .allow_rule_customization,
            allow_elevation_customization: profile_permissions(PROFILE_BALANCED)
                .allow_elevation_customization,
            allow_network_customization: profile_permissions(PROFILE_BALANCED)
                .allow_network_customization,
            allow_domain_customization: profile_permissions(PROFILE_BALANCED)
                .allow_domain_customization,
            can_create_custom_profile: profile_permissions(PROFILE_BALANCED)
                .can_create_custom_profile,
        },
        PolicyProfile {
            id: PROFILE_CODING_NERD.to_string(),
            label: "Coding/Nerd".to_string(),
            description:
                "More freedom for coding-heavy workflows while keeping core guardrails on."
                    .to_string(),
            details: vec![
                "Designed for advanced users who want fewer interruptions on dev routines."
                    .to_string(),
                "Advanced rule customization is enabled, but minimum safety guards still apply."
                    .to_string(),
                "Great for iterative coding tasks, package installs, and local tooling."
                    .to_string(),
            ],
            allow_rule_customization: profile_permissions(PROFILE_CODING_NERD)
                .allow_rule_customization,
            allow_elevation_customization: profile_permissions(PROFILE_CODING_NERD)
                .allow_elevation_customization,
            allow_network_customization: profile_permissions(PROFILE_CODING_NERD)
                .allow_network_customization,
            allow_domain_customization: profile_permissions(PROFILE_CODING_NERD)
                .allow_domain_customization,
            can_create_custom_profile: profile_permissions(PROFILE_CODING_NERD)
                .can_create_custom_profile,
        },
        PolicyProfile {
            id: PROFILE_I_DONT_CARE.to_string(),
            label: "I DON'T CARE".to_string(),
            description:
                "Minimum-friction mode for emergency workflows."
                    .to_string(),
            details: vec![
                "Very permissive behavior for trusted local usage and temporary unblocking."
                    .to_string(),
                "You don't care, but I do lol 😂.".to_string(),
                "Still enforces baseline safety guards on critical boundaries.".to_string(),
                "Not recommended for normal day-to-day operation.".to_string(),
            ],
            allow_rule_customization: profile_permissions(PROFILE_I_DONT_CARE)
                .allow_rule_customization,
            allow_elevation_customization: profile_permissions(PROFILE_I_DONT_CARE)
                .allow_elevation_customization,
            allow_network_customization: profile_permissions(PROFILE_I_DONT_CARE)
                .allow_network_customization,
            allow_domain_customization: profile_permissions(PROFILE_I_DONT_CARE)
                .allow_domain_customization,
            can_create_custom_profile: profile_permissions(PROFILE_I_DONT_CARE)
                .can_create_custom_profile,
        },
        PolicyProfile {
            id: PROFILE_CUSTOM.to_string(),
            label: "Custom".to_string(),
            description:
                "Full personalization mode for operators who want to tune almost everything."
                    .to_string(),
            details: vec![
                "All major policy controls are configurable in the control panel.".to_string(),
                "Use this for team-specific workflows, exceptions, and experiments.".to_string(),
                "Baseline safety guards are still enforced underneath.".to_string(),
            ],
            allow_rule_customization: profile_permissions(PROFILE_CUSTOM).allow_rule_customization,
            allow_elevation_customization: profile_permissions(PROFILE_CUSTOM)
                .allow_elevation_customization,
            allow_network_customization: profile_permissions(PROFILE_CUSTOM)
                .allow_network_customization,
            allow_domain_customization: profile_permissions(PROFILE_CUSTOM)
                .allow_domain_customization,
            can_create_custom_profile: profile_permissions(PROFILE_CUSTOM)
                .can_create_custom_profile,
        },
    ]
}

pub fn profile_allows_rule_customization(profile: &str) -> bool {
    profile_permissions(profile).allow_rule_customization
}

pub fn enforce_minimum_safety_guards(policy: &mut Policy) {
    // "You don't care, but I do": always keep core boundaries active.
    policy.rules.filesystem.system_critical = RuleDisposition::Deny;
    policy.rules.filesystem.secrets = RuleDisposition::Deny;
    policy.rules.execution.quarantine_on_download_exec_chain = true;
    policy.safeguards.mass_delete_threshold = policy.safeguards.mass_delete_threshold.max(20);
}

pub fn enforce_system_critical_guard(policy: &mut Policy) {
    policy.rules.filesystem.system_critical = RuleDisposition::Deny;
}

pub fn apply_profile_preset(policy: &mut Policy, profile: &str) -> Result<()> {
    let mut network_allowlist = policy.rules.network.allowlist_hosts.clone();
    network_allowlist.retain(|item| !item.trim().is_empty());
    network_allowlist.sort();
    network_allowlist.dedup();

    let mut network_denylist = policy.rules.network.denylist_hosts.clone();
    network_denylist.retain(|item| !item.trim().is_empty());
    network_denylist.sort();
    network_denylist.dedup();

    match canonical_profile_id(profile) {
        Some(PROFILE_STRICT) => {
            policy.profile = PROFILE_STRICT.to_string();
            policy.rules.filesystem.workspace = RuleDisposition::Allow;
            policy.rules.filesystem.user_data = RuleDisposition::Allow;
            policy.rules.filesystem.shared = RuleDisposition::Approval;
            policy.rules.filesystem.secrets = RuleDisposition::Deny;

            policy.rules.network.default_deny = true;
            policy.rules.network.require_approval_for_post = true;
            policy.rules.network.invert_allowlist = true;
            policy.rules.network.invert_denylist = true;

            policy.rules.execution.deny_workspace_exec = true;
            policy.rules.execution.deny_tmp_exec = true;
            policy.rules.execution.quarantine_on_download_exec_chain = true;

            policy.rules.persistence.deny_autostart = false;
            policy.safeguards.mass_delete_threshold = DEFAULT_MASS_DELETE_THRESHOLD;
        }
        Some(PROFILE_SIMPLE_USER) => {
            policy.profile = PROFILE_SIMPLE_USER.to_string();
            policy.rules.filesystem.workspace = RuleDisposition::Allow;
            policy.rules.filesystem.user_data = RuleDisposition::Allow;
            policy.rules.filesystem.shared = RuleDisposition::Approval;
            policy.rules.filesystem.secrets = RuleDisposition::Deny;

            policy.rules.network.default_deny = true;
            policy.rules.network.require_approval_for_post = true;
            policy.rules.network.invert_allowlist = true;
            policy.rules.network.invert_denylist = true;

            policy.rules.execution.deny_workspace_exec = false;
            policy.rules.execution.deny_tmp_exec = true;
            policy.rules.execution.quarantine_on_download_exec_chain = true;

            policy.rules.persistence.deny_autostart = false;
            policy.safeguards.mass_delete_threshold = DEFAULT_MASS_DELETE_THRESHOLD;
        }
        Some(PROFILE_BALANCED) => {
            policy.profile = PROFILE_BALANCED.to_string();
            policy.rules.filesystem.workspace = RuleDisposition::Allow;
            policy.rules.filesystem.user_data = RuleDisposition::Allow;
            policy.rules.filesystem.shared = RuleDisposition::Approval;
            policy.rules.filesystem.secrets = RuleDisposition::Deny;

            policy.rules.network.default_deny = false;
            policy.rules.network.require_approval_for_post = true;
            policy.rules.network.invert_allowlist = true;
            policy.rules.network.invert_denylist = true;

            policy.rules.execution.deny_workspace_exec = false;
            policy.rules.execution.deny_tmp_exec = true;
            policy.rules.execution.quarantine_on_download_exec_chain = true;

            policy.rules.persistence.deny_autostart = false;
            policy.safeguards.mass_delete_threshold = DEFAULT_MASS_DELETE_THRESHOLD;
        }
        Some(PROFILE_CODING_NERD) => {
            policy.profile = PROFILE_CODING_NERD.to_string();
            policy.rules.filesystem.workspace = RuleDisposition::Allow;
            policy.rules.filesystem.user_data = RuleDisposition::Allow;
            policy.rules.filesystem.shared = RuleDisposition::Approval;
            policy.rules.filesystem.secrets = RuleDisposition::Deny;

            policy.rules.network.default_deny = false;
            policy.rules.network.require_approval_for_post = true;
            policy.rules.network.invert_allowlist = true;
            policy.rules.network.invert_denylist = true;

            policy.rules.execution.deny_workspace_exec = false;
            policy.rules.execution.deny_tmp_exec = false;
            policy.rules.execution.quarantine_on_download_exec_chain = true;

            policy.rules.persistence.deny_autostart = false;
            policy.safeguards.mass_delete_threshold = 80;
        }
        Some(PROFILE_I_DONT_CARE) => {
            policy.profile = PROFILE_I_DONT_CARE.to_string();
            policy.rules.filesystem.workspace = RuleDisposition::Allow;
            policy.rules.filesystem.user_data = RuleDisposition::Allow;
            policy.rules.filesystem.shared = RuleDisposition::Allow;
            policy.rules.filesystem.secrets = RuleDisposition::Deny;

            policy.rules.network.default_deny = false;
            policy.rules.network.invert_allowlist = true;
            policy.rules.network.invert_denylist = true;
            network_allowlist.clear();

            policy.rules.execution.deny_workspace_exec = false;
            policy.rules.execution.deny_tmp_exec = false;
            policy.rules.execution.quarantine_on_download_exec_chain = true;

            policy.rules.persistence.deny_autostart = false;
            policy.rules.persistence.approval_paths.clear();

            policy.safeguards.mass_delete_threshold = OPEN_MASS_DELETE_THRESHOLD;
        }
        Some(PROFILE_CUSTOM) => {
            policy.profile = PROFILE_CUSTOM.to_string();
        }
        None => return Err(anyhow!("unsupported profile: {profile}")),
        Some(other) => return Err(anyhow!("unsupported profile: {other}")),
    }

    policy.rules.network.allowlist_hosts = network_allowlist;
    policy.rules.network.denylist_hosts = network_denylist;
    enforce_minimum_safety_guards(policy);
    Ok(())
}
