use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use chrono::Utc;

use crate::approvals::ApprovalStore;
use crate::config::RuntimeState;
use crate::model::{ActionKind, ActionRequest, Decision, ProcessContext, ReasonCode, Verdict};
use crate::policy::PolicyEngine;
use crate::receipts::ReceiptStore;

use super::{
    append_receipt, insert_runner_id_metadata, quarantine_path, redacted_command_for_receipts,
};

pub(super) fn preflight_elevation_actions(
    cmd: &[String],
    runtime: &RuntimeState,
    approvals: &ApprovalStore,
    receipts: &ReceiptStore,
) -> Result<()> {
    let Some(intent) = parse_sudo_intent(cmd) else {
        return Ok(());
    };
    let receipt_command = redacted_command_for_receipts(cmd);

    let base_request = ActionRequest {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        kind: ActionKind::Execute,
        operation: "elevation_request".to_string(),
        path: Some(PathBuf::from("/usr/bin/apt-get")),
        secondary_path: None,
        host: None,
        metadata: {
            let mut metadata = BTreeMap::new();
            metadata.insert("argv".to_string(), receipt_command.clone());
            insert_runner_id_metadata(&mut metadata, cmd);
            metadata
        },
        process: ProcessContext {
            pid: std::process::id(),
            ppid: None,
            command: receipt_command,
            process_tree: vec![std::process::id()],
        },
    };

    let deny_and_log = |reason: ReasonCode, detail: String| -> Result<()> {
        append_receipt(
            receipts,
            runtime,
            base_request.clone(),
            Decision {
                verdict: Verdict::Deny,
                reason,
                detail: detail.clone(),
                approval_ttl_seconds: None,
            },
            None,
            None,
            "elevation-preflight-deny",
        )?;
        Err(anyhow!("{detail}"))
    };

    if !runtime.policy.rules.elevation.enabled {
        return deny_and_log(
            ReasonCode::DenyElevationUnsupported,
            "elevation is disabled by policy".to_string(),
        );
    }

    match intent {
        ElevationIntent::InstallPackages { packages } => {
            if packages.is_empty() {
                return deny_and_log(
                    ReasonCode::DenyInvalidRequest,
                    "elevation install_packages requires at least one package".to_string(),
                );
            }

            let denylist = &runtime.policy.rules.elevation.denied_packages;
            for package in &packages {
                if denylist.iter().any(|item| item == package) {
                    return deny_and_log(
                        ReasonCode::DenyElevationPackageNotAllowlisted,
                        format!("package '{}' is denylisted for elevation", package),
                    );
                }
            }

            let allowlist = &runtime.policy.rules.elevation.allowed_packages;
            let use_allowlist = runtime.policy.rules.elevation.use_allowlist;
            if use_allowlist && allowlist.is_empty() {
                return deny_and_log(
                    ReasonCode::DenyElevationPackageNotAllowlisted,
                    "use_allowlist is enabled, but no elevation packages are allowlisted"
                        .to_string(),
                );
            }

            if use_allowlist {
                for package in &packages {
                    if !allowlist.iter().any(|item| item == package) {
                        return deny_and_log(
                            ReasonCode::DenyElevationPackageNotAllowlisted,
                            format!("package '{}' is not allowlisted for elevation", package),
                        );
                    }
                }
            }

            let nonce = uuid::Uuid::new_v4().to_string();
            let mut request = base_request;
            request.operation = "elevation_install_packages".to_string();
            request
                .metadata
                .insert("elevation_packages".to_string(), packages.join(","));
            request
                .metadata
                .insert("elevation_verb".to_string(), "install_packages".to_string());
            request
                .metadata
                .insert("elevation_nonce".to_string(), nonce);

            let decision = Decision {
                verdict: Verdict::RequireApproval,
                reason: ReasonCode::ApprovalRequiredElevation,
                detail: format!(
                    "Elevation requested: install packages [{}]. Approval + OS auth required.",
                    packages.join(", ")
                ),
                approval_ttl_seconds: Some(runtime.policy.approvals.ttl_seconds),
            };

            append_receipt(
                receipts,
                runtime,
                request.clone(),
                decision.clone(),
                None,
                None,
                "elevation-preflight-pending",
            )?;

            let approval = approvals.create_pending(
                &request,
                &decision,
                "elevation install_packages requires approval",
            )?;
            return Err(anyhow!(
                "Elevation requested: install packages [{}]. Approval + OS auth required. pending id: {}",
                packages.join(", "),
                approval.id
            ));
        }
        ElevationIntent::Unsupported { detail } => {
            deny_and_log(ReasonCode::DenyElevationUnsupported, detail)?;
        }
    }

    Ok(())
}

