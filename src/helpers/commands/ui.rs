//! UI control helpers extracted from `src/agent_ruler.rs`.
//! Keep CLI orchestration focused on intent while helpers encapsulate file-side effects.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::config::RuntimeState;

const UI_PID_FILE_NAME: &str = "agent-ruler-ui.pid";

/// Path used to persist the UI process pid for later stop commands.
pub fn ui_pid_path(runtime: &RuntimeState) -> PathBuf {
    runtime
        .config
        .runtime_root
        .join("user_data")
        .join("logs")
        .join(UI_PID_FILE_NAME)
}

/// Guard that keeps track of the UI pid file while the server runs.
pub struct UiPidGuard {
    path: PathBuf,
}

impl UiPidGuard {
    /// Create or update the pid file with the current process id.
    pub fn create(runtime: &RuntimeState) -> Result<Self> {
        let path = ui_pid_path(runtime);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::write(&path, format!("{}\n", std::process::id()))
            .with_context(|| format!("write {}", path.display()))?;
        Ok(Self { path })
    }
}

impl Drop for UiPidGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Stop the UI process recorded in the pid file, if present.
pub fn stop_ui_process(runtime: &RuntimeState) -> Result<bool> {
    let pid_path = ui_pid_path(runtime);
    stop_ui_process_from_pid_file(&pid_path)
}

/// Stop UI processes for every runtime under the same projects root as the active runtime.
///
/// This prevents stale UI daemons (for older installed versions/runtimes) from continuing
/// to serve outdated assets on the same bind after upgrades.
pub fn stop_ui_processes_in_projects_root(runtime: &RuntimeState) -> Result<bool> {
    let mut all_stopped = true;
    let current_pid_path = ui_pid_path(runtime);
    if !stop_ui_process_from_pid_file(&current_pid_path)? {
        all_stopped = false;
    }

    let Some(projects_root) = runtime.config.runtime_root.parent() else {
        return Ok(all_stopped);
    };
    if !projects_root.exists() {
        return Ok(all_stopped);
    }

    for entry in fs::read_dir(projects_root)
        .with_context(|| format!("read projects root {}", projects_root.display()))?
    {
        let entry = match entry {
            Ok(value) => value,
            Err(err) => {
                eprintln!(
                    "ui stop: unable to inspect runtime entry under {}: {err}",
                    projects_root.display()
                );
                all_stopped = false;
                continue;
            }
        };
        let runtime_root = entry.path();
        if !runtime_root.is_dir() || runtime_root == runtime.config.runtime_root {
            continue;
        }
        let pid_path = runtime_root
            .join("user_data")
            .join("logs")
            .join(UI_PID_FILE_NAME);
        if !pid_path.exists() {
            continue;
        }

        if !stop_ui_process_from_pid_file(&pid_path)? {
            all_stopped = false;
        }
    }

    // Fallback for stale/orphaned UI daemons where pid files are missing
    // (for example older versions, manual cleanup, or interrupted shutdowns).
    let mut seen = BTreeSet::new();
    for pid in discover_orphaned_ui_pids()? {
        if !seen.insert(pid) {
            continue;
        }
        if !terminate_ui_pid(pid, "process scan fallback")? {
            all_stopped = false;
        }
    }

    Ok(all_stopped)
}

fn stop_ui_process_from_pid_file(pid_path: &Path) -> Result<bool> {
    if !pid_path.exists() {
        println!(
            "ui stop: no persisted UI pid record found ({}); nothing to stop.",
            pid_path.display()
        );
        return Ok(true);
    }

    let raw =
        fs::read_to_string(&pid_path).with_context(|| format!("read {}", pid_path.display()))?;
    let pid = raw
        .trim()
        .parse::<u32>()
        .with_context(|| format!("parse pid from {}", pid_path.display()))?;

    if !is_agent_ruler_ui_process(pid) {
        let _ = fs::remove_file(pid_path);
        println!(
            "ui stop: pid {} from {} is not an Agent Ruler UI process; cleared stale pid record.",
            pid,
            pid_path.display()
        );
        return Ok(true);
    }

    let stopped = terminate_ui_pid(pid, &pid_path.display().to_string())?;
    if stopped {
        let _ = fs::remove_file(pid_path);
    }
    Ok(stopped)
}

