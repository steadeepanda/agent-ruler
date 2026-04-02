#![cfg(target_os = "linux")]

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use agent_ruler::config::{init_layout, load_runtime, save_config, save_policy, CONFIG_FILE_NAME};
use agent_ruler::runners::{RunnerAssociation, RunnerKind, RunnerMissingState};

use common::TestRuntimeDir;

const ROUTES_POINTER: &str =
    "plugins.entries.openclaw-agent-ruler-tools.config.approvalBridgeRoutes";

fn bin_path() -> PathBuf {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_agent-ruler") {
        return PathBuf::from(path);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("agent-ruler")
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }
}

fn setup_openclaw_runtime(
    project: &TestRuntimeDir,
    runtime_root: &TestRuntimeDir,
) -> (agent_ruler::config::RuntimeState, PathBuf, PathBuf) {
    init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
    let mut runtime =
        load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");

    let managed_home = runtime
        .config
        .runtime_root
        .join("user_data")
        .join("openclaw_home");
    let managed_workspace = runtime
        .config
        .runtime_root
        .join("user_data")
        .join("openclaw_workspace");
    fs::create_dir_all(&managed_home).expect("create managed home");
    fs::create_dir_all(&managed_workspace).expect("create managed workspace");

    runtime.config.runner = Some(RunnerAssociation {
        kind: RunnerKind::Openclaw,
        managed_home: managed_home.clone(),
        managed_workspace,
        integrations: Vec::new(),
        missing: RunnerMissingState::default(),
    });

    runtime.policy.rules.network.default_deny = true;
    runtime.policy.rules.network.allowlist_hosts = vec!["github.com".to_string()];
    runtime.policy.rules.network.denylist_hosts.clear();
    runtime.policy.rules.network.invert_allowlist = false;
    runtime.policy.rules.network.invert_denylist = false;

    save_config(
        &runtime.config.state_dir.join(CONFIG_FILE_NAME),
        &runtime.config,
    )
    .expect("save config");
    save_policy(&runtime.config.policy_file, &runtime.policy).expect("save policy");

    let managed_cfg_dir = managed_home.join(".openclaw");
    fs::create_dir_all(&managed_cfg_dir).expect("create openclaw state dir");
    fs::write(
        managed_cfg_dir.join("openclaw.json"),
        r#"{
  "auth": {
    "profiles": {
      "zai:default": {
        "provider": "zai",
        "mode": "api_key"
      }
    }
  },
  "agents": {
    "defaults": {
      "model": {
        "primary": "zai/glm-4.7"
      }
    }
  },
  "hooks": {
    "internal": {
      "entries": {
        "session-memory": {
          "enabled": true
        }
      }
    }
  },
  "channels": {
    "telegram": {
      "enabled": true,
      "botToken": "test-token"
    }
  }
}"#,
    )
    .expect("write managed openclaw config");
    let managed_agent_dir = managed_cfg_dir.join("agents").join("main").join("agent");
    fs::create_dir_all(&managed_agent_dir).expect("create managed agent dir");
    fs::write(
        managed_agent_dir.join("auth-profiles.json"),
        r#"{ "default": "anthropic" }"#,
    )
    .expect("write legacy auth profiles");
    fs::write(
        managed_agent_dir.join("auth.json"),
        r#"{
  "zai": {
    "type": "api_key",
    "key": "test-zai-key"
  }
}"#,
    )
    .expect("write managed auth store");
    fs::write(
        managed_agent_dir.join("models.json"),
        r#"{
  "providers": {
    "zai": {
      "apiKey": "test-zai-key",
      "models": [{ "id": "glm-4.7" }]
    }
  }
}"#,
    )
    .expect("write managed models");

    let bridge_dir = runtime.config.runtime_root.join("user_data").join("bridge");
    fs::create_dir_all(&bridge_dir).expect("create bridge dir");
    fs::write(
        bridge_dir.join("openclaw-channel-bridge.generated.json"),
        r#"{"inbound_bind":"127.0.0.1:4661","routes":[]}"#,
    )
    .expect("write generated bridge config");

    let bin_dir = runtime.config.runtime_root.join("test-bin");
    fs::create_dir_all(&bin_dir).expect("create test bin dir");
    let route_store = runtime.config.runtime_root.join("route-store.json");
    let shim = format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "${{1:-}}" == "config" && "${{2:-}}" == "get" ]]; then
  pointer="${{3:-}}"
  if [[ "$pointer" == "{routes_pointer}" ]]; then
    if [[ "${{AR_ROUTE_POINTER_MISSING:-0}}" == "1" ]]; then
      echo "Config path not found: $pointer" >&2
      exit 1
    fi
    if [[ -f "$AR_ROUTE_STORE" ]]; then
      cat "$AR_ROUTE_STORE"
    else
      echo '[]'
    fi
    exit 0
  fi
  if [[ "$pointer" == "channels" ]]; then
    echo '{{"telegram":{{"enabled":true}}}}'
    exit 0
  fi
  echo 'null'
  exit 0