pub(super) fn preflight_utility_actions(
    cmd: &[String],
    runtime: &RuntimeState,
    engine: &PolicyEngine,
    approvals: &ApprovalStore,
    receipts: &ReceiptStore,
) -> Result<()> {
    if cmd.is_empty() {
        return Ok(());
    }
    let receipt_command = redacted_command_for_receipts(cmd);

    let tool_name = Path::new(&cmd[0])
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| cmd[0].clone());

    match tool_name.as_str() {
        "rm" => {
            for arg in cmd.iter().skip(1).filter(|a| !a.starts_with('-')) {
                let target = normalize_runtime_path(runtime, arg);
                let action = ActionRequest {
                    id: uuid::Uuid::new_v4().to_string(),
                    timestamp: Utc::now(),
                    kind: ActionKind::FileDelete,
                    operation: "preflight_rm".to_string(),
                    path: Some(target.clone()),
                    secondary_path: None,
                    host: None,
                    metadata: {
                        let mut metadata = BTreeMap::new();
                        insert_runner_id_metadata(&mut metadata, cmd);
                        metadata
                    },
                    process: ProcessContext {
                        pid: std::process::id(),
                        ppid: None,
                        command: receipt_command.clone(),
                        process_tree: vec![std::process::id()],
                    },
                };
                deny_direct_delivery_destination_write(&action, runtime, receipts, None)?;
                evaluate_and_log_action(
                    &action,
                    runtime,
                    engine,
                    approvals,
                    receipts,
                    "utility-preflight",
                    "utility action requires approval",
                )?;
            }
            Ok(())
        }
        "mv" => {
            let args: Vec<&String> = cmd.iter().skip(1).filter(|a| !a.starts_with('-')).collect();
            if args.len() >= 2 {
                let src = normalize_runtime_path(runtime, args[0]);
                let dst = normalize_runtime_path(runtime, args[1]);
                let action = ActionRequest {
                    id: uuid::Uuid::new_v4().to_string(),
                    timestamp: Utc::now(),
                    kind: ActionKind::FileRename,
                    operation: "preflight_mv".to_string(),
                    path: Some(dst),
                    secondary_path: Some(src),
                    host: None,
                    metadata: {
                        let mut metadata = BTreeMap::new();
                        insert_runner_id_metadata(&mut metadata, cmd);
                        metadata
                    },
                    process: ProcessContext {
                        pid: std::process::id(),
                        ppid: None,
                        command: receipt_command.clone(),
                        process_tree: vec![std::process::id()],
                    },
                };
                deny_direct_delivery_destination_write(&action, runtime, receipts, None)?;
                evaluate_and_log_action(
                    &action,
                    runtime,
                    engine,
                    approvals,
                    receipts,
                    "utility-preflight",
                    "utility action requires approval",
                )?;
            }
            Ok(())
        }
        "cp" => {
            let args: Vec<&String> = cmd.iter().skip(1).filter(|a| !a.starts_with('-')).collect();
            if args.len() >= 2 {
                let src = normalize_runtime_path(runtime, args[0]);
                let dst = normalize_runtime_path(runtime, args[args.len() - 1]);
                let action = ActionRequest {
                    id: uuid::Uuid::new_v4().to_string(),
                    timestamp: Utc::now(),
                    kind: ActionKind::FileWrite,
                    operation: "preflight_cp".to_string(),
                    path: Some(dst),
                    secondary_path: Some(src),
                    host: None,
                    metadata: {
                        let mut metadata = BTreeMap::new();
                        insert_runner_id_metadata(&mut metadata, cmd);
                        metadata
                    },
                    process: ProcessContext {
                        pid: std::process::id(),
                        ppid: None,
                        command: receipt_command.clone(),
                        process_tree: vec![std::process::id()],
                    },
                };
                deny_direct_delivery_destination_write(&action, runtime, receipts, None)?;
                evaluate_and_log_action(
                    &action,
                    runtime,
                    engine,
                    approvals,
                    receipts,
                    "utility-preflight",
                    "utility action requires approval",
                )?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

pub(super) fn preflight_network_egress_actions(
    cmd: &[String],
    runtime: &RuntimeState,
    engine: &PolicyEngine,
    approvals: &ApprovalStore,
    receipts: &ReceiptStore,
) -> Result<()> {
    let hosts = extract_hosts_from_command(cmd);
    if hosts.is_empty() {
        return Ok(());
    }
    let receipt_command = redacted_command_for_receipts(cmd);

    let upload_pattern = looks_like_data_upload_command(cmd);
    let method = infer_http_method(cmd, upload_pattern);

    for host in hosts {
        let mut metadata = BTreeMap::new();
        metadata.insert("argv".to_string(), receipt_command.clone());
        metadata.insert("host".to_string(), host.clone());
        metadata.insert("method".to_string(), method.clone());
        insert_runner_id_metadata(&mut metadata, cmd);
        if upload_pattern {
            metadata.insert("upload_pattern".to_string(), "true".to_string());
        }

        let action = ActionRequest {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            kind: ActionKind::NetworkEgress,
            operation: if upload_pattern {
                "preflight_network_upload".to_string()
            } else {
                "preflight_network_egress".to_string()
            },
            path: None,
            secondary_path: None,
            host: Some(host),
            metadata,
            process: ProcessContext {
                pid: std::process::id(),
                ppid: None,
                command: receipt_command.clone(),
                process_tree: vec![std::process::id()],
            },
        };

        evaluate_and_log_action(
            &action,
            runtime,
            engine,
            approvals,
            receipts,
            if upload_pattern {
                "network-upload-preflight"
            } else {
                "network-preflight"
            },
            if upload_pattern {
                "network upload-style action requires approval"
            } else {
                "network egress requires approval"
            },
        )?;
    }

    Ok(())
}

pub(super) fn preflight_persistence_actions(
    cmd: &[String],
    runtime: &RuntimeState,
    engine: &PolicyEngine,
    approvals: &ApprovalStore,
    receipts: &ReceiptStore,
) -> Result<()> {
    let candidates = detect_persistence_candidates(cmd, runtime);
    if candidates.is_empty() {
        return Ok(());
    }
    let receipt_command = redacted_command_for_receipts(cmd);

    for candidate in candidates {
        let mut metadata = BTreeMap::new();
        metadata.insert("argv".to_string(), receipt_command.clone());
        metadata.insert(
            "persistence_mechanism".to_string(),
            candidate.mechanism.clone(),
        );
        metadata.insert("persistence_scope".to_string(), candidate.scope.clone());
        metadata.insert(
            "target_path".to_string(),
            candidate.path.to_string_lossy().to_string(),
        );
        metadata.insert(
            "requires_elevation".to_string(),
            candidate.requires_elevation.to_string(),
        );
        insert_runner_id_metadata(&mut metadata, cmd);
        metadata.insert(
            "command_summary".to_string(),
            summarize_persistence_command(cmd, &candidate),
        );
        if candidate.suspicious_chain {
            metadata.insert("suspicious_chain".to_string(), "true".to_string());
        }

        let action = ActionRequest {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            kind: ActionKind::Persistence,
            operation: format!(
                "preflight_persistence_{}_{}",
                candidate.mechanism, candidate.scope
            ),
            path: Some(candidate.path),
            secondary_path: None,
            host: None,
            metadata,
            process: ProcessContext {
                pid: std::process::id(),
                ppid: None,
                command: receipt_command.clone(),
                process_tree: vec![std::process::id()],
            },
        };

        evaluate_and_log_action(
            &action,
            runtime,
            engine,
            approvals,
            receipts,
            "persistence-preflight",
            "persistence action requires approval",
        )?;
    }

    Ok(())
}

pub(super) fn preflight_interpreter_exec_actions(
    cmd: &[String],
    runtime: &RuntimeState,
    engine: &PolicyEngine,
    approvals: &ApprovalStore,
    receipts: &ReceiptStore,
) -> Result<()> {
    if cmd.is_empty() {
        return Ok(());
    }
    let receipt_command = redacted_command_for_receipts(cmd);

    let tool_name = Path::new(&cmd[0])
        .file_name()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_else(|| cmd[0].to_ascii_lowercase());

    if !is_interpreter_tool(&tool_name) {
        return Ok(());
    }

    for candidate in interpreter_script_candidates(cmd) {
        let target = normalize_runtime_path(runtime, &candidate);
        let mut metadata = BTreeMap::new();
        metadata.insert("argv".to_string(), receipt_command.clone());
        metadata.insert("interpreter".to_string(), "true".to_string());
        metadata.insert("interpreter_name".to_string(), tool_name.clone());
        insert_runner_id_metadata(&mut metadata, cmd);
        metadata.insert(
            "script_path".to_string(),
            target.to_string_lossy().to_string(),
        );

        let action = ActionRequest {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            kind: ActionKind::Execute,
            operation: "preflight_interpreter_exec".to_string(),
            path: Some(target),
            secondary_path: None,
            host: None,
            metadata,
            process: ProcessContext {
                pid: std::process::id(),
                ppid: None,
                command: receipt_command.clone(),
                process_tree: vec![std::process::id()],
            },
        };

        evaluate_and_log_action(
            &action,
            runtime,
            engine,
            approvals,
            receipts,
            "interpreter-preflight",
            "interpreter execution requires approval",
        )?;
    }

    if looks_like_stream_download_exec(cmd) {
        let executable =
            crate::utils::resolve_command_path(&cmd[0]).unwrap_or_else(|| PathBuf::from(&cmd[0]));
        for host in extract_hosts_from_command(cmd) {
            let mut metadata = BTreeMap::new();
            metadata.insert("argv".to_string(), receipt_command.clone());
            metadata.insert("stream_exec".to_string(), "true".to_string());
            metadata.insert("download_source".to_string(), host.clone());
            insert_runner_id_metadata(&mut metadata, cmd);

            let action = ActionRequest {
                id: uuid::Uuid::new_v4().to_string(),
                timestamp: Utc::now(),
                kind: ActionKind::Execute,
                operation: "preflight_stream_exec".to_string(),
                path: Some(executable.clone()),
                secondary_path: None,
                host: Some(host),
                metadata,
                process: ProcessContext {
                    pid: std::process::id(),
                    ppid: None,
                    command: receipt_command.clone(),
                    process_tree: vec![std::process::id()],
                },
            };

            evaluate_and_log_action(
                &action,
                runtime,
                engine,
                approvals,
                receipts,
                "stream-download-preflight",
                "interpreter stream execution blocked",
            )?;
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct PersistenceCandidate {
    mechanism: String,
    scope: String,
    path: PathBuf,
    requires_elevation: bool,
    suspicious_chain: bool,
}

fn detect_persistence_candidates(
    cmd: &[String],
    runtime: &RuntimeState,
) -> Vec<PersistenceCandidate> {
    if cmd.is_empty() {
        return Vec::new();
    }

    let tool_name = Path::new(&cmd[0])
        .file_name()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_else(|| cmd[0].to_ascii_lowercase());
    let joined = cmd.join(" ");
    let joined_lower = joined.to_ascii_lowercase();

    let mut candidates = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    if tool_name == "crontab" {
        let scope = if has_flag(cmd, "-u") {
            "system"
        } else {
            "user"
        };
        let path = explicit_target_path(cmd)
            .map(|value| normalize_runtime_path(runtime, &value))
            .unwrap_or_else(|| persistence_anchor(runtime, "cron", scope));
        push_persistence_candidate(
            &mut candidates,
            &mut seen,
            path,
            "cron",
            scope,
            scope == "system",
            is_suspicious_persistence_chain(&joined_lower, scope),
        );
    }

    if tool_name == "systemctl" && systemctl_persistence_subcommand(cmd) {
        let scope = if has_flag(cmd, "--user") {
            "user"
        } else {
            "system"
        };
        let path = systemctl_target_path(cmd)
            .map(|value| normalize_runtime_path(runtime, &value))
            .unwrap_or_else(|| persistence_anchor(runtime, "systemd", scope));
        push_persistence_candidate(
            &mut candidates,
            &mut seen,
            path,
            "systemd",
            scope,
            scope == "system",
            is_suspicious_persistence_chain(&joined_lower, scope),
        );
    }

    if !looks_like_write_operation(cmd) {
        return candidates;
    }

    for token in path_tokens_from_command(cmd) {
        let normalized = normalize_runtime_path(runtime, &token);
        if let Some((mechanism, scope)) = classify_persistence_path(&normalized) {
            push_persistence_candidate(
                &mut candidates,
                &mut seen,
                normalized,
                mechanism,
                scope,
                scope == "system",
                is_suspicious_persistence_chain(&joined_lower, scope),
            );
        }
    }

    candidates
}

fn push_persistence_candidate(
    out: &mut Vec<PersistenceCandidate>,
    seen: &mut std::collections::BTreeSet<String>,
    path: PathBuf,
    mechanism: &str,
    scope: &str,
    requires_elevation: bool,
    suspicious_chain: bool,
) {
    let key = format!("{}|{}|{}", mechanism, scope, path.to_string_lossy());
    if !seen.insert(key) {
        return;
    }

    out.push(PersistenceCandidate {
        mechanism: mechanism.to_string(),
        scope: scope.to_string(),
        path,
        requires_elevation,
        suspicious_chain,
    });
}

fn summarize_persistence_command(cmd: &[String], candidate: &PersistenceCandidate) -> String {
    let tool = Path::new(cmd.first().map(|value| value.as_str()).unwrap_or_default())
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    format!(
        "{} {} persistence via {} -> {}",
        candidate.scope,
        candidate.mechanism,
        tool,
        candidate.path.display()
    )
}

fn systemctl_persistence_subcommand(cmd: &[String]) -> bool {
    cmd.iter().any(|value| {
        matches!(
            value.as_str(),
            "enable" | "preset" | "link" | "add-wants" | "add-requires"
        )
    })
}

fn systemctl_target_path(cmd: &[String]) -> Option<String> {
    let mut saw_subcommand = false;
    for token in cmd.iter().skip(1) {
        if token.starts_with('-') {
            continue;
        }
        if !saw_subcommand {
            if matches!(
                token.as_str(),
                "enable" | "preset" | "link" | "add-wants" | "add-requires"
            ) {
                saw_subcommand = true;
            }
            continue;
        }
        if token.contains('/') || token.starts_with("~/") || token.starts_with("$HOME/") {
            return Some(token.clone());
        }
    }
    None
}

fn explicit_target_path(cmd: &[String]) -> Option<String> {
    for token in cmd.iter().skip(1) {
        if token.starts_with('-') || token == "-" {
            continue;
        }
        if token.contains('/') || token.starts_with("~/") || token.starts_with("$HOME/") {
            return Some(token.clone());
        }
    }
    None
}

fn has_flag(cmd: &[String], flag: &str) -> bool {
    cmd.iter().any(|value| value == flag)
}

fn persistence_anchor(runtime: &RuntimeState, mechanism: &str, scope: &str) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| runtime.config.workspace.clone());
    match (mechanism, scope) {
        ("cron", "user") => home.join(".config/cron/user-crontab"),
        ("cron", "system") => PathBuf::from("/etc/cron.d/agent-ruler"),
        ("systemd", "user") => home.join(".config/systemd/user/agent-ruler.service"),
        ("systemd", "system") => PathBuf::from("/etc/systemd/system/agent-ruler.service"),
        ("autostart", "user") => home.join(".config/autostart/agent-ruler.desktop"),
        ("autostart", "system") => PathBuf::from("/etc/xdg/autostart/agent-ruler.desktop"),
        (_, "system") => PathBuf::from("/etc/agent-ruler-persistence"),
        _ => home.join(".config/agent-ruler-persistence"),
    }
}

fn classify_persistence_path(path: &Path) -> Option<(&'static str, &'static str)> {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let value = normalized.to_ascii_lowercase();

    if value.contains("/etc/systemd/system/")
        || value.contains("/usr/lib/systemd/system/")
        || value.contains("/lib/systemd/system/")
    {
        return Some(("systemd", "system"));
    }
    if value.contains("/.config/systemd/user/") {
        return Some(("systemd", "user"));
    }

    if value.starts_with("/etc/cron")
        || value.contains("/etc/cron.")
        || value.contains("/var/spool/cron/")
        || value.ends_with("/etc/crontab")
    {
        return Some(("cron", "system"));
    }
    if value.contains("/.config/cron/") || value.ends_with("/.crontab") {
        return Some(("cron", "user"));
    }

    if value.contains("/etc/xdg/autostart/") || value.contains("/usr/share/applications/") {
        return Some(("autostart", "system"));
    }
    if value.contains("/.config/autostart/") {
        return Some(("autostart", "user"));
    }

    if value.contains("/etc/init.d/") || value.contains("/etc/rc.local") {
        return Some(("other", "system"));
    }

    None
}

fn path_tokens_from_command(cmd: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    for token in cmd {
        for part in token.split_whitespace() {
            let cleaned = clean_shell_token(part);
            if cleaned.is_empty() {
                continue;
            }
            if looks_like_path_token(&cleaned) && seen.insert(cleaned.clone()) {
                out.push(cleaned);
            }
        }
    }

    out
}

fn looks_like_path_token(value: &str) -> bool {
    if value.starts_with("http://") || value.starts_with("https://") {
        return false;
    }

    value.starts_with('/')
        || value.starts_with("~/")
        || value.starts_with("$HOME/")
        || value.contains('/')
}

fn is_suspicious_persistence_chain(command_lower: &str, scope: &str) -> bool {
    let references_temp = command_lower.contains("/tmp/")
        || command_lower.contains("/var/tmp/")
        || command_lower.contains("/dev/shm/");
    let references_download = command_lower.contains("curl ")
        || command_lower.contains("wget ")
        || command_lower.contains("http://")
        || command_lower.contains("https://");

    references_temp && (scope == "system" || references_download)
}

fn extract_hosts_from_command(cmd: &[String]) -> Vec<String> {
    let mut uniq = std::collections::BTreeSet::new();
    let joined = cmd
        .iter()
        .filter(|token| !is_agent_ruler_internal_env_assignment(token))
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ");

    for host in extract_url_hosts(&joined) {
        uniq.insert(host);
    }

    uniq.into_iter().collect()
}

fn is_agent_ruler_internal_env_assignment(token: &str) -> bool {
    let Some((key, _value)) = token.split_once('=') else {
        return false;
    };
    if key.is_empty() {
        return false;
    }
    key.starts_with("AGENT_RULER_")
}

fn extract_url_hosts(text: &str) -> Vec<String> {
    let mut hosts = Vec::new();
    for marker in ["https://", "http://"] {
        let mut offset = 0usize;
        while let Some(pos) = text[offset..].find(marker) {
            let start = offset + pos;
            let rest = &text[start..];
            let end = rest.find(is_url_terminator).unwrap_or(rest.len());
            let raw_url = &rest[..end];
            if let Some(host) = host_from_url(raw_url) {
                hosts.push(host);
            }
            offset = start.saturating_add(marker.len());
            if offset >= text.len() {
                break;
            }
        }
    }
    hosts
}

fn is_url_terminator(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '"' | '\'' | ')' | ']' | '>' | ',')
}

fn host_from_url(url: &str) -> Option<String> {
    let no_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;

    let host_port = no_scheme.split('/').next().unwrap_or_default().trim();

    if host_port.is_empty() {
        return None;
    }

    let host = host_port.split(':').next().unwrap_or_default().trim();
    if host.is_empty() {
        return None;
    }

    Some(host.to_string())
}

fn looks_like_data_upload_command(cmd: &[String]) -> bool {
    let joined = cmd.join(" ").to_ascii_lowercase();
    let has_transfer_tool = joined.contains("curl ") || joined.contains("wget ");

    let stream_upload = joined.contains("| curl") || joined.contains("|wget");
    let explicit_upload = has_transfer_tool
        && (joined.contains("--data")
            || joined.contains("--data-binary")
            || joined.contains(" --upload-file ")
            || joined.contains(" -t "));

    stream_upload || explicit_upload || joined.contains("scp ") || joined.contains("rsync ")
}

fn looks_like_write_operation(cmd: &[String]) -> bool {
    let joined = cmd.join(" ").to_ascii_lowercase();
    joined.contains(" >")
        || joined.contains(">>")
        || joined.contains("tee ")
        || joined.contains(" install ")
        || joined.contains(" cp ")
        || joined.contains(" mv ")
        || joined.contains(" crontab ")
        || joined.contains("systemctl ")
}

fn infer_http_method(cmd: &[String], upload_pattern: bool) -> String {
    for idx in 0..cmd.len() {
        let token = cmd[idx].as_str();
        let takes_value = token == "-X" || token == "--request" || token == "--method";
        if takes_value {
            if let Some(next) = cmd.get(idx + 1) {
                let method = next.trim().to_ascii_uppercase();
                if !method.is_empty() {
                    return method;
                }
            }
        }
    }

    if upload_pattern {
        return "POST".to_string();
    }

    "GET".to_string()
}

fn is_interpreter_tool(name: &str) -> bool {
    matches!(
        name,
        "bash" | "sh" | "zsh" | "dash" | "python" | "python3" | "node" | "perl" | "ruby"
    )
}

fn interpreter_script_candidates(cmd: &[String]) -> Vec<String> {
    let mut candidates = Vec::new();

    let mut idx = 1usize;
    while idx < cmd.len() {
        let arg = cmd[idx].trim();
        if arg == "-c" || arg == "-lc" {
            if let Some(script) = cmd.get(idx + 1) {
                candidates.extend(script_path_candidates(script));
            }
            idx += 2;
            continue;
        }

        if arg.starts_with('-') {
            idx += 1;
            continue;
        }

        if looks_like_script_path(arg) {
            candidates.push(clean_shell_token(arg));
        }
        idx += 1;
    }

    candidates.sort();
    candidates.dedup();
    candidates
}

fn script_path_candidates(script: &str) -> Vec<String> {
    let Some(raw) = script.split_whitespace().next() else {
        return Vec::new();
    };
    let token = clean_shell_token(raw);
    if token.is_empty() || !looks_like_script_path(&token) {
        return Vec::new();
    }
    vec![token]
}

fn looks_like_script_path(value: &str) -> bool {
    if value.starts_with("http://") || value.starts_with("https://") || value.contains("://") {
        return false;
    }

    let pathy = value.starts_with('/')
        || value.starts_with("./")
        || value.starts_with("../")
        || value.contains('/');

    let has_script_ext = [".sh", ".py", ".pl", ".rb", ".js", ".ts", ".ps1"]
        .iter()
        .any(|ext| value.to_ascii_lowercase().ends_with(ext));

    pathy || has_script_ext
}

fn clean_shell_token(token: &str) -> String {
    token
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\'' | ';' | '|' | '&' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | '>' | '<'
            )
        })
        .to_string()
}

