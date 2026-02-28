use crate::config::RuleDisposition;
use crate::model::{ActionKind, ActionRequest, Decision, ReasonCode, Verdict, Zone};
use crate::utils::is_subpath;
use std::path::Path;

use super::helpers::{deny_reason_for_zone, is_tmp_path};
use super::PolicyEngine;

/// Check if the action kind is a write operation (modifies filesystem).
fn is_write_action(kind: ActionKind) -> bool {
    matches!(
        kind,
        ActionKind::FileWrite | ActionKind::FileDelete | ActionKind::FileRename
    )
}

/// Check if the action is a read operation (non-mutating).
/// Reads may be hinted via metadata for tools that use FileWrite kind for reads.
fn is_read_action(request: &ActionRequest) -> bool {
    // Explicit read hint from tool preflight
    if request
        .metadata
        .get("non_mutating_hint")
        .map(|v| v == "read")
        .unwrap_or(false)
    {
        return true;
    }
    // Native read action kinds would go here if we add them
    false
}

/// Check if this is an import operation (reading from external source into workspace).
/// Imports read from user_data but write to workspace, so they should not be blocked
/// by user_data write denial.
fn is_import_operation(request: &ActionRequest) -> bool {
    // Import operations have import_src metadata and write to workspace
    request.metadata.contains_key("import_src")
        && request
            .metadata
            .get("import_dst")
            .map(|_| true)
            .unwrap_or(false)
}

fn targets_agent_ruler_cli(request: &ActionRequest) -> bool {
    if let Some(path) = request.path.as_ref() {
        if is_agent_ruler_token(path.to_string_lossy().as_ref()) {
            return true;
        }
    }

    if request
        .metadata
        .get("argv")
        .map(|raw| contains_agent_ruler_token(raw))
        .unwrap_or(false)
    {
        return true;
    }

    contains_agent_ruler_token(&request.process.command)
}

fn contains_agent_ruler_token(text: &str) -> bool {
    text.split_whitespace().any(is_agent_ruler_token)
}

fn is_agent_ruler_token(token: &str) -> bool {
    let cleaned = token.trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\'' | ';' | '|' | '&' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | '>' | '<'
        )
    });

    Path::new(cleaned)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("agent-ruler"))
        .unwrap_or(false)
}

impl PolicyEngine {
    pub(super) fn evaluate_filesystem(&self, request: &ActionRequest, zone: Zone) -> Decision {
        let Some(path) = request.path.as_ref() else {
            return Decision {
                verdict: Verdict::Deny,
                reason: ReasonCode::DenyInvalidRequest,
                detail: "filesystem action missing path".to_string(),
                approval_ttl_seconds: None,
            };
        };

        if matches!(request.kind, ActionKind::FileDelete) {
            if request
                .metadata
                .get("delete_wildcard")
                .map(|value| value == "true")
                .unwrap_or(false)
            {
                return Decision {
                    verdict: Verdict::RequireApproval,
                    reason: ReasonCode::ApprovalRequiredMassDelete,
                    detail: "mass delete guard tripped: wildcard delete requires approval"
                        .to_string(),
                    approval_ttl_seconds: Some(self.policy.approvals.ttl_seconds),
                };
            }
            if let Some(raw) = request.metadata.get("delete_count") {
                if let Ok(count) = raw.parse::<usize>() {
                    if count >= self.policy.safeguards.mass_delete_threshold {
                        return Decision {
                            verdict: Verdict::RequireApproval,
                            reason: ReasonCode::ApprovalRequiredMassDelete,
                            detail: format!(
                                "mass delete guard tripped: {} items >= threshold {}",
                                count, self.policy.safeguards.mass_delete_threshold
                            ),
                            approval_ttl_seconds: Some(self.policy.approvals.ttl_seconds),
                        };
                    }
                }
            }
        }

        // Doc intent enforcement for user_data zone:
        // - Reads MAY be allowed depending on policy disposition
        // - Writes MUST be denied - modifications must go through stage/deliver workflow
        // This ensures agents can read user documents to simulate working in user space,
        // but cannot directly modify them outside the workspace.
        // Exception: Import operations read from user_data and write to workspace, which is allowed.
        if zone == Zone::UserData
            && is_write_action(request.kind)
            && !is_read_action(request)
            && !is_import_operation(request)
        {
            return Decision {
                verdict: Verdict::Deny,
                reason: ReasonCode::DenyUserDataWrite,
                detail: format!(
                    "write to user data zone denied (use stage/deliver workflow): {}",
                    path.display()
                ),
                approval_ttl_seconds: None,
            };
        }

        match self.disposition_for_zone(zone) {
            RuleDisposition::Allow => Decision {
                verdict: Verdict::Allow,
                reason: ReasonCode::AllowedByPolicy,
                detail: format!(
                    "filesystem action allowed in zone {:?}: {}",
                    zone,
                    path.display()
                ),
                approval_ttl_seconds: None,
            },
            RuleDisposition::Approval => Decision {
                verdict: Verdict::RequireApproval,
                reason: ReasonCode::ApprovalRequiredZone2,
                detail: format!("filesystem action requires approval in zone {:?}", zone),
                approval_ttl_seconds: Some(self.policy.approvals.ttl_seconds),
            },
            RuleDisposition::Deny => Decision {
                verdict: Verdict::Deny,
                reason: deny_reason_for_zone(zone),
                detail: format!(
                    "filesystem action denied for zone {:?}: {}",
                    zone,
                    path.display()
                ),
                approval_ttl_seconds: None,
            },
        }
    }

