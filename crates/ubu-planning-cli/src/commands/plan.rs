use std::fs;

use super::{strategy_args, Strategy};
use anyhow::{Context, Result};
use ubu_planning_core::PlanningRequest;
use ubu_planning_cpu::{ChunkedSweepStrategy, CpuStrategy};

pub fn run(args: &[String]) -> Result<()> {
    let (path, strategy) = strategy_args(args, "plan")?;
    let input = fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
    let request: PlanningRequest = serde_json::from_str(&input)?;
    let response = match strategy {
        Strategy::Greedy => ubu_planning_core::plan(request, &CpuStrategy),
        Strategy::Chunked => ubu_planning_core::plan(request, &ChunkedSweepStrategy::default()),
    };
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}