fi
if [[ "${{1:-}}" == "config" && "${{2:-}}" == "set" ]]; then
  pointer="${{3:-}}"
  value="${{4:-null}}"
  if [[ "$pointer" == "{routes_pointer}" ]]; then
    printf '%s' "$value" > "$AR_ROUTE_STORE"
  fi
  echo '{{"ok":true}}'
  exit 0
fi
echo '{{"ok":true}}'
"#,
        routes_pointer = ROUTES_POINTER
    );
    write_executable(&bin_dir.join("openclaw"), &shim);

    (runtime, bin_dir, route_store)
}

#[test]
fn doctor_json_output_reports_checks() {
    let project = TestRuntimeDir::new("doctor-json-project");
    let runtime_root = TestRuntimeDir::new("doctor-json-runtime");
    let (_runtime, bin_dir, route_store) = setup_openclaw_runtime(&project, &runtime_root);

    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", bin_dir.display(), path_env);
    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["doctor", "--json"])
        .env("PATH", merged_path)
        .env("AR_ROUTE_STORE", &route_store)
        .output()
        .expect("run doctor");

    assert!(
        output.status.success(),
        "doctor command should succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse doctor json output");
    assert!(payload.get("status").is_some(), "doctor status required");
    assert!(
        payload["checks"]
            .as_array()
            .map(|v| !v.is_empty())
            .unwrap_or(false),
        "doctor checks should not be empty"
    );
    assert_eq!(payload["repair_selection"], serde_json::Value::Null);
}

#[test]
fn doctor_repair_adds_telegram_allowlist_baseline() {
    let project = TestRuntimeDir::new("doctor-repair-project");
    let runtime_root = TestRuntimeDir::new("doctor-repair-runtime");
    let (runtime, bin_dir, route_store) = setup_openclaw_runtime(&project, &runtime_root);

    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", bin_dir.display(), path_env);
    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["doctor", "--repair", "7", "--json"])
        .env("PATH", merged_path)
        .env("AR_ROUTE_STORE", &route_store)
        .output()
        .expect("run doctor repair");

    assert!(
        output.status.success(),
        "doctor repair should succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse doctor repair json output");
    let checks = payload["checks"]
        .as_array()
        .expect("doctor repair checks should be an array");
    let telegram_check = checks
        .iter()
        .find(|check| check["id"] == "telegram_allowlist_baseline")
        .expect("telegram allowlist check should be present");
    assert_eq!(telegram_check["status"], "ok");
    assert_eq!(telegram_check["repaired"], true);
    assert_eq!(telegram_check["number"], 7);
    assert_eq!(payload["repair_selection"], "7");

    let reloaded = load_runtime(project.path(), Some(runtime_root.path())).expect("reload runtime");
    assert!(
        reloaded
            .policy
            .rules
            .network
            .allowlist_hosts
            .iter()
            .any(|host| host.eq_ignore_ascii_case("api.telegram.org")),
        "doctor --repair should persist telegram allowlist baseline"
    );
    assert_eq!(
        reloaded.config.policy_file, runtime.config.policy_file,
        "repair should update the same runtime policy file"
    );
}

fn setup_non_openclaw_runtime(
    project: &TestRuntimeDir,
    runtime_root: &TestRuntimeDir,
) -> agent_ruler::config::RuntimeState {
    init_layout(project.path(), Some(runtime_root.path()), None, true).expect("init runtime");
    let mut runtime =
        load_runtime(project.path(), Some(runtime_root.path())).expect("load runtime");

    let managed_home = runtime
        .config
        .runtime_root
        .join("user_data")
        .join("claudecode_home");
    let managed_workspace = runtime.config.runtime_root.join("workspace");
    fs::create_dir_all(&managed_home).expect("create managed home");
    fs::create_dir_all(&managed_workspace).expect("create managed workspace");

    runtime.config.runner = Some(RunnerAssociation {
        kind: RunnerKind::Claudecode,
        managed_home,
        managed_workspace,
        integrations: Vec::new(),
        missing: RunnerMissingState::default(),
    });

    save_config(
        &runtime.config.state_dir.join(CONFIG_FILE_NAME),
        &runtime.config,
    )
    .expect("save runtime config");
    runtime
}