fn looks_like_stream_download_exec(cmd: &[String]) -> bool {
    let joined = cmd.join(" ").to_ascii_lowercase();
    let has_url = !extract_hosts_from_command(cmd).is_empty();

    if !has_url {
        return false;
    }

    joined.contains("| bash")
        || joined.contains("| sh")
        || joined.contains("| python")
        || joined.contains("$(curl")
        || joined.contains("$(wget")
}

fn normalize_runtime_path(runtime: &RuntimeState, value: &str) -> PathBuf {
    let normalized = if value.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            home.join(value.trim_start_matches("~/"))
                .to_string_lossy()
                .to_string()
        } else {
            value.to_string()
        }
    } else if value.starts_with("$HOME/") {
        if let Some(home) = dirs::home_dir() {
            home.join(value.trim_start_matches("$HOME/"))
                .to_string_lossy()
                .to_string()
        } else {
            value.to_string()
        }
    } else {
        value.to_string()
    };

    let path = PathBuf::from(normalized);
    if path.is_absolute() {
        path
    } else {
        runtime.config.workspace.join(path)
    }
}

fn deny_direct_delivery_destination_write(
    action: &ActionRequest,
    runtime: &RuntimeState,
    receipts: &ReceiptStore,
    zone: Option<crate::model::Zone>,
) -> Result<()> {
    let Some(target) = action.path.as_ref() else {
        return Ok(());
    };
    if !path_targets_default_delivery(target, runtime) {
        return Ok(());
    }

    let detail = format!(
        "direct writes to user destination are blocked for `{}`; use stage + deliver flow",
        target.display()
    );
    append_receipt(
        receipts,
        runtime,
        action.clone(),
        Decision {
            verdict: Verdict::Deny,
            reason: ReasonCode::DenyUserDataWrite,
            detail: detail.clone(),
            approval_ttl_seconds: None,
        },
        zone,
        None,
        "utility-preflight",
    )?;
    Err(anyhow!(detail))
}