    // Execution policy applies explicit download->exec quarantine before generic path denies.
    // This implements OWASP-aligned controls for download→exec chains and interpreter attacks.
    pub(super) fn evaluate_execution(&self, request: &ActionRequest, zone: Zone) -> Decision {
        let Some(path) = request.path.as_ref() else {
            return Decision {
                verdict: Verdict::Deny,
                reason: ReasonCode::DenyInvalidRequest,
                detail: "execution request missing path".to_string(),
                approval_ttl_seconds: None,
            };
        };

        if targets_agent_ruler_cli(request) {
            return Decision {
                verdict: Verdict::Deny,
                reason: ReasonCode::DenySystemCritical,
                detail: "agent-ruler CLI is operator-only; use Agent Ruler API endpoints/tools"
                    .to_string(),
                approval_ttl_seconds: None,
            };
        }

        if request
            .metadata
            .get("stream_exec")
            .map(|v| v == "true")
            .unwrap_or(false)
        {
            return Decision {
                verdict: Verdict::Deny,
                reason: ReasonCode::DenyInterpreterStreamExec,
                detail: format!(
                    "interpreter stream execution blocked (curl|bash pattern): {}",
                    path.display()
                ),
                approval_ttl_seconds: None,
            };
        }

        if request
            .metadata
            .get("downloaded")
            .map(|v| v == "true")
            .unwrap_or(false)
        {
            if request
                .metadata
                .get("interpreter")
                .map(|v| v == "true")
                .unwrap_or(false)
            {
                return Decision {
                    verdict: Verdict::Quarantine,
                    reason: ReasonCode::QuarantineInterpreterDownload,
                    detail: format!(
                        "interpreter execution of downloaded content quarantined: {}",
                        path.display()
                    ),
                    approval_ttl_seconds: None,
                };
            }

            if self
                .policy
                .rules
                .execution
                .quarantine_on_download_exec_chain
            {
                return Decision {
                    verdict: Verdict::Quarantine,
                    reason: ReasonCode::QuarantineDownloadExecChain,
                    detail: format!(
                        "download->exec chain flagged; artifact quarantined: {}",
                        path.display()
                    ),
                    approval_ttl_seconds: None,
                };
            }

            return Decision {
                verdict: Verdict::Deny,
                reason: ReasonCode::DenyExecutionDownloaded,
                detail: format!("execution of downloaded file denied: {}", path.display()),
                approval_ttl_seconds: None,
            };
        }

        if request
            .metadata
            .get("interpreter")
            .map(|v| v == "true")
            .unwrap_or(false)
        {
            if let Some(script_path) = request.metadata.get("script_path") {
                let script = std::path::PathBuf::from(script_path);
                if is_subpath(&script, &self.workspace) {
                    return Decision {
                        verdict: Verdict::Quarantine,
                        reason: ReasonCode::QuarantineInterpreterDownload,
                        detail: format!(
                            "interpreter execution of untrusted script quarantined: {}",
                            script_path
                        ),
                        approval_ttl_seconds: None,
                    };
                }
            }
        }

        if self.policy.rules.execution.deny_workspace_exec && is_subpath(path, &self.workspace) {
            return Decision {
                verdict: Verdict::Deny,
                reason: ReasonCode::DenyExecutionFromWorkspace,
                detail: format!(
                    "execution blocked for workspace artifact {}; use export/approval flow",
                    path.display()
                ),
                approval_ttl_seconds: None,
            };
        }

        if self.policy.rules.execution.deny_tmp_exec && is_tmp_path(path) {
            return Decision {
                verdict: Verdict::Deny,
                reason: ReasonCode::DenyExecutionFromTemp,
                detail: format!(
                    "execution blocked for temporary path {}; likely download->exec chain",
                    path.display()
                ),
                approval_ttl_seconds: None,
            };
        }

        let explicitly_allowed = self
            .compiled
            .allowed_exec_prefixes
            .iter()
            .any(|p| p.matches(path));

        if explicitly_allowed {
            return Decision {
                verdict: Verdict::Allow,
                reason: ReasonCode::AllowedByPolicy,
                detail: format!(
                    "execution allowed by explicit prefix for {}",
                    path.display()
                ),
                approval_ttl_seconds: None,
            };
        }

        match self.disposition_for_zone(zone) {
            RuleDisposition::Allow => Decision {
                verdict: Verdict::Allow,
                reason: ReasonCode::AllowedByPolicy,
                detail: format!("execution allowed for {}", path.display()),
                approval_ttl_seconds: None,
            },
            RuleDisposition::Approval => Decision {
                verdict: Verdict::RequireApproval,
                reason: ReasonCode::ApprovalRequiredZone2,
                detail: format!("execution requires approval in zone {:?}", zone),
                approval_ttl_seconds: Some(self.policy.approvals.ttl_seconds),
            },
            RuleDisposition::Deny => Decision {
                verdict: Verdict::Deny,
                reason: deny_reason_for_zone(zone),
                detail: format!("execution denied in zone {:?}: {}", zone, path.display()),
                approval_ttl_seconds: None,
            },
        }
    }