#[test]
fn doctor_non_openclaw_runner_marks_openclaw_checks_not_applicable() {
    let project = TestRuntimeDir::new("doctor-non-openclaw-project");
    let runtime_root = TestRuntimeDir::new("doctor-non-openclaw-runtime");
    setup_non_openclaw_runtime(&project, &runtime_root);

    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["doctor", "--json"])
        .output()
        .expect("run doctor for non-openclaw runtime");
    assert!(
        output.status.success(),
        "doctor should succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse doctor json output");
    let checks = payload["checks"]
        .as_array()
        .expect("doctor checks should be an array");
    for check_id in [
        "openclaw_bridge_config_artifact",
        "openclaw_route_seed",
        "openclaw_config_discovery_latency",
        "openclaw_provider_guard",
        "telegram_allowlist_baseline",
        "openclaw_telegram_command_sync",
    ] {
        let check = checks
            .iter()
            .find(|item| item["id"] == check_id)
            .unwrap_or_else(|| panic!("expected check {check_id}"));
        assert_eq!(
            check["status"], "ok",
            "{check_id} should be ok (not applicable) instead of warn/fail"
        );
        assert!(
            check["message"]
                .as_str()
                .unwrap_or_default()
                .contains("not applicable"),
            "{check_id} should explain non-applicable scope"
        );
    }
}

#[test]
fn doctor_config_discovery_tolerates_missing_route_pointer() {
    let project = TestRuntimeDir::new("doctor-route-pointer-missing-project");
    let runtime_root = TestRuntimeDir::new("doctor-route-pointer-missing-runtime");
    let (_runtime, bin_dir, route_store) = setup_openclaw_runtime(&project, &runtime_root);

    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", bin_dir.display(), path_env);
    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["doctor", "--json"])
        .env("PATH", merged_path)
        .env("AR_ROUTE_STORE", &route_store)
        .env("AR_ROUTE_POINTER_MISSING", "1")
        .output()
        .expect("run doctor with route pointer missing");

    assert!(
        output.status.success(),
        "doctor should succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse doctor json output");
    let checks = payload["checks"]
        .as_array()
        .expect("doctor checks should be an array");
    let latency = checks
        .iter()
        .find(|check| check["id"] == "openclaw_config_discovery_latency")
        .expect("latency check should exist");
    assert_ne!(
        latency["status"], "fail",
        "missing route pointer should not be treated as discovery failure"
    );
}

#[test]
fn doctor_route_seed_reports_running_unconfigured_bridge_mode() {
    let project = TestRuntimeDir::new("doctor-route-seed-runtime-log-project");
    let runtime_root = TestRuntimeDir::new("doctor-route-seed-runtime-log-runtime");
    let (_runtime, bin_dir, route_store) = setup_openclaw_runtime(&project, &runtime_root);
    let logs_dir = runtime_root.path().join("user_data").join("logs");
    fs::create_dir_all(&logs_dir).expect("create logs dir");
    fs::write(
        logs_dir.join("openclaw-channel-bridge.log"),
        "[bridge] config loaded: routes_source=openclaw_startup_deferred routes=0\n[bridge] listening on http://127.0.0.1:4661/inbound\n[bridge] routes refreshed: source=openclaw_unconfigured routes=0\n",
    )
    .expect("write bridge log");

    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", bin_dir.display(), path_env);
    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["doctor", "--json"])
        .env("PATH", merged_path)
        .env("AR_ROUTE_STORE", &route_store)
        .env("AR_ROUTE_POINTER_MISSING", "1")
        .output()
        .expect("run doctor with unconfigured bridge log");

    assert!(
        output.status.success(),
        "doctor should succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse doctor json output");
    let checks = payload["checks"]
        .as_array()
        .expect("doctor checks should be an array");
    let route_seed = checks
        .iter()
        .find(|check| check["id"] == "openclaw_route_seed")
        .expect("route seed check should exist");
    assert_eq!(route_seed["status"], "warn");
    assert!(
        route_seed["message"]
            .as_str()
            .unwrap_or_default()
            .contains("running in unconfigured autodiscovery mode"),
        "route seed message should reflect the active bridge runtime state"
    );
}

