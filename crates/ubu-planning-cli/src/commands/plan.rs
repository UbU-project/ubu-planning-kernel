use std::fs;

use anyhow::{Context, Result};
use ubu_planning_core::PlanningRequest;
use ubu_planning_cpu::CpuStrategy;

pub fn run(path: Option<&String>) -> Result<()> {
    let path = path.context("plan requires a PlanningRequest JSON path")?;
    let input = fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
    let request: PlanningRequest = serde_json::from_str(&input)?;
    let response = ubu_planning_core::plan(request, &CpuStrategy);
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}
