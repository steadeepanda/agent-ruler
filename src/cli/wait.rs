use std::path::Path;

use anyhow::{anyhow, Context, Result};

use agent_ruler::approvals::ApprovalStore;
use agent_ruler::config::load_runtime;
use agent_ruler::runners::{reconcile_runner_executable_with_options, RunnerCheckOptions};

/// Wait for an approval decision - useful for agents to poll without failing
pub fn run_wait_for_approval(
    ruler_root: &Path,
    runtime_dir: Option<&Path>,
    approval_id: &str,
    timeout_secs: u64,
    json: bool,
) -> Result<()> {
    use std::time::{Duration, Instant};

    let mut runtime = load_runtime(ruler_root, runtime_dir)
        .context("load runtime (run `agent-ruler init` first)")?;
    let _ = reconcile_runner_executable_with_options(
        &mut runtime,
        "wait",
        RunnerCheckOptions {
            allow_prompt: !json,
            emit_to_stderr: json,
        },
    )?;
    let approvals = ApprovalStore::new(&runtime.config.approvals_file);

    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);
    let poll_interval = Duration::from_millis(500);

    loop {
        match approvals.get(approval_id)? {
            Some(record) => match record.status {
                agent_ruler::model::ApprovalStatus::Pending => {
                    if start.elapsed() >= timeout {
                        if json {
                            println!(
                                "{}",
                                serde_json::to_string(&serde_json::json!({
                                    "approval_id": approval_id,
                                    "status": "timeout",
                                    "message": format!("Timeout after {} seconds", timeout_secs)
                                }))?
                            );
                        } else {
                            println!(
                                "Timeout: approval {} still pending after {}s",
                                approval_id, timeout_secs
                            );
                        }
                        return Ok(());
                    }
                    std::thread::sleep(poll_interval);
                }
                agent_ruler::model::ApprovalStatus::Approved => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string(&serde_json::json!({
                                "approval_id": approval_id,
                                "status": "approved",
                                "decided_at": record.decided_at.map(|t| t.to_rfc3339())
                            }))?
                        );
                    } else {
                        println!("Approved: {}", approval_id);
                    }
                    return Ok(());
                }
                agent_ruler::model::ApprovalStatus::Denied => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string(&serde_json::json!({
                                "approval_id": approval_id,
                                "status": "denied",
                                "decided_at": record.decided_at.map(|t| t.to_rfc3339())
                            }))?
                        );
                    } else {
                        println!("Denied: {}", approval_id);
                    }
                    return Ok(());
                }
                agent_ruler::model::ApprovalStatus::Expired => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string(&serde_json::json!({
                                "approval_id": approval_id,
                                "status": "expired"
                            }))?
                        );
                    } else {
                        println!("Expired: {}", approval_id);
                    }
                    return Ok(());
                }
            },
            None => {
                return Err(anyhow!("Approval {} not found", approval_id));
            }
        }
    }
}