    pub(super) fn evaluate_network(&self, request: &ActionRequest) -> Decision {
        let host = request.host.as_ref();
        if host.is_none() && self.policy.rules.network.default_deny {
            return Decision {
                verdict: Verdict::Deny,
                reason: ReasonCode::DenyNetworkDefault,
                detail: "network request missing host under restricted policy".to_string(),
                approval_ttl_seconds: None,
            };
        }

        if let Some(candidate) = host {
            let host_check = evaluate_network_host(
                &self.policy.rules.network,
                candidate,
                self.policy.rules.network.default_deny,
            );
            if !host_check.allowed {
                return Decision {
                    verdict: Verdict::Deny,
                    reason: ReasonCode::DenyNetworkNotAllowlisted,
                    detail: host_check.detail,
                    approval_ttl_seconds: None,
                };
            }

            let method = request
                .metadata
                .get("method")
                .map(|v| v.to_ascii_uppercase())
                .unwrap_or_else(|| "GET".to_string());
            if self.policy.rules.network.require_approval_for_post && method == "POST" {
                return Decision {
                    verdict: Verdict::RequireApproval,
                    reason: ReasonCode::ApprovalRequiredNetworkUpload,
                    detail: format!("network POST request to {} requires approval", candidate),
                    approval_ttl_seconds: Some(self.policy.approvals.ttl_seconds),
                };
            }

            if request
                .metadata
                .get("upload_pattern")
                .map(|v| v == "true")
                .unwrap_or(false)
            {
                return Decision {
                    verdict: Verdict::RequireApproval,
                    reason: ReasonCode::ApprovalRequiredNetworkUpload,
                    detail: format!(
                        "network upload-style request to {} requires approval",
                        candidate
                    ),
                    approval_ttl_seconds: Some(self.policy.approvals.ttl_seconds),
                };
            }

            return Decision {
                verdict: Verdict::Allow,
                reason: ReasonCode::AllowedByPolicy,
                detail: host_check.allow_detail,
                approval_ttl_seconds: None,
            };
        }

        Decision {
            verdict: Verdict::Allow,
            reason: ReasonCode::AllowedByPolicy,
            detail: "network permitted by profile".to_string(),
            approval_ttl_seconds: None,
        }
    }

