use std::path::Path;

use std::collections::BTreeSet;

use anyhow::{anyhow, Context, Result};

use ::agent_ruler::config::load_runtime;
use ::agent_ruler::doctor::{run as run_doctor_checks, DoctorOptions, RepairSelection};

pub fn run_doctor(
    ruler_root: &Path,
    runtime_dir: Option<&Path>,
    repair: Option<&str>,
    json: bool,
) -> Result<()> {
    let mut runtime = load_runtime(ruler_root, runtime_dir)
        .context("load runtime (run `agent-ruler init` first)")?;
    let report = run_doctor_checks(
        &mut runtime,
        DoctorOptions {
            repair: parse_repair_selection(repair)?,
        },
    )?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).context("serialize doctor report as JSON")?
        );
    } else {
        println!("{}", report.output);
    }

    Ok(())
}

fn parse_repair_selection(raw: Option<&str>) -> Result<RepairSelection> {
    let Some(raw) = raw.map(str::trim) else {
        return Ok(RepairSelection::None);
    };
    if raw.is_empty() || raw.eq_ignore_ascii_case("all") {
        return Ok(RepairSelection::All);
    }

    let mut numbers = BTreeSet::new();
    for piece in raw.split(',') {
        let trimmed = piece.trim();
        if trimmed.is_empty() {
            continue;
        }
        let number = trimmed
            .parse::<usize>()
            .with_context(|| format!("invalid doctor repair target `{trimmed}`"))?;
        if number == 0 {
            return Err(anyhow!("doctor repair targets are 1-based check numbers"));
        }
        numbers.insert(number);
    }

    if numbers.is_empty() {
        return Err(anyhow!(
            "doctor repair selection was empty; use `--repair`, `--repair all`, or `--repair 1,4,6`"
        ));
    }
    Ok(RepairSelection::Checks(numbers))
}
