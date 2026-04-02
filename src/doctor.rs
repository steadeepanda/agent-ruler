//! Runtime diagnostics and safe local repair routines.
//!
//! `agent-ruler doctor` uses these checks to diagnose common runtime failures
//! and apply explicit safe local repairs when `--repair` is requested.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::config::{save_policy, NetworkRules, RuntimeState};
use crate::openclaw_bridge::{ensure_generated_config, generated_config_path};
#[cfg(target_os = "linux")]
use crate::runner::probe_linux_runtime_availability;
use crate::runners::openclaw::{
    enforce_managed_provider_auth_compatibility, inspect_managed_provider_auth_compatibility,
    inspect_managed_telegram_config, ManagedProviderAuthCompatibility,
};
use crate::runners::{apply_runner_env_to_command, RunnerKind};
use crate::utils::resolve_command_path;

const OPENCLAW_BRIDGE_ROUTES_POINTER: &str =
    "plugins.entries.openclaw-agent-ruler-tools.config.approvalBridgeRoutes";
const OPENCLAW_CHANNEL_BRIDGE_CONFIG_FILE_NAME: &str = "channel-bridge.json";
const OPENCLAW_CHANNEL_BRIDGE_LEGACY_CONFIG_FILE_NAME: &str = "openclaw-channel-bridge.json";
const OPENCLAW_CHANNEL_BRIDGE_LOCAL_CONFIG_FILE_NAME: &str = "channel-bridge.local.json";
const OPENCLAW_CHANNEL_BRIDGE_LEGACY_LOCAL_CONFIG_FILE_NAME: &str =
    "openclaw-channel-bridge.local.json";