fn path_targets_default_delivery(path: &Path, runtime: &RuntimeState) -> bool {
    path_matches_or_descends(path, &runtime.config.default_delivery_dir)
}

fn path_matches_or_descends(path: &Path, root: &Path) -> bool {
    if path == root || path.starts_with(root) {
        return true;
    }

    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    canonical_path == canonical_root || canonical_path.starts_with(&canonical_root)
}

enum ElevationIntent {
    InstallPackages { packages: Vec<String> },
    Unsupported { detail: String },
}

fn parse_sudo_intent(cmd: &[String]) -> Option<ElevationIntent> {
    if cmd.is_empty() {
        return None;
    }

    let tool = Path::new(&cmd[0])
        .file_name()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_else(|| cmd[0].to_ascii_lowercase());
    if tool != "sudo" {
        return None;
    }

    let mut idx = 1usize;
    while idx < cmd.len() {
        let token = cmd[idx].as_str();
        if token == "--" {
            idx += 1;
            break;
        }
        if token.starts_with('-') {
            idx += 1;
            continue;
        }
        break;
    }

    if idx >= cmd.len() {
        return Some(ElevationIntent::Unsupported {
            detail: "unsupported elevation request: missing elevated command after sudo"
                .to_string(),
        });
    }

    let elevated_tool = Path::new(&cmd[idx])
        .file_name()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_else(|| cmd[idx].to_ascii_lowercase());

    if !matches!(elevated_tool.as_str(), "apt" | "apt-get") {
        return Some(ElevationIntent::Unsupported {
            detail: format!(
                "unsupported elevation request: only 'sudo apt install ...' is currently supported (got sudo {})",
                elevated_tool
            ),
        });
    }

    idx += 1;
    while idx < cmd.len() && cmd[idx].starts_with('-') {
        idx += 1;
    }
    if idx >= cmd.len() || cmd[idx] != "install" {
        return Some(ElevationIntent::Unsupported {
            detail:
                "unsupported elevation request: only install_packages verb is supported for apt"
                    .to_string(),
        });
    }

    idx += 1;
    let mut packages = Vec::new();
    while idx < cmd.len() {
        let token = cmd[idx].trim();
        idx += 1;
        if token.is_empty() || token.starts_with('-') {
            continue;
        }
        if !is_valid_debian_package_name(token) {
            return Some(ElevationIntent::Unsupported {
                detail: format!(
                    "unsupported elevation request: invalid package token '{}'",
                    token
                ),
            });
        }
        packages.push(token.to_string());
    }

    packages.sort();
    packages.dedup();
    Some(ElevationIntent::InstallPackages { packages })
}

