use std::fs;

use anyhow::{Context, Result};
use ubu_planning_core::Plan;

pub fn run(path: Option<&String>) -> Result<()> {
    let path = path.context("validate requires a Plan JSON path")?;
    let input = fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
    let plan: Plan = serde_json::from_str(&input)?;
    let response = ubu_planning_core::validate_plan(&plan);
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}