    pub(super) fn evaluate_download(&self, request: &ActionRequest) -> Decision {
        if let Some(host) = &request.host {
            let host_check = evaluate_network_host(
                &self.policy.rules.network,
                host,
                self.policy.rules.network.default_deny,
            );
            if !host_check.allowed {
                return Decision {
                    verdict: Verdict::Deny,
                    reason: ReasonCode::DenyNetworkNotAllowlisted,
                    detail: format!("download denied: {}", host_check.detail),
                    approval_ttl_seconds: None,
                };
            }
        }

        if request
            .metadata
            .get("marks_executable")
            .map(|v| v == "true")
            .unwrap_or(false)
        {
            return Decision {
                verdict: Verdict::RequireApproval,
                reason: ReasonCode::ApprovalRequiredZone2,
                detail: "download marked executable requires approval".to_string(),
                approval_ttl_seconds: Some(self.policy.approvals.ttl_seconds),
            };
        }

        Decision {
            verdict: Verdict::Allow,
            reason: ReasonCode::AllowedByPolicy,
            detail: "download metadata accepted".to_string(),
            approval_ttl_seconds: None,
        }
    }

    pub(super) fn evaluate_persistence(&self, request: &ActionRequest, zone: Zone) -> Decision {
        let path = request.path.as_ref().cloned().or_else(|| {
            request
                .metadata
                .get("target_path")
                .map(std::path::PathBuf::from)
        });
        let Some(path) = path else {
            return Decision {
                verdict: Verdict::Deny,
                reason: ReasonCode::DenyInvalidRequest,
                detail: "persistence request missing path".to_string(),
                approval_ttl_seconds: None,
            };
        };
        let mechanism = persistence_mechanism(request, &path);
        let scope = persistence_scope(request, &path);
        let suspicious_chain = request
            .metadata
            .get("suspicious_chain")
            .map(|value| value == "true")
            .unwrap_or(false);

        if suspicious_chain {
            return Decision {
                verdict: Verdict::Quarantine,
                reason: ReasonCode::QuarantineHighRiskPattern,
                detail: format!(
                    "persistence {} target in {} scope flagged as suspicious chain: {}",
                    mechanism,
                    scope,
                    path.display()
                ),
                approval_ttl_seconds: None,
            };
        }

        if self.matches_any(&path, &self.compiled.persistence_deny) {
            return Decision {
                verdict: Verdict::Deny,
                reason: ReasonCode::DenyPersistence,
                detail: format!(
                    "persistence {} target denied by policy for {} scope: {}",
                    mechanism,
                    scope,
                    path.display()
                ),
                approval_ttl_seconds: None,
            };
        }

        if scope == "system" || self.matches_any(&path, &self.compiled.persistence_approval) {
            return Decision {
                verdict: Verdict::RequireApproval,
                reason: ReasonCode::ApprovalRequiredPersistence,
                detail: format!(
                    "persistence {} target requires approval for {} scope: {}",
                    mechanism,
                    scope,
                    path.display()
                ),
                approval_ttl_seconds: Some(self.policy.approvals.ttl_seconds),
            };
        }

        if self.policy.rules.persistence.deny_autostart && looks_like_user_autostart(&path) {
            return Decision {
                verdict: Verdict::Deny,
                reason: ReasonCode::DenyPersistence,
                detail: format!(
                    "user autostart persistence denied by policy: {}",
                    path.display()
                ),
                approval_ttl_seconds: None,
            };
        }

        match self.disposition_for_zone(zone) {
            RuleDisposition::Allow => Decision {
                verdict: Verdict::Allow,
                reason: ReasonCode::AllowedByPolicy,
                detail: format!(
                    "persistence {} target allowed in {} scope (zone {:?})",
                    mechanism, scope, zone
                ),
                approval_ttl_seconds: None,
            },
            RuleDisposition::Approval => Decision {
                verdict: Verdict::RequireApproval,
                reason: ReasonCode::ApprovalRequiredPersistence,
                detail: format!(
                    "persistence {} target requires approval in {} scope (zone {:?})",
                    mechanism, scope, zone
                ),
                approval_ttl_seconds: Some(self.policy.approvals.ttl_seconds),
            },
            RuleDisposition::Deny => Decision {
                verdict: Verdict::Deny,
                reason: ReasonCode::DenyPersistence,
                detail: format!(
                    "persistence {} target denied in {} scope (zone {:?})",
                    mechanism, scope, zone
                ),
                approval_ttl_seconds: None,
            },
        }
    }