fn is_valid_debian_package_name(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(ch) if ch.is_ascii_alphanumeric() => {}
        _ => return false,
    }

    chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-'))
}

fn evaluate_and_log_action(
    action: &ActionRequest,
    runtime: &RuntimeState,
    engine: &PolicyEngine,
    approvals: &ApprovalStore,
    receipts: &ReceiptStore,
    receipt_label: &str,
    pending_note: &str,
) -> Result<()> {
    let (decision, zone) = engine.evaluate(action);
    let decision = finalize_with_approval(decision, approvals, action)?;
    append_receipt(
        receipts,
        runtime,
        action.clone(),
        decision.clone(),
        zone,
        None,
        receipt_label,
    )?;

    match decision.verdict {
        Verdict::Allow => Ok(()),
        Verdict::RequireApproval => {
            let approval = approvals.create_pending(action, &decision, pending_note)?;
            Err(anyhow!(
                "approval required for preflight action {} (id: {})",
                action.operation,
                approval.id
            ))
        }
        Verdict::Deny => Err(anyhow!(
            "preflight action blocked by policy: {:?} ({})",
            decision.reason,
            decision.detail
        )),
        Verdict::Quarantine => {
            if let Some(path) = &action.path {
                if path.exists() {
                    let _ = quarantine_path(&runtime.config.quarantine_dir, path);
                }
            }
            Err(anyhow!("preflight action quarantined by policy"))
        }
    }
}

