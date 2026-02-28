use anyhow::{anyhow, Result};

use agent_ruler::approvals::ApprovalStore;

pub fn resolve_approval_targets(
    approvals: &ApprovalStore,
    id: Option<String>,
    all: bool,
) -> Result<Vec<String>> {
    if all {
        return Ok(approvals
            .list_pending()?
            .into_iter()
            .map(|a| a.id)
            .collect());
    }
    if let Some(id) = id {
        return Ok(vec![id]);
    }

    Err(anyhow!("provide --id <approval-id> or --all"))
}