    pub(super) fn evaluate_export(&self, request: &ActionRequest, zone: Zone) -> Decision {
        let Some(dst) = request.path.as_ref() else {
            return Decision {
                verdict: Verdict::Deny,
                reason: ReasonCode::DenyInvalidRequest,
                detail: "export request missing destination path".to_string(),
                approval_ttl_seconds: None,
            };
        };

        match zone {
            Zone::Workspace => Decision {
                verdict: Verdict::Allow,
                reason: ReasonCode::AllowedByPolicy,
                detail: "export destination is workspace".to_string(),
                approval_ttl_seconds: None,
            },
            Zone::UserData => Decision {
                verdict: Verdict::RequireApproval,
                reason: ReasonCode::ApprovalRequiredExport,
                detail: format!(
                    "export destination in user-data zone requires approval: {}",
                    dst.display()
                ),
                approval_ttl_seconds: Some(self.policy.approvals.ttl_seconds),
            },
            Zone::Shared => Decision {
                verdict: Verdict::RequireApproval,
                reason: ReasonCode::ApprovalRequiredExport,
                detail: format!("export destination in shared zone: {}", dst.display()),
                approval_ttl_seconds: Some(self.policy.approvals.ttl_seconds),
            },
            Zone::SystemCritical => Decision {
                verdict: Verdict::Deny,
                reason: ReasonCode::DenySystemCritical,
                detail: format!("export destination is system-critical: {}", dst.display()),
                approval_ttl_seconds: None,
            },
            Zone::Secrets => Decision {
                verdict: Verdict::Deny,
                reason: ReasonCode::DenySecrets,
                detail: format!("export destination is secrets zone: {}", dst.display()),
                approval_ttl_seconds: None,
            },
        }
    }

    fn disposition_for_zone(&self, zone: Zone) -> RuleDisposition {
        match zone {
            Zone::Workspace => self.policy.rules.filesystem.workspace,
            Zone::UserData => self.policy.rules.filesystem.user_data,
            Zone::Shared => self.policy.rules.filesystem.shared,
            Zone::SystemCritical => self.policy.rules.filesystem.system_critical,
            Zone::Secrets => self.policy.rules.filesystem.secrets,
        }
    }
}

fn persistence_scope(request: &ActionRequest, path: &std::path::Path) -> &'static str {
    if let Some(scope) = request.metadata.get("persistence_scope") {
        let normalized = scope.to_ascii_lowercase();
        if normalized == "system" {
            return "system";
        }
        if normalized == "user" {
            return "user";
        }
    }
    infer_persistence_scope(path)
}