pub(super) fn finalize_with_approval(
    decision: Decision,
    approvals: &ApprovalStore,
    request: &ActionRequest,
) -> Result<Decision> {
    if decision.verdict != Verdict::RequireApproval {
        return Ok(decision);
    }

    if approvals.has_active_approval_for(request)? {
        return Ok(Decision {
            verdict: Verdict::Allow,
            reason: ReasonCode::AllowedByPolicy,
            detail: "allowed by active approval scope".to_string(),
            approval_ttl_seconds: None,
        });
    }

    Ok(decision)
}

#[cfg(test)]
mod tests {
    use super::extract_hosts_from_command;

    #[test]
    fn extract_hosts_ignores_agent_ruler_internal_base_url_assignment() {
        let cmd = vec![
            "env".to_string(),
            "AGENT_RULER_BASE_URL=http://127.0.0.1:4622".to_string(),
            "opencode".to_string(),
            "run".to_string(),
            "reply with exactly ok".to_string(),
        ];
        assert!(
            extract_hosts_from_command(&cmd).is_empty(),
            "internal Agent Ruler base url env assignment must not be treated as outbound egress"
        );
    }

    #[test]
    fn extract_hosts_still_detects_external_targets_with_internal_env_assignment() {
        let cmd = vec![
            "env".to_string(),
            "AGENT_RULER_BASE_URL=http://127.0.0.1:4622".to_string(),
            "curl".to_string(),
            "https://api.example.com/v1/ping".to_string(),
        ];
        assert_eq!(
            extract_hosts_from_command(&cmd),
            vec!["api.example.com".to_string()]
        );
    }
}