#[test]
fn doctor_provider_guard_detects_and_repairs_non_anthropic_session_memory() {
    let project = TestRuntimeDir::new("doctor-provider-guard-project");
    let runtime_root = TestRuntimeDir::new("doctor-provider-guard-runtime");
    let (runtime, bin_dir, route_store) = setup_openclaw_runtime(&project, &runtime_root);
    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", bin_dir.display(), path_env);

    let warn_output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["doctor", "--json"])
        .env("PATH", &merged_path)
        .env("AR_ROUTE_STORE", &route_store)
        .output()
        .expect("run doctor before repair");
    assert!(
        warn_output.status.success(),
        "doctor should succeed: stderr={}",
        String::from_utf8_lossy(&warn_output.stderr)
    );
    let warn_payload: serde_json::Value =
        serde_json::from_slice(&warn_output.stdout).expect("parse doctor json output");
    let warn_checks = warn_payload["checks"]
        .as_array()
        .expect("doctor checks should be an array");
    let provider_warn = warn_checks
        .iter()
        .find(|check| check["id"] == "openclaw_provider_guard")
        .expect("provider guard check should be present");
    assert_eq!(provider_warn["status"], "warn");
    assert_eq!(provider_warn["repairable"], true);
    assert_eq!(provider_warn["number"], 6);

    let repair_output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["doctor", "--repair", "6", "--json"])
        .env("PATH", merged_path)
        .env("AR_ROUTE_STORE", &route_store)
        .output()
        .expect("run doctor repair");
    assert!(
        repair_output.status.success(),
        "doctor repair should succeed: stderr={}",
        String::from_utf8_lossy(&repair_output.stderr)
    );
    let repair_payload: serde_json::Value =
        serde_json::from_slice(&repair_output.stdout).expect("parse doctor repair json output");
    let repair_checks = repair_payload["checks"]
        .as_array()
        .expect("doctor checks should be an array");
    let provider_repair = repair_checks
        .iter()
        .find(|check| check["id"] == "openclaw_provider_guard")
        .expect("provider guard check should be present");
    assert_eq!(provider_repair["status"], "ok");
    assert_eq!(provider_repair["repaired"], true);
    assert_eq!(repair_payload["repair_selection"], "6");

    let managed_config = runtime
        .config
        .runtime_root
        .join("user_data")
        .join("openclaw_home")
        .join(".openclaw")
        .join("openclaw.json");
    let raw = fs::read_to_string(&managed_config).expect("read managed config");
    let parsed: serde_json::Value = json5::from_str(&raw).expect("parse managed config");
    assert_eq!(
        parsed.pointer("/hooks/internal/entries/session-memory/enabled"),
        Some(&serde_json::Value::Bool(true)),
        "doctor repair should preserve session-memory hook state"
    );
    let auth_profiles = runtime
        .config
        .runtime_root
        .join("user_data")
        .join("openclaw_home")
        .join(".openclaw")
        .join("agents/main/agent/auth-profiles.json");
    let profiles_raw = fs::read_to_string(auth_profiles).expect("read repaired auth profiles");
    let profiles: serde_json::Value =
        serde_json::from_str(&profiles_raw).expect("parse repaired auth profiles");
    assert_eq!(
        profiles.pointer("/profiles/zai:default/provider"),
        Some(&serde_json::Value::String("zai".to_string())),
        "doctor repair should align auth profiles with the selected provider"
    );
}

#[test]
fn doctor_telegram_command_sync_uses_bridge_log_when_gateway_log_is_missing() {
    let project = TestRuntimeDir::new("doctor-telegram-bridge-log-project");
    let runtime_root = TestRuntimeDir::new("doctor-telegram-bridge-log-runtime");
    let (_runtime, bin_dir, route_store) = setup_openclaw_runtime(&project, &runtime_root);
    let logs_dir = runtime_root.path().join("user_data").join("logs");
    fs::create_dir_all(&logs_dir).expect("create logs dir");
    fs::write(
        logs_dir.join("openclaw-channel-bridge.log"),
        "[bridge] Telegram setMyCommands network request failed: ETIMEDOUT\n",
    )
    .expect("write bridge log");

    let path_env = std::env::var("PATH").unwrap_or_default();
    let merged_path = format!("{}:{}", bin_dir.display(), path_env);
    let output = Command::new(bin_path())
        .current_dir(project.path())
        .arg("--runtime-dir")
        .arg(runtime_root.path())
        .args(["doctor", "--json"])
        .env("PATH", merged_path)
        .env("AR_ROUTE_STORE", &route_store)
        .output()
        .expect("run doctor with bridge log signal");

    assert!(
        output.status.success(),
        "doctor should succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse doctor json output");
    let checks = payload["checks"]
        .as_array()
        .expect("doctor checks should be an array");
    let sync = checks
        .iter()
        .find(|check| check["id"] == "openclaw_telegram_command_sync")
        .expect("telegram command sync check should exist");
    assert_eq!(sync["status"], "ok");
    assert!(
        sync["details"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .any(|line| line.contains("openclaw-channel-bridge.log")),
        "command sync health should report the bridge log path when that is the signal source"
    );
}