fn persistence_mechanism(request: &ActionRequest, path: &std::path::Path) -> &'static str {
    if let Some(mechanism) = request.metadata.get("persistence_mechanism") {
        let normalized = mechanism.to_ascii_lowercase();
        if normalized == "cron" {
            return "cron";
        }
        if normalized == "systemd" {
            return "systemd";
        }
        if normalized == "autostart" {
            return "autostart";
        }
    }
    infer_persistence_mechanism(path)
}

fn infer_persistence_scope(path: &std::path::Path) -> &'static str {
    let normalized = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    if normalized.contains("/etc/")
        || normalized.contains("/usr/lib/systemd/")
        || normalized.contains("/lib/systemd/")
        || normalized.contains("/var/spool/cron/")
        || normalized.contains("/etc/cron")
    {
        return "system";
    }
    "user"
}

fn infer_persistence_mechanism(path: &std::path::Path) -> &'static str {
    let normalized = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    if normalized.contains("systemd") || normalized.ends_with(".service") {
        return "systemd";
    }
    if normalized.contains("autostart") || normalized.ends_with(".desktop") {
        return "autostart";
    }
    if normalized.contains("cron") || normalized.ends_with("crontab") {
        return "cron";
    }
    "other"
}

fn looks_like_user_autostart(path: &std::path::Path) -> bool {
    let normalized = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    normalized.contains("/.config/autostart/")
        || normalized.contains("/.config/systemd/user/")
        || normalized.contains("/.config/cron/")
        || normalized.ends_with("/.crontab")
}

struct NetworkHostCheck {
    allowed: bool,
    detail: String,
    allow_detail: String,
}

fn evaluate_network_host(
    rules: &crate::config::NetworkRules,
    host: &str,
    require_explicit_allow: bool,
) -> NetworkHostCheck {
    let in_allowlist = host_in_list(&rules.allowlist_hosts, host);
    let in_denylist = host_in_list(&rules.denylist_hosts, host);

    let (allowlist_pass, allowlist_explicit, allowlist_desc) = if rules.allowlist_hosts.is_empty() {
        (
            true,
            false,
            "allowlist empty (no allowlist restriction)".to_string(),
        )
    } else if rules.invert_allowlist {
        (
            !in_allowlist,
            false,
            if in_allowlist {
                "host matched inverted allowlist deny-set".to_string()
            } else {
                "host passed inverted allowlist deny-set".to_string()
            },
        )
    } else {
        (
            in_allowlist,
            in_allowlist,
            if in_allowlist {
                "host matched allowlist".to_string()
            } else {
                "host missing from allowlist".to_string()
            },
        )
    };

    let (denylist_pass, denylist_explicit, denylist_desc) = if rules.denylist_hosts.is_empty() {
        (
            true,
            false,
            "denylist empty (no denylist restriction)".to_string(),
        )
    } else if rules.invert_denylist {
        (
            in_denylist,
            in_denylist,
            if in_denylist {
                "host matched inverted denylist allow-set".to_string()
            } else {
                "host missing from inverted denylist allow-set".to_string()
            },
        )
    } else {
        (
            !in_denylist,
            false,
            if in_denylist {
                "host blocked by denylist".to_string()
            } else {
                "host passed denylist".to_string()
            },
        )
    };

    let policy_pass = allowlist_pass && denylist_pass;
    if !policy_pass {
        return NetworkHostCheck {
            allowed: false,
            detail: format!("network host {host} rejected ({allowlist_desc}; {denylist_desc})"),
            allow_detail: String::new(),
        };
    }

    let explicit_allow = allowlist_explicit || denylist_explicit;
    if require_explicit_allow && !explicit_allow {
        return NetworkHostCheck {
            allowed: false,
            detail: format!(
                "network host {host} blocked by default-deny: explicit allowlist match required"
            ),
            allow_detail: String::new(),
        };
    }

    NetworkHostCheck {
        allowed: true,
        detail: format!("network host {host} accepted ({allowlist_desc}; {denylist_desc})"),
        allow_detail: format!("network host {host} permitted ({allowlist_desc}; {denylist_desc})"),
    }
}

fn host_in_list(list: &[String], host: &str) -> bool {
    list.iter().any(|entry| entry.eq_ignore_ascii_case(host))
}
