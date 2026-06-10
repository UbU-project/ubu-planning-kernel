use std::fs;

use anyhow::{Context, Result};
use ubu_planning_core::RepairRequest;
use ubu_planning_cpu::CpuStrategy;

pub fn run(path: Option<&String>) -> Result<()> {
    let path = path.context("repair requires a RepairRequest JSON path")?;
    let input = fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
    let request: RepairRequest = serde_json::from_str(&input)?;
    let response = ubu_planning_core::repair(request, &CpuStrategy);
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}