const TELEGRAM_HOST_BASELINE: &str = "api.telegram.org";
const SLOW_OPENCLAW_CONFIG_THRESHOLD: Duration = Duration::from_secs(2);
const OPENCLAW_CONFIG_COMMAND_TIMEOUT: Duration = Duration::from_secs(12);
const OPENCLAW_CHANNEL_BRIDGE_LOG_FILE_NAME: &str = "openclaw-channel-bridge.log";
const OPENCLAW_GATEWAY_LOG_FILE_NAME: &str = "openclaw-gateway.log";
const BWRAP_APPARMOR_PROFILE_PATH: &str = "/etc/apparmor.d/bwrap";
const BWRAP_APPARMOR_PROFILE: &str = r#"abi <abi/4.0>, include <tunables/global>
profile bwrap /usr/bin/bwrap flags=(unconfined) {
  userns,
  include if exists <local/bwrap>
}
"#;
const BUBBLEWRAP_CURRENT_CHECK_NUMBER: usize = 1;
const RUNTIME_ROOT_CHECK_NUMBER: usize = 2;
const OPENCLAW_BRIDGE_ARTIFACT_CHECK_NUMBER: usize = 3;
const OPENCLAW_ROUTE_SEED_CHECK_NUMBER: usize = 4;
const OPENCLAW_DISCOVERY_CHECK_NUMBER: usize = 5;
const OPENCLAW_PROVIDER_AUTH_CHECK_NUMBER: usize = 6;
const TELEGRAM_ALLOWLIST_CHECK_NUMBER: usize = 7;
const OPENCLAW_TELEGRAM_SYNC_CHECK_NUMBER: usize = 8;
const TELEGRAM_COMMAND_SYNC_FAILURE_HINTS: &[&str] = &["setmycommands", "deletemycommands"];
const TELEGRAM_NETWORK_FAILURE_HINTS: &[&str] = &[
    "network request failed",
    "fetch failed",
    "enotfound",
    "eai_again",
    "etimedout",
    "econnrefused",
    "econnreset",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorStatus {
    Ok,
    Warn,
    Fail,
}

impl DoctorStatus {
    fn severity_rank(self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::Warn => 1,
            Self::Fail => 2,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorCheck {
    pub number: usize,
    pub id: String,
    pub title: String,
    pub status: DoctorStatus,
    pub message: String,
    pub details: Vec<String>,
    pub repairable: bool,
    pub repaired: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub status: DoctorStatus,
    pub repair_requested: bool,
    pub repair_applied: bool,
    pub repair_selection: Option<String>,
    pub summary_line: String,
    pub recommendation: DoctorRecommendation,
    pub checks: Vec<DoctorCheck>,
    pub output: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorRecommendation {
    pub kind: DoctorRecommendationKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorRecommendationKind {
    Continue,
    Repair,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairSelection {
    None,
    All,
    Checks(BTreeSet<usize>),
}

impl RepairSelection {
    fn requested(&self) -> bool {
        !matches!(self, Self::None)
    }

    fn includes(&self, number: usize) -> bool {
        match self {
            Self::None => false,
            Self::All => true,
            Self::Checks(numbers) => numbers.contains(&number),
        }
    }

    fn label(&self) -> Option<String> {
        match self {
            Self::None => None,
            Self::All => Some("all".to_string()),
            Self::Checks(numbers) => Some(
                numbers
                    .iter()
                    .map(|number| number.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            ),
        }
    }
}

impl Default for RepairSelection {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone)]
pub struct DoctorOptions {
    pub repair: RepairSelection,
}

impl Default for DoctorOptions {
    fn default() -> Self {
        Self {
            repair: RepairSelection::None,
        }
    }
}

pub fn run(runtime: &mut RuntimeState, options: DoctorOptions) -> Result<DoctorReport> {
    let mut checks = Vec::new();
    let active_runner = active_runner_kind(runtime);

    checks.push(check_bubblewrap_runtime(
        BUBBLEWRAP_CURRENT_CHECK_NUMBER,
        options.repair.includes(BUBBLEWRAP_CURRENT_CHECK_NUMBER),
    ));
    checks.push(check_runtime_root_consistency(
        RUNTIME_ROOT_CHECK_NUMBER,
        runtime,
    ));
    checks.push(check_bridge_config_artifact(
        OPENCLAW_BRIDGE_ARTIFACT_CHECK_NUMBER,
        runtime,
        active_runner,
        options
            .repair
            .includes(OPENCLAW_BRIDGE_ARTIFACT_CHECK_NUMBER),
    ));
    checks.push(check_openclaw_route_seed(
        OPENCLAW_ROUTE_SEED_CHECK_NUMBER,
        runtime,
        active_runner,
        options.repair.includes(OPENCLAW_ROUTE_SEED_CHECK_NUMBER),
    ));
    checks.push(check_openclaw_config_discovery_latency(
        OPENCLAW_DISCOVERY_CHECK_NUMBER,
        runtime,
        active_runner,
    ));
    checks.push(check_openclaw_provider_guard(
        OPENCLAW_PROVIDER_AUTH_CHECK_NUMBER,
        runtime,
        active_runner,
        options.repair.includes(OPENCLAW_PROVIDER_AUTH_CHECK_NUMBER),
    ));
    checks.push(check_telegram_allowlist_baseline(
        TELEGRAM_ALLOWLIST_CHECK_NUMBER,
        runtime,
        active_runner,
        options.repair.includes(TELEGRAM_ALLOWLIST_CHECK_NUMBER),
    ));
    checks.push(check_openclaw_telegram_command_sync_health(
        OPENCLAW_TELEGRAM_SYNC_CHECK_NUMBER,
        runtime,
        active_runner,
    ));

    let status = checks
        .iter()
        .map(|check| check.status)
        .max_by_key(|status| status.severity_rank())
        .unwrap_or(DoctorStatus::Ok);
    let repair_applied = checks.iter().any(|check| check.repaired);
    let summary_line = build_summary_line(&checks, status);
    let recommendation = build_recommendation(&checks, status, &options.repair, repair_applied);
    let output = render_output(runtime, &checks, status, &options.repair, repair_applied);

    Ok(DoctorReport {
        status,
        repair_requested: options.repair.requested(),
        repair_applied,
        repair_selection: options.repair.label(),
        summary_line,
        recommendation,
        checks,
        output,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveRunner {
    None,
    Openclaw,
    Claudecode,
    Opencode,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
struct BwrapProbeObservation {
    label: String,
    profile: Option<String>,
    result: std::result::Result<(), String>,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
enum BubblewrapFailureMode {
    CurrentLauncher { detail: String },
    HostLikeLauncher { detail: String },
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
struct BubblewrapRepairCapability {
    auto_repairable: bool,
    detail: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct BridgeRouteRuntimeObservation {
    log_path: PathBuf,
    config_source: Option<String>,
    refreshed_source: Option<String>,
    refreshed_routes: Option<usize>,
    inbound_ready: bool,
}

#[cfg(target_os = "linux")]
fn build_bubblewrap_runtime_check(number: usize, repair: bool) -> DoctorCheck {
    let current_profile = read_launcher_profile();
    let current = BwrapProbeObservation {
        label: "current launcher".to_string(),
        profile: current_profile.clone(),
        result: probe_linux_runtime_availability(),
    };
    let host_like = probe_host_launcher_bwrap();
    let mut details = Vec::new();
    details.push(format_probe_observation(&current));
    if let Some(observation) = host_like.as_ref() {
        details.push(format_probe_observation(observation));
    }

    let failure_mode = match (&current.result, host_like.as_ref().map(|item| &item.result)) {
        (Ok(()), Some(Err(detail))) => Some(BubblewrapFailureMode::HostLikeLauncher {
            detail: detail.clone(),
        }),
        (Err(detail), _) => Some(BubblewrapFailureMode::CurrentLauncher {
            detail: detail.clone(),
        }),
        _ => None,
    };

    let repair_capability = bubblewrap_repair_capability(failure_mode.as_ref());
    let repair_supported = repair_capability.auto_repairable;

    if repair && repair_supported {
        if let Some(failure_mode) = failure_mode.as_ref() {
            return repair_bubblewrap_runtime_check(number, &details, failure_mode);
        }
    }

    match (&current.result, host_like.as_ref().map(|item| &item.result)) {
        (Ok(()), Some(Err(_detail))) => DoctorCheck {
            number,
            id: "bubblewrap_runtime".to_string(),
            title: "Bubblewrap runtime probe".to_string(),
            status: DoctorStatus::Fail,
            message: "Host-like terminal launches cannot create Bubblewrap namespaces.".to_string(),
            details: {
                let mut combined = details;
                combined.extend(bubblewrap_manual_remediation_details(&repair_capability));
                combined
            },
            repairable: repair_supported,
            repaired: false,
        },
        (Ok(()), _) => DoctorCheck {
            number,
            id: "bubblewrap_runtime".to_string(),
            title: "Bubblewrap runtime probe".to_string(),
            status: DoctorStatus::Ok,
            message: "Bubblewrap namespace probe succeeded.".to_string(),
            details,
            repairable: false,
            repaired: false,
        },
        (Err(_detail), _) => DoctorCheck {
            number,
            id: "bubblewrap_runtime".to_string(),
            title: "Bubblewrap runtime probe".to_string(),
            status: DoctorStatus::Fail,
            message: "This launcher cannot create Bubblewrap namespaces.".to_string(),
            details: {
                let mut combined = details;
                combined.extend(bubblewrap_manual_remediation_details(&repair_capability));
                combined
            },
            repairable: repair_supported,
            repaired: false,
        },
    }
}

#[cfg(target_os = "linux")]
fn format_probe_observation(observation: &BwrapProbeObservation) -> String {
    let profile = observation.profile.as_deref().unwrap_or("profile unknown");
    match &observation.result {
        Ok(()) => format!(
            "{} (`{profile}`): bubblewrap probe succeeded",
            observation.label
        ),
        Err(detail) => format!(
            "{} (`{profile}`): bubblewrap probe failed: {detail}",
            observation.label
        ),
    }
}

#[cfg(target_os = "linux")]
fn bubblewrap_namespace_repair_supported(failure_mode: &BubblewrapFailureMode) -> bool {
    match failure_mode {
        BubblewrapFailureMode::CurrentLauncher { detail }
        | BubblewrapFailureMode::HostLikeLauncher { detail } => {
            let lowered = detail.to_ascii_lowercase();
            lowered.contains("setting up uid map")
                || lowered.contains("uid map")
                || lowered.contains("permission denied")
                || lowered.contains("operation not permitted")
        }
    }
}

#[cfg(target_os = "linux")]
fn bubblewrap_repair_capability(
    failure_mode: Option<&BubblewrapFailureMode>,
) -> BubblewrapRepairCapability {
    let Some(failure_mode) = failure_mode else {
        return BubblewrapRepairCapability {
            auto_repairable: false,
            detail: None,
        };
    };
    if !bubblewrap_namespace_repair_supported(failure_mode) {
        return BubblewrapRepairCapability {
            auto_repairable: false,
            detail: Some(
                "the current failure mode is not a known safe automated namespace/AppArmor fix."
                    .to_string(),
            ),
        };
    }

    let parser = resolve_command_path("apparmor_parser").or_else(|| {
        let candidate = PathBuf::from("/usr/sbin/apparmor_parser");
        candidate.is_file().then_some(candidate)
    });
    if parser.is_none() {
        return BubblewrapRepairCapability {
            auto_repairable: false,
            detail: Some(
                "this host does not have `apparmor_parser`, so Doctor cannot install or reload `/etc/apparmor.d/bwrap` automatically.".to_string(),
            ),
        };
    }

    let Some(sudo) = resolve_command_path("sudo") else {
        return BubblewrapRepairCapability {
            auto_repairable: false,
            detail: Some(
                "`sudo` is not installed, so Doctor cannot apply the required host AppArmor change automatically."
                    .to_string(),
            ),
        };
    };

    match Command::new(&sudo).args(["-n", "true"]).output() {
        Ok(output) if output.status.success() => BubblewrapRepairCapability {
            auto_repairable: true,
            detail: None,
        },
        Ok(output) => BubblewrapRepairCapability {
            auto_repairable: false,
            detail: Some(format!(
                "this session does not have non-interactive root access (`sudo -n` failed: {}).",
                command_failure_detail(&output).replace('\n', " ")
            )),
        },
        Err(err) => BubblewrapRepairCapability {
            auto_repairable: false,
            detail: Some(format!(
                "Doctor could not verify non-interactive root access for the AppArmor repair: {err}"
            )),
        },
    }
}

#[cfg(target_os = "linux")]
fn repair_bubblewrap_runtime_check(
    number: usize,
    details: &[String],
    failure_mode: &BubblewrapFailureMode,
) -> DoctorCheck {
    let mut combined = details.to_vec();
    match attempt_bubblewrap_namespace_repair() {
        Ok(mut repair_details) => {
            combined.append(&mut repair_details);
            let current_after = BwrapProbeObservation {
                label: "current launcher (after repair)".to_string(),
                profile: read_launcher_profile(),
                result: probe_linux_runtime_availability(),
            };
            combined.push(format_probe_observation(&current_after));
            let host_after = probe_host_launcher_bwrap();
            if let Some(observation) = host_after.as_ref() {
                combined.push(format_probe_observation(observation));
            }

            let still_failing = match (
                &current_after.result,
                host_after.as_ref().map(|item| &item.result),
            ) {
                (Ok(()), Some(Err(detail))) => Some(detail.clone()),
                (Err(detail), _) => Some(detail.clone()),
                _ => None,
            };

            if let Some(detail) = still_failing {
                combined.push(format!("post-repair probe detail: {detail}"));
                return DoctorCheck {
                    number,
                    id: "bubblewrap_runtime".to_string(),
                    title: "Bubblewrap runtime probe".to_string(),
                    status: DoctorStatus::Fail,
                    message:
                        "Bubblewrap namespace repair ran, but confinement is still unavailable."
                            .to_string(),
                    details: combined,
                    repairable: true,
                    repaired: false,
                };
            }

            DoctorCheck {
                number,
                id: "bubblewrap_runtime".to_string(),
                title: "Bubblewrap runtime probe".to_string(),
                status: DoctorStatus::Ok,
                message:
                    "Applied Bubblewrap namespace repair and verified confinement is available."
                        .to_string(),
                details: combined,
                repairable: true,
                repaired: true,
            }
        }
        Err(err) => {
            combined.push(format!("repair attempt failed: {err}"));
            combined.push(
                format!(
                    "automatic Bubblewrap repair installs `{BWRAP_APPARMOR_PROFILE_PATH}` and reloads AppArmor; Doctor could not apply that host change in this session."
                )
                    .to_string(),
            );
            let message = match failure_mode {
                BubblewrapFailureMode::CurrentLauncher { .. } => {
                    "Bubblewrap namespace repair could not be applied for the current launcher."
                }
                BubblewrapFailureMode::HostLikeLauncher { .. } => {
                    "Bubblewrap namespace repair could not be applied for host-like terminal launches."
                }
            };
            DoctorCheck {
                number,
                id: "bubblewrap_runtime".to_string(),
                title: "Bubblewrap runtime probe".to_string(),
                status: DoctorStatus::Fail,
                message: message.to_string(),
                details: combined,
                repairable: true,
                repaired: false,
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn attempt_bubblewrap_namespace_repair() -> Result<Vec<String>> {
    let parser = resolve_command_path("apparmor_parser")
        .or_else(|| {
            let candidate = PathBuf::from("/usr/sbin/apparmor_parser");
            candidate.is_file().then_some(candidate)
        })
        .ok_or_else(|| {
            anyhow!(
                "AppArmor tooling `apparmor_parser` is not installed on this host; install AppArmor utilities or apply the Bubblewrap allow rule manually."
            )
        })?;
    let sudo = resolve_command_path("sudo").ok_or_else(|| {
        anyhow!("`sudo` is not available; root privileges are required for the AppArmor repair.")
    })?;

    let sudo_probe = Command::new(&sudo)
        .args(["-n", "true"])
        .output()
        .context("probe non-interactive sudo access")?;
    if !sudo_probe.status.success() {
        return Err(anyhow!(
            "operator-auth root privileges are required; `sudo -n` is not available in this session ({})",
            command_failure_detail(&sudo_probe)
        ));
    }

    let install = resolve_command_path("install").ok_or_else(|| {
        anyhow!("`install` is not available; Doctor cannot stage the AppArmor profile safely.")
    })?;

    let temp_profile =
        std::env::temp_dir().join(format!("agent-ruler-bwrap-{}.apparmor", std::process::id()));
    fs::write(&temp_profile, BWRAP_APPARMOR_PROFILE).with_context(|| {
        format!(
            "write temporary Bubblewrap AppArmor profile {}",
            temp_profile.display()
        )
    })?;

    let install_output = Command::new(&sudo)
        .arg("-n")
        .arg(&install)
        .args(["-m", "0644"])
        .arg(&temp_profile)
        .arg(BWRAP_APPARMOR_PROFILE_PATH)
        .output()
        .with_context(|| {
            format!(
                "install `{BWRAP_APPARMOR_PROFILE_PATH}` via `{}`",
                install.display()
            )
        })?;
    let _ = fs::remove_file(&temp_profile);
    if !install_output.status.success() {
        return Err(anyhow!(command_failure_detail(&install_output)));
    }

    let parser_output = Command::new(&sudo)
        .arg("-n")
        .arg(&parser)
        .args(["-r", BWRAP_APPARMOR_PROFILE_PATH])
        .output()
        .with_context(|| {
            format!(
                "reload `{BWRAP_APPARMOR_PROFILE_PATH}` via `{}`",
                parser.display()
            )
        })?;
    if !parser_output.status.success() {
        return Err(anyhow!(command_failure_detail(&parser_output)));
    }

    let details = vec![
        format!("installed AppArmor profile: `{BWRAP_APPARMOR_PROFILE_PATH}`"),
        format!(
            "reloaded AppArmor profile with `sudo {} -r {BWRAP_APPARMOR_PROFILE_PATH}`",
            parser.display()
        ),
        format!(
            "manual revert: `sudo {} -R {BWRAP_APPARMOR_PROFILE_PATH}` then `sudo rm {BWRAP_APPARMOR_PROFILE_PATH}`",
            parser.display()
        ),
    ];
    Ok(details)
}

#[cfg(target_os = "linux")]
fn read_launcher_profile() -> Option<String> {
    fs::read_to_string("/proc/self/attr/current")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(target_os = "linux")]
fn probe_host_launcher_bwrap() -> Option<BwrapProbeObservation> {
    if matches!(read_launcher_profile().as_deref(), Some("unconfined")) {
        return None;
    }
    let profile = Command::new("systemd-run")
        .args([
            "--user",
            "--wait",
            "--pipe",
            "cat",
            "/proc/self/attr/current",
        ])
        .output()
        .ok()
        .and_then(|output| {
            if !output.status.success() {
                return None;
            }
            String::from_utf8(output.stdout)
                .ok()
                .and_then(|stdout| stdout.lines().last().map(str::trim).map(ToOwned::to_owned))
        });
    let output = Command::new("systemd-run")
        .args([
            "--user",
            "--wait",
            "--pipe",
            "--property=RuntimeMaxSec=8s",
            "bwrap",
            "--die-with-parent",
            "--new-session",
            "--ro-bind",
            "/",
            "/",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
            "--tmpfs",
            "/run",
            "--chdir",
            "/",
            "--",
            "/bin/true",
        ])
        .output()
        .ok()?;
    let result = if output.status.success() {
        Ok(())
    } else {
        Err(sanitize_systemd_run_probe_detail(&command_failure_detail(
            &output,
        )))
    };
    Some(BwrapProbeObservation {
        label: "host-like launcher via `systemd-run --user`".to_string(),
        profile,
        result,
    })
}

#[cfg(target_os = "linux")]
fn bubblewrap_manual_remediation_details(
    repair_capability: &BubblewrapRepairCapability,
) -> Vec<String> {
    let mut details = Vec::new();
    details.push(
        "fix: install `/etc/apparmor.d/bwrap` with an AppArmor profile that leaves `/usr/bin/bwrap` unconfined and allows `userns`, then reload AppArmor."
            .to_string(),
    );
    details.push("privileges: requires operator-auth root on the host.".to_string());
    if repair_capability.auto_repairable {
        details.push(
            "automatic repair: available in this session with `agent-ruler doctor --repair 1`."
                .to_string(),
        );
    } else if let Some(detail) = repair_capability.detail.as_ref() {
        details.push(format!(
            "automatic repair: unavailable in this session because {detail}"
        ));
    }
    details
}

#[cfg(target_os = "linux")]
fn sanitize_systemd_run_probe_detail(detail: &str) -> String {
    let filtered: Vec<&str> = detail
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !is_systemd_run_probe_noise_line(line))
        .collect();
    if filtered.is_empty() {
        detail.trim().to_string()
    } else {
        filtered.join("\n")
    }
}

#[cfg(target_os = "linux")]
fn is_systemd_run_probe_noise_line(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    lowered.starts_with("running as unit:")
        || lowered.starts_with("finished with result:")
        || lowered.starts_with("main processes terminated with:")
        || lowered.starts_with("service runtime:")
        || lowered.starts_with("cpu time consumed:")
        || lowered.starts_with("memory peak:")
        || lowered.starts_with("memory swap peak:")
}

fn check_bubblewrap_runtime(number: usize, repair: bool) -> DoctorCheck {
    #[cfg(not(target_os = "linux"))]
    {
        return DoctorCheck {
            number,
            id: "bubblewrap_runtime".to_string(),
            title: "Bubblewrap runtime probe".to_string(),
            status: DoctorStatus::Warn,
            message: "Bubblewrap confinement checks are Linux-only in this build.".to_string(),
            details: Vec::new(),
            repairable: false,
            repaired: false,
        };
    }

    #[cfg(target_os = "linux")]
    {
        build_bubblewrap_runtime_check(number, repair)
    }
}

fn check_runtime_root_consistency(number: usize, runtime: &RuntimeState) -> DoctorCheck {
    let mut details = Vec::new();
    let mut fail = false;

    details.push(format!(
        "runtime root: {}",
        runtime.config.runtime_root.display()
    ));
    if !runtime.config.runtime_root.exists() {
        fail = true;
        details.push("runtime root path is missing on disk.".to_string());
    }

    if let Some(runner) = runtime.config.runner.as_ref() {
        details.push(format!("runner kind: {}", runner.kind.display_name()));
        details.push(format!("managed home: {}", runner.managed_home.display()));
        details.push(format!(
            "managed workspace: {}",
            runner.managed_workspace.display()
        ));

        if !runner.managed_home.exists() {
            fail = true;
            details.push("managed home path is missing.".to_string());
        }
        if !runner.managed_workspace.exists() {
            fail = true;
            details.push("managed workspace path is missing.".to_string());
        }
        if !runner
            .managed_home
            .starts_with(&runtime.config.runtime_root)
        {
            fail = true;
            details.push("managed home is outside runtime root.".to_string());
        }
        if !runner
            .managed_workspace
            .starts_with(&runtime.config.runtime_root)
        {
            fail = true;
            details.push("managed workspace is outside runtime root.".to_string());
        }
    } else {
        details.push("runner is not configured for this runtime.".to_string());
    }

    DoctorCheck {
        number,
        id: "runtime_root_consistency".to_string(),
        title: "Runtime root / managed path consistency".to_string(),
        status: if fail {
            DoctorStatus::Fail
        } else {
            DoctorStatus::Ok
        },
        message: if fail {
            "Managed runtime paths are inconsistent.".to_string()
        } else {
            "Managed runtime paths are consistent.".to_string()
        },
        details,
        repairable: false,
        repaired: false,
    }
}

fn check_bridge_config_artifact(
    number: usize,
    runtime: &RuntimeState,
    active_runner: ActiveRunner,
    repair: bool,
) -> DoctorCheck {
    if active_runner != ActiveRunner::Openclaw {
        return openclaw_not_applicable(
            number,
            runtime,
            "openclaw_bridge_config_artifact",
            "OpenClaw generated bridge config artifact",
            "skipped artifact check",
        );
    }

    let config_path = generated_config_path(runtime);
    if config_path.exists() {
        return DoctorCheck {
            number,
            id: "openclaw_bridge_config_artifact".to_string(),
            title: "OpenClaw generated bridge config artifact".to_string(),
            status: DoctorStatus::Ok,
            message: "Generated bridge config artifact is present.".to_string(),
            details: vec![format!("path: {}", config_path.display())],
            repairable: false,
            repaired: false,
        };
    }

    if repair {
        return match ensure_generated_config(runtime) {
            Ok(_view) => DoctorCheck {
                number,
                id: "openclaw_bridge_config_artifact".to_string(),
                title: "OpenClaw generated bridge config artifact".to_string(),
                status: DoctorStatus::Ok,
                message: "Generated missing bridge config artifact.".to_string(),
                details: vec![format!("path: {}", config_path.display())],
                repairable: true,
                repaired: true,
            },
            Err(err) => DoctorCheck {
                number,
                id: "openclaw_bridge_config_artifact".to_string(),
                title: "OpenClaw generated bridge config artifact".to_string(),
                status: DoctorStatus::Fail,
                message: "Failed to generate missing bridge config artifact.".to_string(),
                details: vec![err.to_string()],
                repairable: true,
                repaired: false,
            },
        };
    }

    DoctorCheck {
        number,
        id: "openclaw_bridge_config_artifact".to_string(),
        title: "OpenClaw generated bridge config artifact".to_string(),
        status: DoctorStatus::Fail,
        message: "Generated bridge config artifact is missing.".to_string(),
        details: vec![
            format!("expected path: {}", config_path.display()),
            "run `agent-ruler doctor --repair` to regenerate the managed artifact.".to_string(),
        ],
        repairable: true,
        repaired: false,
    }
}

fn check_openclaw_route_seed(
    number: usize,
    runtime: &RuntimeState,
    active_runner: ActiveRunner,
    repair: bool,
) -> DoctorCheck {
    if active_runner != ActiveRunner::Openclaw {
        return openclaw_not_applicable(
            number,
            runtime,
            "openclaw_route_seed",
            "OpenClaw managed bridge route seed",
            "skipped managed route seed check",
        );
    }

    match managed_openclaw_bridge_routes_count(runtime) {
        Ok(Some(count)) if count > 0 => DoctorCheck {
            number,
            id: "openclaw_route_seed".to_string(),
            title: "OpenClaw managed bridge route seed".to_string(),
            status: DoctorStatus::Ok,
            message: format!("Managed bridge routes are present ({count} route(s))."),
            details: vec![format!("pointer: {OPENCLAW_BRIDGE_ROUTES_POINTER}")],
            repairable: false,
            repaired: false,
        },
        Ok(_) => {
            let legacy = find_legacy_bridge_routes(runtime);
            match legacy {
                Ok(Some((source_path, routes))) if !routes.is_empty() => {
                    if repair {
                        match write_managed_openclaw_bridge_routes(runtime, &routes) {
                            Ok(()) => DoctorCheck {
                                number,
                                id: "openclaw_route_seed".to_string(),
                                title: "OpenClaw managed bridge route seed".to_string(),
                                status: DoctorStatus::Ok,
                                message: format!(
                                    "Seeded managed bridge routes from legacy config ({} route(s)).",
                                    routes.len()
                                ),
                                details: vec![
                                    format!("source: {}", source_path.display()),
                                    format!("pointer: {OPENCLAW_BRIDGE_ROUTES_POINTER}"),
                                ],
                                repairable: true,
                                repaired: true,
                            },
                            Err(err) => DoctorCheck {
                                number,
                                id: "openclaw_route_seed".to_string(),
                                title: "OpenClaw managed bridge route seed".to_string(),
                                status: DoctorStatus::Fail,
                                message:
                                    "Managed bridge routes are missing and route seeding failed."
                                        .to_string(),
                                details: vec![
                                    format!("source: {}", source_path.display()),
                                    err.to_string(),
                                ],
                                repairable: true,
                                repaired: false,
                            },
                        }
                    } else {
                        DoctorCheck {
                            number,
                            id: "openclaw_route_seed".to_string(),
                            title: "OpenClaw managed bridge route seed".to_string(),
                            status: DoctorStatus::Fail,
                            message:
                                "Managed bridge routes are missing but legacy routes were found."
                                    .to_string(),
                            details: vec![
                                format!("source: {}", source_path.display()),
                                format!("pointer: {OPENCLAW_BRIDGE_ROUTES_POINTER}"),
                                "run `agent-ruler doctor --repair` to seed managed routes."
                                    .to_string(),
                            ],
                            repairable: true,
                            repaired: false,
                        }
                    }
                }
                Ok(_) => match discover_openclaw_channel_default_routes(runtime) {
                    Ok(discovered) if !discovered.is_empty() => {
                        if repair {
                            match write_managed_openclaw_bridge_routes(runtime, &discovered) {
                                Ok(()) => DoctorCheck {
                                    number,
                                    id: "openclaw_route_seed".to_string(),
                                    title: "OpenClaw managed bridge route seed".to_string(),
                                    status: DoctorStatus::Ok,
                                    message: format!(
                                        "Persisted {count} autodiscovered bridge route(s) from enabled channel defaults.",
                                        count = discovered.len()
                                    ),
                                    details: vec![
                                        format!("pointer: {OPENCLAW_BRIDGE_ROUTES_POINTER}"),
                                        "source: enabled `channels.*` config + `credentials/*-allowFrom.json`"
                                            .to_string(),
                                    ],
                                    repairable: true,
                                    repaired: true,
                                },
                                Err(err) => DoctorCheck {
                                    number,
                                    id: "openclaw_route_seed".to_string(),
                                    title: "OpenClaw managed bridge route seed".to_string(),
                                    status: DoctorStatus::Fail,
                                    message:
                                        "Managed bridge routes are missing and autodiscovery sync failed."
                                            .to_string(),
                                    details: vec![
                                        format!("pointer: {OPENCLAW_BRIDGE_ROUTES_POINTER}"),
                                        err.to_string(),
                                    ],
                                    repairable: true,
                                    repaired: false,
                                },
                            }
                        } else {
                            DoctorCheck {
                                number,
                                id: "openclaw_route_seed".to_string(),
                                title: "OpenClaw managed bridge route seed".to_string(),
                                status: DoctorStatus::Warn,
                                message:
                                    "Managed bridge routes are missing, but channel-default autodiscovery can seed them."
                                        .to_string(),
                                details: vec![
                                    format!("pointer: {OPENCLAW_BRIDGE_ROUTES_POINTER}"),
                                    format!(
                                        "discovered route candidates: {}",
                                        discovered.len()
                                    ),
                                    "autodiscovery runs during bridge startup and route refresh, not only after user messages."
                                        .to_string(),
                                ],
                                repairable: true,
                                repaired: false,
                            }
                        }
                    }
                    Ok(_) => {
                        let observation = inspect_openclaw_bridge_route_runtime(runtime);
                        if let Some(observation) = observation.as_ref() {
                            if observation.inbound_ready
                                && observation.refreshed_source.as_deref()
                                    == Some("openclaw_unconfigured")
                            {
                                let mut details = vec![
                                    format!("pointer: {OPENCLAW_BRIDGE_ROUTES_POINTER}"),
                                    format!(
                                        "bridge runtime: {} reports source={} routes={}",
                                        observation.log_path.display(),
                                        observation
                                            .refreshed_source
                                            .as_deref()
                                            .unwrap_or("unknown"),
                                        observation.refreshed_routes.unwrap_or(0)
                                    ),
                                    "bridge startup is working, but approval delivery remains deferred until a channel `allowFrom` sender is configured or paired."
                                        .to_string(),
                                    "safe repair is only possible after OpenClaw stores route candidates in `channels.*.allowFrom` or `credentials/*-allowFrom.json`."
                                        .to_string(),
                                ];
                                if let Some(source) = observation.config_source.as_deref() {
                                    details.push(format!("bridge startup config source: {source}"));
                                }
                                return DoctorCheck {
                                    number,
                                    id: "openclaw_route_seed".to_string(),
                                    title: "OpenClaw managed bridge route seed".to_string(),
                                    status: DoctorStatus::Warn,
                                    message:
                                        "Managed bridge routes are missing; the active bridge is running in unconfigured autodiscovery mode."
                                            .to_string(),
                                    details,
                                    repairable: false,
                                    repaired: false,
                                };
                            }
                            if observation.inbound_ready
                                && observation.refreshed_routes.unwrap_or(0) > 0
                            {
                                let source = observation
                                    .refreshed_source
                                    .as_deref()
                                    .unwrap_or("runtime bridge");
                                return DoctorCheck {
                                    number,
                                    id: "openclaw_route_seed".to_string(),
                                    title: "OpenClaw managed bridge route seed".to_string(),
                                    status: DoctorStatus::Ok,
                                    message: format!(
                                        "Managed bridge routes are missing, but the active bridge already resolved {} route(s) from {source}.",
                                        observation.refreshed_routes.unwrap_or(0)
                                    ),
                                    details: vec![
                                        format!("pointer: {OPENCLAW_BRIDGE_ROUTES_POINTER}"),
                                        format!(
                                            "bridge runtime log: {}",
                                            observation.log_path.display()
                                        ),
                                    ],
                                    repairable: false,
                                    repaired: false,
                                };
                            }
                        }

                        DoctorCheck {
                            number,
                            id: "openclaw_route_seed".to_string(),
                            title: "OpenClaw managed bridge route seed".to_string(),
                            status: DoctorStatus::Warn,
                            message:
                                "Managed bridge routes are missing and no legacy or autodiscovery seed source was found."
                                    .to_string(),
                            details: vec![
                                format!("pointer: {OPENCLAW_BRIDGE_ROUTES_POINTER}"),
                                "OpenClaw may still attempt channel-default autodiscovery at bridge startup if channel config changes later."
                                    .to_string(),
                            ],
                            repairable: false,
                            repaired: false,
                        }
                    }
                    Err(err) => DoctorCheck {
                        number,
                        id: "openclaw_route_seed".to_string(),
                        title: "OpenClaw managed bridge route seed".to_string(),
                        status: DoctorStatus::Fail,
                        message: "Failed to inspect channel-default bridge autodiscovery sources."
                            .to_string(),
                        details: vec![err.to_string()],
                        repairable: false,
                        repaired: false,
                    },
                },
                Err(err) => DoctorCheck {
                    number,
                    id: "openclaw_route_seed".to_string(),
                    title: "OpenClaw managed bridge route seed".to_string(),
                    status: DoctorStatus::Fail,
                    message: "Failed to inspect managed bridge route state.".to_string(),
                    details: vec![err.to_string()],
                    repairable: false,
                    repaired: false,
                },
            }
        }
        Err(err) => DoctorCheck {
            number,
            id: "openclaw_route_seed".to_string(),
            title: "OpenClaw managed bridge route seed".to_string(),
            status: DoctorStatus::Fail,
            message: "Failed to query managed OpenClaw bridge routes.".to_string(),
            details: vec![err.to_string()],
            repairable: false,
            repaired: false,
        },
    }
}

fn check_openclaw_config_discovery_latency(
    number: usize,
    runtime: &RuntimeState,
    active_runner: ActiveRunner,
) -> DoctorCheck {
    if active_runner != ActiveRunner::Openclaw {
        return openclaw_not_applicable(
            number,
            runtime,
            "openclaw_config_discovery_latency",
            "OpenClaw config discovery latency",
            "skipped discovery latency check",
        );
    }

    let routes_probe = timed_openclaw_config_probe(runtime, OPENCLAW_BRIDGE_ROUTES_POINTER);
    let channels_probe = timed_openclaw_config_probe(runtime, "channels");
    let details = vec![
        describe_config_probe(
            OPENCLAW_BRIDGE_ROUTES_POINTER,
            &routes_probe,
            "route pointer is optional; channel-default autodiscovery may still work",
        ),
        describe_config_probe(
            "channels",
            &channels_probe,
            "channels config is used for route fallback",
        ),
    ];

    if let Some(failure) = first_probe_failure(&routes_probe, &channels_probe) {
        return DoctorCheck {
            number,
            id: "openclaw_config_discovery_latency".to_string(),
            title: "OpenClaw config discovery latency".to_string(),
            status: DoctorStatus::Fail,
            message: "Failed to probe OpenClaw config reads used during bridge startup."
                .to_string(),
            details: vec![failure, details[0].clone(), details[1].clone()],
            repairable: false,
            repaired: false,
        };
    }

    let slow_probe = routes_probe
        .elapsed()
        .is_some_and(|latency| latency >= SLOW_OPENCLAW_CONFIG_THRESHOLD)
        || channels_probe
            .elapsed()
            .is_some_and(|latency| latency >= SLOW_OPENCLAW_CONFIG_THRESHOLD);
    if slow_probe {
        return DoctorCheck {
            number,
            id: "openclaw_config_discovery_latency".to_string(),
            title: "OpenClaw config discovery latency".to_string(),
            status: DoctorStatus::Warn,
            message: "OpenClaw config reads are slow; bridge startup timeouts are more likely on cold starts.".to_string(),
            details: vec![
                details[0].clone(),
                details[1].clone(),
                "consider seeding managed bridge routes to reduce startup auto-discovery work.".to_string(),
            ],
            repairable: false,
            repaired: false,
        };
    }

    DoctorCheck {
        number,
        id: "openclaw_config_discovery_latency".to_string(),
        title: "OpenClaw config discovery latency".to_string(),
        status: DoctorStatus::Ok,
        message: "OpenClaw config discovery latency is within expected range.".to_string(),
        details,
        repairable: false,
        repaired: false,
    }
}

fn check_telegram_allowlist_baseline(
    number: usize,
    runtime: &mut RuntimeState,
    active_runner: ActiveRunner,
    repair: bool,
) -> DoctorCheck {
    if active_runner != ActiveRunner::Openclaw {
        return openclaw_not_applicable(
            number,
            runtime,
            "telegram_allowlist_baseline",
            "Telegram allowlist baseline",
            "skipped Telegram baseline check",
        );
    }

    let managed_home = managed_openclaw_home(runtime);
    let telegram = match inspect_managed_telegram_config(&managed_home) {
        Ok(value) => value,
        Err(err) => {
            return DoctorCheck {
                number,
                id: "telegram_allowlist_baseline".to_string(),
                title: "Telegram allowlist baseline".to_string(),
                status: DoctorStatus::Fail,
                message: "Unable to inspect managed Telegram channel configuration.".to_string(),
                details: vec![err.to_string()],
                repairable: false,
                repaired: false,
            }
        }
    };

    if !telegram.enabled {
        return DoctorCheck {
            number,
            id: "telegram_allowlist_baseline".to_string(),
            title: "Telegram allowlist baseline".to_string(),
            status: DoctorStatus::Ok,
            message: "Telegram channel is disabled; allowlist baseline is not required."
                .to_string(),
            details: Vec::new(),
            repairable: false,
            repaired: false,
        };
    }

    if !telegram.token_present {
        return DoctorCheck {
            number,
            id: "telegram_allowlist_baseline".to_string(),
            title: "Telegram allowlist baseline".to_string(),
            status: DoctorStatus::Warn,
            message:
                "Telegram is enabled but no managed bot token is configured in OpenClaw config."
                    .to_string(),
            details: vec![
                "configure `channels.telegram.botToken` (or `channels.telegram.token`) in managed OpenClaw config.".to_string(),
            ],
            repairable: false,
            repaired: false,
        };
    }

    if network_policy_allows_host(&runtime.policy.rules.network, TELEGRAM_HOST_BASELINE) {
        return DoctorCheck {
            number,
            id: "telegram_allowlist_baseline".to_string(),
            title: "Telegram allowlist baseline".to_string(),
            status: DoctorStatus::Ok,
            message: format!(
                "Network policy explicitly allows `{TELEGRAM_HOST_BASELINE}` for Telegram operations."
            ),
            details: Vec::new(),
            repairable: false,
            repaired: false,
        };
    }

    if repair {
        let mut hosts = runtime.policy.rules.network.allowlist_hosts.clone();
        hosts.push(TELEGRAM_HOST_BASELINE.to_string());
        hosts.sort();
        hosts.dedup();
        runtime.policy.rules.network.allowlist_hosts = hosts;
        match save_policy(&runtime.config.policy_file, &runtime.policy) {
            Ok(()) => DoctorCheck {
                number,
                id: "telegram_allowlist_baseline".to_string(),
                title: "Telegram allowlist baseline".to_string(),
                status: DoctorStatus::Ok,
                message: format!(
                    "Added `{TELEGRAM_HOST_BASELINE}` to network allowlist presets for Telegram baseline."
                ),
                details: vec![format!(
                    "policy file updated: {}",
                    runtime.config.policy_file.display()
                )],
                repairable: true,
                repaired: true,
            },
            Err(err) => DoctorCheck {
                number,
                id: "telegram_allowlist_baseline".to_string(),
                title: "Telegram allowlist baseline".to_string(),
                status: DoctorStatus::Fail,
                message: format!(
                    "Network policy blocks `{TELEGRAM_HOST_BASELINE}` and repair failed."
                ),
                details: vec![err.to_string()],
                repairable: true,
                repaired: false,
            },
        }
    } else {
        DoctorCheck {
            number,
            id: "telegram_allowlist_baseline".to_string(),
            title: "Telegram allowlist baseline".to_string(),
            status: DoctorStatus::Fail,
            message: format!(
                "Network policy does not currently allow `{TELEGRAM_HOST_BASELINE}` while Telegram is enabled."
            ),
            details: vec![
                "run `agent-ruler doctor --repair` to add the baseline allowlist host."
                    .to_string(),
            ],
            repairable: true,
            repaired: false,
        }
    }
}

fn check_openclaw_provider_guard(
    number: usize,
    runtime: &RuntimeState,
    active_runner: ActiveRunner,
    repair: bool,
) -> DoctorCheck {
    if active_runner != ActiveRunner::Openclaw {
        return openclaw_not_applicable(
            number,
            runtime,
            "openclaw_provider_guard",
            "OpenClaw provider/auth compatibility",
            "skipped provider/auth compatibility check",
        );
    }

    let managed_home = managed_openclaw_home(runtime);
    let compatibility = match inspect_managed_provider_auth_compatibility(&managed_home) {
        Ok(value) => value,
        Err(err) => {
            return DoctorCheck {
                number,
                id: "openclaw_provider_guard".to_string(),
                title: "OpenClaw provider/auth compatibility".to_string(),
                status: DoctorStatus::Fail,
                message: "Unable to inspect managed OpenClaw provider/auth compatibility."
                    .to_string(),
                details: vec![err.to_string()],
                repairable: false,
                repaired: false,
            }
        }
    };

    build_provider_auth_check(number, compatibility, repair, &managed_home)
}

fn check_openclaw_telegram_command_sync_health(
    number: usize,
    runtime: &RuntimeState,
    active_runner: ActiveRunner,
) -> DoctorCheck {
    if active_runner != ActiveRunner::Openclaw {
        return openclaw_not_applicable(
            number,
            runtime,
            "openclaw_telegram_command_sync",
            "OpenClaw Telegram command-sync health",
            "skipped Telegram command-sync log check",
        );
    }

    let managed_home = managed_openclaw_home(runtime);
    let telegram = match inspect_managed_telegram_config(&managed_home) {
        Ok(value) => value,
        Err(err) => {
            return DoctorCheck {
                number,
                id: "openclaw_telegram_command_sync".to_string(),
                title: "OpenClaw Telegram command-sync health".to_string(),
                status: DoctorStatus::Warn,
                message: "Unable to inspect Telegram config before command-sync health check."
                    .to_string(),
                details: vec![err.to_string()],
                repairable: false,
                repaired: false,
            }
        }
    };
    if !telegram.enabled {
        return DoctorCheck {
            number,
            id: "openclaw_telegram_command_sync".to_string(),
            title: "OpenClaw Telegram command-sync health".to_string(),
            status: DoctorStatus::Ok,
            message: "Telegram channel is disabled; command-sync check is not required."
                .to_string(),
            details: Vec::new(),
            repairable: false,
            repaired: false,
        };
    }

    let mut checked_logs = Vec::new();
    let mut recent_signal = None;
    for log_path in openclaw_logs_for_diagnostics(runtime) {
        match fs::read_to_string(&log_path) {
            Ok(raw) => {
                checked_logs.push(format!("log: {}", log_path.display()));
                let recent = tail_lines(&raw, 240).join("\n").to_ascii_lowercase();
                let mentions_sync = TELEGRAM_COMMAND_SYNC_FAILURE_HINTS
                    .iter()
                    .any(|hint| recent.contains(hint));
                let mentions_network = TELEGRAM_NETWORK_FAILURE_HINTS
                    .iter()
                    .any(|hint| recent.contains(hint));
                if mentions_sync && mentions_network {
                    recent_signal = Some(log_path);
                    break;
                }
            }
            Err(err) => checked_logs.push(format!("log: {} ({err})", log_path.display())),
        }
    }

    if let Some(log_path) = recent_signal {
        return DoctorCheck {
            number,
            id: "openclaw_telegram_command_sync".to_string(),
            title: "OpenClaw Telegram command-sync health".to_string(),
            status: DoctorStatus::Ok,
            message: "Recent Telegram command-sync network failures were detected, but this signal is retryable and non-fatal on its own.".to_string(),
            details: vec![
                "if Telegram approvals/messages are still flowing, no immediate action is required.".to_string(),
                "if delivery is failing, verify token + allowlist baseline and retry gateway launch.".to_string(),
                format!("log: {}", log_path.display()),
            ],
            repairable: false,
            repaired: false,
        };
    }

    DoctorCheck {
        number,
        id: "openclaw_telegram_command_sync".to_string(),
        title: "OpenClaw Telegram command-sync health".to_string(),
        status: DoctorStatus::Ok,
        message: "No recent Telegram command-sync network failure pattern detected.".to_string(),
        details: checked_logs,
        repairable: false,
        repaired: false,
    }
}

fn build_summary_line(checks: &[DoctorCheck], status: DoctorStatus) -> String {
    let ok_count = checks
        .iter()
        .filter(|check| check.status == DoctorStatus::Ok)
        .count();
    let warn_count = checks
        .iter()
        .filter(|check| check.status == DoctorStatus::Warn)
        .count();
    let fail_count = checks
        .iter()
        .filter(|check| check.status == DoctorStatus::Fail)
        .count();
    format!(
        "summary: status={:?} ok={} warn={} fail={}",
        status, ok_count, warn_count, fail_count
    )
}

fn build_recommendation(
    checks: &[DoctorCheck],
    status: DoctorStatus,
    repair_selection: &RepairSelection,
    repair_applied: bool,
) -> DoctorRecommendation {
    let repairable_numbers = checks
        .iter()
        .filter(|check| check.repairable && !check.repaired)
        .map(|check| check.number.to_string())
        .collect::<Vec<_>>();

    if status == DoctorStatus::Ok {
        if repair_applied {
            return DoctorRecommendation {
                kind: DoctorRecommendationKind::Continue,
                message:
                    "Doctor applied the selected safe local repairs successfully. You can continue."
                        .to_string(),
            };
        }
        return DoctorRecommendation {
            kind: DoctorRecommendationKind::Continue,
            message: "Doctor did not find any blocking runtime issues. You can continue."
                .to_string(),
        };
    }

    if !repair_selection.requested() && !repairable_numbers.is_empty() {
        let targets = repairable_numbers.join(",");
        return DoctorRecommendation {
            kind: DoctorRecommendationKind::Repair,
            message: format!(
                "Run `agent-ruler doctor --repair {targets}` to apply the available safe local fixes."
            ),
        };
    }

    if repair_selection.requested() && !repair_applied {
        return DoctorRecommendation {
            kind: DoctorRecommendationKind::Manual,
            message:
                "Automatic repair did not clear the selected issue(s). Inspect the check details, logs, and documentation, then report the issue if needed."
                    .to_string(),
        };
    }

    DoctorRecommendation {
        kind: DoctorRecommendationKind::Manual,
        message:
            "Inspect the reported check details, relevant logs, and documentation before continuing."
                .to_string(),
    }
}

fn command_output_with_timeout(command: &mut Command, timeout: Duration) -> Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn {:?}", command))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .context("collect command output after process exit");
            }
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(anyhow!("timed out after {:.0}s", timeout.as_secs_f64()));
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(err) => return Err(err).context("wait for command completion"),
        }
    }
}

fn render_output(
    runtime: &RuntimeState,
    checks: &[DoctorCheck],
    status: DoctorStatus,
    repair_selection: &RepairSelection,
    repair_applied: bool,
) -> String {
    let mut lines = Vec::new();
    lines.push("Agent Ruler Doctor".to_string());
    lines.push(format!(
        "runtime root: {}",
        runtime.config.runtime_root.display()
    ));
    lines.push(build_summary_line(checks, status));
    if let Some(label) = repair_selection.label() {
        lines.push(format!("repair selection: {label}"));
    }
    lines.push(String::new());

    for check in checks {
        let status_label = match check.status {
            DoctorStatus::Ok => "ok",
            DoctorStatus::Warn => "warn",
            DoctorStatus::Fail => "fail",
        };
        let repaired_label = if check.repaired { " (repaired)" } else { "" };
        lines.push(format!(
            "{}. [{status_label}] {}{}",
            check.number, check.title, repaired_label
        ));
        lines.push(format!("  {}", check.message));
        for detail in &check.details {
            lines.push(format!("  - {}", detail));
        }
        if check.repairable && !repair_selection.requested() && !check.repaired {
            lines.push(format!(
                "  - repair available: `agent-ruler doctor --repair {}`",
                check.number
            ));
        }
        if repair_selection.requested()
            && repair_selection.includes(check.number)
            && !check.repairable
            && !check.repaired
        {
            lines.push(
                "  - repair skipped: no safe automated repair is available for this check."
                    .to_string(),
            );
        }
        lines.push(String::new());
    }

    if !repair_selection.requested()
        && checks.iter().any(|check| check.repairable)
        && status != DoctorStatus::Ok
    {
        let repairable = checks
            .iter()
            .filter(|check| check.repairable)
            .map(|check| check.number.to_string())
            .collect::<Vec<_>>()
            .join(",");
        lines.push(format!(
            "repair hint: Run: agent-ruler doctor --repair {repairable}"
        ));
    }
    if repair_selection.requested() && repair_applied {
        lines.push("repair result: safe local repairs were applied.".to_string());
    }

    lines.join("\n")
}

fn active_runner_kind(runtime: &RuntimeState) -> ActiveRunner {
    match runtime.config.runner.as_ref().map(|runner| runner.kind) {
        Some(RunnerKind::Openclaw) => ActiveRunner::Openclaw,
        Some(RunnerKind::Claudecode) => ActiveRunner::Claudecode,
        Some(RunnerKind::Opencode) => ActiveRunner::Opencode,
        None => ActiveRunner::None,
    }
}

fn runner_scope_description(runtime: &RuntimeState) -> String {
    match runtime.config.runner.as_ref() {
        Some(runner) => format!("active runner: {}", runner.kind.display_name()),
        None => "active runner: none configured".to_string(),
    }
}

fn openclaw_not_applicable(
    number: usize,
    runtime: &RuntimeState,
    id: &str,
    title: &str,
    reason: &str,
) -> DoctorCheck {
    DoctorCheck {
        number,
        id: id.to_string(),
        title: title.to_string(),
        status: DoctorStatus::Ok,
        message: format!("OpenClaw-specific check not applicable for this runtime ({reason})."),
        details: vec![runner_scope_description(runtime)],
        repairable: false,
        repaired: false,
    }
}

fn build_provider_auth_check(
    number: usize,
    compatibility: ManagedProviderAuthCompatibility,
    repair: bool,
    managed_home: &Path,
) -> DoctorCheck {
    let Some(provider_name) = compatibility.selected_provider.clone() else {
        return DoctorCheck {
            number,
            id: "openclaw_provider_guard".to_string(),
            title: "OpenClaw provider/auth compatibility".to_string(),
            status: DoctorStatus::Warn,
            message:
                "Unable to determine the selected OpenClaw provider; provider/auth compatibility could not be verified."
                    .to_string(),
            details: vec![
                format!("config path: {}", compatibility.config_path.display()),
                "expected `agents.defaults.model.primary` like `<provider>/<model>`.".to_string(),
            ],
            repairable: false,
            repaired: false,
        };
    };

    let mut details = vec![
        format!("provider: {provider_name}"),
        format!(
            "session-memory.enabled: {}",
            compatibility.session_memory_enabled
        ),
        format!("config path: {}", compatibility.config_path.display()),
        format!(
            "auth profiles path: {}",
            compatibility.auth_profiles_path.display()
        ),
        format!(
            "auth store path: {}",
            compatibility.auth_store_path.display()
        ),
    ];
    if !compatibility.profile_providers.is_empty() {
        details.push(format!(
            "profile providers: {}",
            compatibility.profile_providers.join(", ")
        ));
    }

    if compatibility.selected_provider_profile_present && !compatibility.legacy_profile_format {
        return DoctorCheck {
            number,
            id: "openclaw_provider_guard".to_string(),
            title: "OpenClaw provider/auth compatibility".to_string(),
            status: DoctorStatus::Ok,
            message: format!(
                "Managed provider/auth state aligns with selected provider `{provider_name}`."
            ),
            details,
            repairable: false,
            repaired: false,
        };
    }

    if compatibility
        .selected_provider_api_key
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
    {
        details.push(
            "repair skipped: selected provider credentials are not available in managed auth/model state."
                .to_string(),
        );
        return DoctorCheck {
            number,
            id: "openclaw_provider_guard".to_string(),
            title: "OpenClaw provider/auth compatibility".to_string(),
            status: DoctorStatus::Fail,
            message: format!(
                "Managed auth/profile state is incomplete for selected provider `{provider_name}`."
            ),
            details,
            repairable: false,
            repaired: false,
        };
    }

    if repair {
        return match enforce_managed_provider_auth_compatibility(managed_home) {
            Ok(true) => DoctorCheck {
                number,
                id: "openclaw_provider_guard".to_string(),
                title: "OpenClaw provider/auth compatibility".to_string(),
                status: DoctorStatus::Ok,
                message: format!(
                    "Repaired managed provider/auth state for selected provider `{provider_name}`."
                ),
                details,
                repairable: true,
                repaired: true,
            },
            Ok(false) => DoctorCheck {
                number,
                id: "openclaw_provider_guard".to_string(),
                title: "OpenClaw provider/auth compatibility".to_string(),
                status: DoctorStatus::Ok,
                message: "Managed provider/auth state was already aligned.".to_string(),
                details,
                repairable: true,
                repaired: false,
            },
            Err(err) => {
                details.push(err.to_string());
                DoctorCheck {
                    number,
                    id: "openclaw_provider_guard".to_string(),
                    title: "OpenClaw provider/auth compatibility".to_string(),
                    status: DoctorStatus::Fail,
                    message: "Failed to repair managed provider/auth compatibility.".to_string(),
                    details,
                    repairable: true,
                    repaired: false,
                }
            }
        };
    }

    if compatibility.legacy_profile_format {
        details.push(
            "legacy `auth-profiles.json` format detected; OpenClaw may ignore the selected provider in hook-driven lanes."
                .to_string(),
        );
    } else {
        details.push(
            "selected provider profile is missing from managed auth profiles; hook-driven background lanes can drift to a stale provider."
                .to_string(),
        );
    }
    details.push(
        "this can surface as false `No API key found for provider 'anthropic'` errors without changing the selected model provider."
            .to_string(),
    );

    DoctorCheck {
        number,
        id: "openclaw_provider_guard".to_string(),
        title: "OpenClaw provider/auth compatibility".to_string(),
        status: DoctorStatus::Warn,
        message: format!(
            "Managed auth/profile state does not align with selected provider `{provider_name}`."
        ),
        details,
        repairable: compatibility.repairable(),
        repaired: false,
    }
}

fn managed_openclaw_home(runtime: &RuntimeState) -> PathBuf {
    runtime
        .config
        .runner
        .as_ref()
        .map(|runner| runner.managed_home.clone())
        .unwrap_or_else(|| {
            runtime
                .config
                .runtime_root
                .join("user_data")
                .join("openclaw_home")
        })
}

#[derive(Debug, Clone)]
enum ConfigProbe {
    Ok(Duration),
    PathNotFound(Duration),
    Failed(String),
}

impl ConfigProbe {
    fn elapsed(&self) -> Option<Duration> {
        match self {
            Self::Ok(value) | Self::PathNotFound(value) => Some(*value),
            Self::Failed(_) => None,
        }
    }
}

fn timed_openclaw_config_probe(runtime: &RuntimeState, pointer: &str) -> ConfigProbe {
    let managed_home = managed_openclaw_home(runtime);
    let mut command = Command::new("openclaw");
    command.args(["config", "get", pointer, "--json"]);
    apply_runner_env_to_command(
        &mut command,
        RunnerKind::Openclaw,
        &managed_home,
        &runtime.config.ui_bind,
        runtime.config.approval_wait_timeout_secs,
    );
    let started = Instant::now();
    let output = match command_output_with_timeout(&mut command, OPENCLAW_CONFIG_COMMAND_TIMEOUT) {
        Ok(value) => value,
        Err(err) => {
            return ConfigProbe::Failed(format!("run `openclaw config get {pointer}`: {err}"));
        }
    };
    let elapsed = started.elapsed();
    if !output.status.success() {
        let detail = command_failure_detail(&output).trim().to_string();
        if detail
            .to_ascii_lowercase()
            .contains("config path not found")
        {
            return ConfigProbe::PathNotFound(elapsed);
        }
        return ConfigProbe::Failed(detail);
    }
    ConfigProbe::Ok(elapsed)
}

fn describe_config_probe(pointer: &str, probe: &ConfigProbe, missing_note: &str) -> String {
    match probe {
        ConfigProbe::Ok(latency) => format!(
            "latency `config get {pointer}`: {:.2}s",
            latency.as_secs_f64()
        ),
        ConfigProbe::PathNotFound(latency) => format!(
            "latency `config get {pointer}`: {:.2}s (path missing; {missing_note})",
            latency.as_secs_f64()
        ),
        ConfigProbe::Failed(detail) => {
            format!("`config get {pointer}` failed: {detail}")
        }
    }
}

fn first_probe_failure(routes_probe: &ConfigProbe, channels_probe: &ConfigProbe) -> Option<String> {
    if let ConfigProbe::Failed(detail) = routes_probe {
        return Some(format!(
            "route probe failure (`{OPENCLAW_BRIDGE_ROUTES_POINTER}`): {detail}"
        ));
    }
    if let ConfigProbe::Failed(detail) = channels_probe {
        return Some(format!("channel probe failure (`channels`): {detail}"));
    }
    None
}

fn managed_openclaw_bridge_routes_count(runtime: &RuntimeState) -> Result<Option<usize>> {
    let managed_home = managed_openclaw_home(runtime);
    let mut get_routes_cmd = Command::new("openclaw");
    get_routes_cmd.args(["config", "get", OPENCLAW_BRIDGE_ROUTES_POINTER, "--json"]);
    apply_runner_env_to_command(
        &mut get_routes_cmd,
        RunnerKind::Openclaw,
        &managed_home,
        &runtime.config.ui_bind,
        runtime.config.approval_wait_timeout_secs,
    );
    let output = command_output_with_timeout(&mut get_routes_cmd, OPENCLAW_CONFIG_COMMAND_TIMEOUT)
        .with_context(|| {
            format!(
                "run `openclaw config get {}` with OPENCLAW_HOME={}",
                OPENCLAW_BRIDGE_ROUTES_POINTER,
                managed_home.display()
            )
        })?;

    if !output.status.success() {
        let detail = command_failure_detail(&output).to_ascii_lowercase();
        if detail.contains("config path not found") {
            return Ok(None);
        }
        return Err(anyhow!(command_failure_detail(&output)));
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() || raw.eq_ignore_ascii_case("null") {
        return Ok(None);
    }

    let parsed: serde_json::Value =
        serde_json::from_str(&raw).context("parse OpenClaw bridge routes JSON")?;
    let Some(routes) = parsed.as_array() else {
        return Ok(None);
    };
    if routes.is_empty() {
        return Ok(None);
    }
    Ok(Some(routes.len()))
}

fn non_empty_trimmed(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn tail_lines(raw: &str, max_lines: usize) -> Vec<&str> {
    let mut lines: Vec<&str> = raw.lines().collect();
    if lines.len() > max_lines {
        lines.drain(0..(lines.len() - max_lines));
    }
    lines
}

fn inspect_openclaw_bridge_route_runtime(
    runtime: &RuntimeState,
) -> Option<BridgeRouteRuntimeObservation> {
    let log_path = runtime
        .config
        .runtime_root
        .join("user_data")
        .join("logs")
        .join(OPENCLAW_CHANNEL_BRIDGE_LOG_FILE_NAME);
    let raw = fs::read_to_string(&log_path).ok()?;
    let mut observation = BridgeRouteRuntimeObservation {
        log_path,
        ..BridgeRouteRuntimeObservation::default()
    };

    for line in tail_lines(&raw, 240) {
        let trimmed = line.trim();
        if trimmed.contains("config loaded: routes_source=") {
            observation.config_source = trimmed
                .split("routes_source=")
                .nth(1)
                .and_then(|value| value.split_whitespace().next())
                .map(ToOwned::to_owned);
        }
        if trimmed.contains("routes refreshed: source=") {
            observation.refreshed_source = trimmed
                .split("source=")
                .nth(1)
                .and_then(|value| value.split_whitespace().next())
                .map(ToOwned::to_owned);
            observation.refreshed_routes = trimmed
                .split("routes=")
                .nth(1)
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse::<usize>().ok());
        }
        if trimmed.contains("listening on http://") {
            observation.inbound_ready = true;
        }
    }

    Some(observation)
}

fn openclaw_logs_for_diagnostics(runtime: &RuntimeState) -> Vec<PathBuf> {
    let logs_dir = runtime.config.runtime_root.join("user_data").join("logs");
    [
        logs_dir.join(OPENCLAW_CHANNEL_BRIDGE_LOG_FILE_NAME),
        logs_dir.join(OPENCLAW_GATEWAY_LOG_FILE_NAME),
    ]
    .into_iter()
    .collect()
}

fn discover_openclaw_channel_default_routes(
    runtime: &RuntimeState,
) -> Result<Vec<serde_json::Value>> {
    let managed_home = managed_openclaw_home(runtime);
    let channels = read_openclaw_channels_config(runtime)?;
    let allow_from = read_openclaw_allow_from_entries(&managed_home)?;
    Ok(build_channel_default_route_docs(&channels, &allow_from))
}

fn read_openclaw_channels_config(runtime: &RuntimeState) -> Result<Map<String, Value>> {
    let managed_home = managed_openclaw_home(runtime);
    let mut command = Command::new("openclaw");
    command.args(["config", "get", "channels", "--json"]);
    apply_runner_env_to_command(
        &mut command,
        RunnerKind::Openclaw,
        &managed_home,
        &runtime.config.ui_bind,
        runtime.config.approval_wait_timeout_secs,
    );
    let output = command_output_with_timeout(&mut command, OPENCLAW_CONFIG_COMMAND_TIMEOUT)
        .with_context(|| {
            format!(
                "run `openclaw config get channels` with OPENCLAW_HOME={}",
                managed_home.display()
            )
        })?;
    if !output.status.success() {
        let detail = command_failure_detail(&output).to_ascii_lowercase();
        if detail.contains("config path not found") {
            return Ok(Map::new());
        }
        return Err(anyhow!(command_failure_detail(&output)));
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() || raw.eq_ignore_ascii_case("null") {
        return Ok(Map::new());
    }
    let parsed: Value =
        serde_json::from_str(&raw).context("parse OpenClaw channels config JSON")?;
    Ok(parsed.as_object().cloned().unwrap_or_default())
}

fn read_openclaw_allow_from_entries(
    managed_home: &Path,
) -> Result<BTreeMap<String, BTreeMap<String, Vec<String>>>> {
    let credentials_dir = managed_home.join(".openclaw").join("credentials");
    if !credentials_dir.is_dir() {
        return Ok(BTreeMap::new());
    }

    let mut collected = BTreeMap::new();
    for entry in fs::read_dir(&credentials_dir)
        .with_context(|| format!("read {}", credentials_dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("read entry under {}", credentials_dir.display()))?;
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(base) = file_name.strip_suffix("-allowFrom.json") else {
            continue;
        };
        let (channel, account) = match base.split_once('-') {
            Some((channel, account)) => (channel.trim().to_ascii_lowercase(), account.trim()),
            None => (base.trim().to_ascii_lowercase(), "default"),
        };
        if !matches!(channel.as_str(), "telegram" | "whatsapp" | "discord") {
            continue;
        }

        let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let parsed: Value =
            serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
        let Some(values) = parsed.get("allowFrom").and_then(Value::as_array) else {
            continue;
        };
        let entries = values
            .iter()
            .filter_map(|value| match value {
                Value::String(raw) => non_empty_trimmed(raw),
                Value::Number(raw) => Some(raw.to_string()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        if entries.is_empty() {
            continue;
        }

        let channel_map = collected.entry(channel).or_insert_with(BTreeMap::new);
        channel_map.insert(account.to_string(), entries.into_iter().collect());
    }

    Ok(collected)
}

fn build_channel_default_route_docs(
    channels: &Map<String, Value>,
    allow_from: &BTreeMap<String, BTreeMap<String, Vec<String>>>,
) -> Vec<serde_json::Value> {
    let mut routes = Vec::new();
    let mut seen = BTreeSet::new();

    for channel in ["discord", "telegram", "whatsapp"] {
        let Some(config) = channels.get(channel).and_then(Value::as_object) else {
            continue;
        };
        if !config
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }

        let mut accounts = allow_from
            .get(channel)
            .cloned()
            .unwrap_or_else(BTreeMap::new);
        if let Some(values) = config.get("allowFrom").and_then(Value::as_array) {
            let merged = values
                .iter()
                .filter_map(|value| match value {
                    Value::String(raw) => non_empty_trimmed(raw),
                    Value::Number(raw) => Some(raw.to_string()),
                    _ => None,
                })
                .collect::<BTreeSet<_>>();
            if !merged.is_empty() {
                let bucket = accounts
                    .entry("default".to_string())
                    .or_insert_with(Vec::new);
                bucket.extend(merged);
                bucket.sort();
                bucket.dedup();
            }
        }

        for (account, senders) in accounts {
            for sender in senders {
                let dedupe_key = format!("{channel}\u{0}{account}\u{0}{sender}");
                if !seen.insert(dedupe_key) {
                    continue;
                }
                let mut route = Map::new();
                route.insert("channel".to_string(), Value::String(channel.to_string()));
                route.insert("target".to_string(), Value::String(sender.clone()));
                route.insert(
                    "allow_from".to_string(),
                    Value::Array(vec![Value::String(sender)]),
                );
                if !account.is_empty() {
                    route.insert("account".to_string(), Value::String(account.clone()));
                }
                if channel == "telegram" {
                    route.insert("telegram_inline_buttons".to_string(), Value::Bool(true));
                    route.insert(
                        "telegram_streaming_enabled".to_string(),
                        Value::Bool(
                            config
                                .get("streaming")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        ),
                    );
                }
                if channel == "whatsapp" {
                    route.insert("whatsapp_use_poll".to_string(), Value::Bool(true));
                }
                routes.push(Value::Object(route));
            }
        }
    }

    routes
}

fn write_managed_openclaw_bridge_routes(
    runtime: &RuntimeState,
    routes: &[serde_json::Value],
) -> Result<()> {
    if routes.is_empty() {
        return Ok(());
    }
    let managed_home = managed_openclaw_home(runtime);
    let serialized = serde_json::to_string(routes).context("serialize bridge routes JSON")?;

    let mut set_routes_cmd = Command::new("openclaw");
    set_routes_cmd.args([
        "config",
        "set",
        OPENCLAW_BRIDGE_ROUTES_POINTER,
        &serialized,
        "--json",
    ]);
    apply_runner_env_to_command(
        &mut set_routes_cmd,
        RunnerKind::Openclaw,
        &managed_home,
        &runtime.config.ui_bind,
        runtime.config.approval_wait_timeout_secs,
    );
    let output = command_output_with_timeout(&mut set_routes_cmd, OPENCLAW_CONFIG_COMMAND_TIMEOUT)
        .with_context(|| {
            format!(
                "run `openclaw config set {}` with OPENCLAW_HOME={}",
                OPENCLAW_BRIDGE_ROUTES_POINTER,
                managed_home.display()
            )
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(anyhow!(command_failure_detail(&output)))
}

fn find_legacy_bridge_routes(
    runtime: &RuntimeState,
) -> Result<Option<(PathBuf, Vec<serde_json::Value>)>> {
    for candidate in [
        runtime
            .config
            .ruler_root
            .join("bridge")
            .join("openclaw")
            .join(OPENCLAW_CHANNEL_BRIDGE_CONFIG_FILE_NAME),
        runtime
            .config
            .ruler_root
            .join("bridge")
            .join("openclaw")
            .join(OPENCLAW_CHANNEL_BRIDGE_LOCAL_CONFIG_FILE_NAME),
        runtime
            .config
            .ruler_root
            .join("bridge")
            .join(OPENCLAW_CHANNEL_BRIDGE_LEGACY_CONFIG_FILE_NAME),
        runtime
            .config
            .ruler_root
            .join("bridge")
            .join(OPENCLAW_CHANNEL_BRIDGE_LEGACY_LOCAL_CONFIG_FILE_NAME),
    ] {
        if !candidate.exists() {
            continue;
        }
        if let Some(routes) = bridge_config_routes(&candidate)? {
            return Ok(Some((candidate, routes)));
        }
    }
    Ok(None)
}

fn bridge_config_routes(config_path: &Path) -> Result<Option<Vec<serde_json::Value>>> {
    let raw = fs::read_to_string(config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", config_path.display()))?;
    let routes = parsed
        .get("routes")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    if routes.is_empty() {
        return Ok(None);
    }
    Ok(Some(routes))
}

fn network_policy_allows_host(rules: &NetworkRules, host: &str) -> bool {
    let in_allowlist = rules
        .allowlist_hosts
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(host));
    let in_denylist = rules
        .denylist_hosts
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(host));

    let allowlist_pass = if rules.allowlist_hosts.is_empty() {
        true
    } else if rules.invert_allowlist {
        !in_allowlist
    } else {
        in_allowlist
    };

    let denylist_pass = if rules.denylist_hosts.is_empty() {
        true
    } else if rules.invert_denylist {
        in_denylist
    } else {
        !in_denylist
    };

    if !allowlist_pass || !denylist_pass {
        return false;
    }

    if !rules.default_deny {
        return true;
    }

    (!rules.allowlist_hosts.is_empty() && !rules.invert_allowlist && in_allowlist)
        || (!rules.denylist_hosts.is_empty() && rules.invert_denylist && in_denylist)
}

fn command_failure_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }
    format!("exit status {}", output.status)
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use super::{
        bubblewrap_manual_remediation_details, build_channel_default_route_docs,
        network_policy_allows_host, sanitize_systemd_run_probe_detail, BubblewrapRepairCapability,
    };
    #[cfg(not(target_os = "linux"))]
    use super::{build_channel_default_route_docs, network_policy_allows_host};
    use crate::config::NetworkRules;
    use serde_json::json;

    #[test]
    fn telegram_host_requires_explicit_allowlist_under_default_deny() {
        let rules = NetworkRules {
            default_deny: true,
            allowlist_hosts: vec!["github.com".to_string()],
            require_approval_for_post: true,
            denylist_hosts: Vec::new(),
            invert_allowlist: false,
            invert_denylist: false,
        };
        assert!(!network_policy_allows_host(&rules, "api.telegram.org"));
    }

    #[test]
    fn channel_default_routes_include_config_and_credential_allow_from_entries() {
        let channels = json!({
            "telegram": {
                "enabled": true,
                "allowFrom": ["123456789"],
                "streaming": true
            }
        });
        let allow_from = std::collections::BTreeMap::from([(
            "telegram".to_string(),
            std::collections::BTreeMap::from([(
                "default".to_string(),
                vec!["123456789".to_string(), "987654321".to_string()],
            )]),
        )]);

        let routes = build_channel_default_route_docs(
            channels.as_object().expect("channels object"),
            &allow_from,
        );
        assert_eq!(routes.len(), 2);
        assert!(
            routes.iter().all(|route| {
                route
                    .get("telegram_streaming_enabled")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
            }),
            "telegram autodiscovery should preserve streaming defaults"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sanitize_systemd_run_probe_detail_strips_service_noise() {
        let raw = "Running as unit: run-u451.service; invocation ID: abc\nbwrap: setting up uid map: Permission denied\nFinished with result: exit-code\nMain processes terminated with: code=exited/status=1\nService runtime: 6ms\nCPU time consumed: 5ms\nMemory peak: 292.0K\nMemory swap peak: 0B\n";
        assert_eq!(
            sanitize_systemd_run_probe_detail(raw),
            "bwrap: setting up uid map: Permission denied"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bubblewrap_manual_remediation_uses_profile_based_guidance() {
        let details = bubblewrap_manual_remediation_details(&BubblewrapRepairCapability {
            auto_repairable: false,
            detail: Some("this session does not have non-interactive root access.".to_string()),
        });
        assert!(details.iter().any(|line| line.contains("fix: install")));
        assert!(details
            .iter()
            .any(|line| line.contains("/etc/apparmor.d/bwrap")));
        assert!(details
            .iter()
            .any(|line| line.contains("privileges: requires operator-auth root")));
        assert!(details
            .iter()
            .any(|line| line.contains("automatic repair: unavailable")));
        assert!(!details.iter().any(|line| line.contains("aa-complain")));
    }
}