fn terminate_ui_pid(pid: u32, origin: &str) -> Result<bool> {
    if !process_exists(pid) {
        println!(
            "ui stop: recorded pid {} from {} is not running; nothing to stop.",
            pid, origin
        );
        return Ok(true);
    }

    println!(
        "ui stop: stopping Agent Ruler UI process (pid: {}, source: {}).",
        pid, origin
    );

    if let Err(err) = send_signal(pid, "-TERM") {
        eprintln!("ui stop: unable to signal TERM to pid {}: {err}", pid);
    }
    if wait_for_process_exit(pid, Duration::from_millis(100), 40) {
        println!("ui stop: Agent Ruler UI stopped (pid: {}).", pid);
        return Ok(true);
    }

    if let Err(err) = send_signal(pid, "-KILL") {
        eprintln!("ui stop: unable to signal KILL to pid {}: {err}", pid);
    }
    if wait_for_process_exit(pid, Duration::from_millis(100), 20) {
        println!("ui stop: Agent Ruler UI force-stopped (pid: {}).", pid);
        return Ok(true);
    }

    eprintln!(
        "ui stop: Agent Ruler UI pid {} is still alive after TERM/KILL attempts.",
        pid
    );
    Ok(false)
}

fn process_exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

fn is_agent_ruler_ui_process(pid: u32) -> bool {
    let Some(parts) = read_cmdline_parts(pid) else {
        return false;
    };
    let contains_agent_ruler = parts.iter().any(|part| part.contains("agent-ruler"));
    let contains_ui = parts.iter().any(|part| part == "ui");
    contains_agent_ruler && contains_ui
}

fn discover_orphaned_ui_pids() -> Result<Vec<u32>> {
    let mut pids = Vec::new();
    let self_pid = std::process::id();
    #[cfg(unix)]
    let current_uid = {
        use std::os::unix::fs::MetadataExt;
        fs::metadata("/proc/self")
            .context("read /proc/self metadata")?
            .uid()
    };

    for entry in fs::read_dir("/proc").context("read /proc")? {
        let entry = match entry {
            Ok(value) => value,
            Err(_) => continue,
        };
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if pid == self_pid {
            continue;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let Ok(meta) = fs::metadata(entry.path()) else {
                continue;
            };
            if meta.uid() != current_uid {
                continue;
            }
        }

        let Some(parts) = read_cmdline_parts(pid) else {
            continue;
        };
        let contains_agent_ruler = parts.iter().any(|part| part.contains("agent-ruler"));
        let has_ui_subcommand = parts.iter().any(|part| part == "ui");
        let is_stop_action = parts.iter().any(|part| part == "stop");
        if contains_agent_ruler && has_ui_subcommand && !is_stop_action {
            pids.push(pid);
        }
    }

    Ok(pids)
}

fn read_cmdline_parts(pid: u32) -> Option<Vec<String>> {
    if !process_exists(pid) {
        return None;
    }
    let cmdline_path = format!("/proc/{pid}/cmdline");
    let Ok(raw) = fs::read(&cmdline_path) else {
        return None;
    };
    if raw.is_empty() {
        return None;
    }
    Some(
        raw.split(|byte| *byte == 0)
            .filter(|segment| !segment.is_empty())
            .map(|segment| String::from_utf8_lossy(segment).to_string())
            .collect(),
    )
}

fn send_signal(pid: u32, signal: &str) -> Result<()> {
    Command::new("kill")
        .arg(signal)
        .arg(pid.to_string())
        .status()
        .with_context(|| format!("signal {} to pid {}", signal, pid))?;
    Ok(())
}

fn wait_for_process_exit(pid: u32, delay: Duration, attempts: usize) -> bool {
    for _ in 0..attempts {
        if !process_exists(pid) {
            return true;
        }
        thread::sleep(delay);
    }
    false
}
