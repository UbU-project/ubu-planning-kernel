use std::fs;

use super::{strategy_args, Strategy};
use anyhow::{Context, Result};
use ubu_planning_core::RepairRequest;
use ubu_planning_cpu::{ChunkedSweepStrategy, CpuStrategy};

pub fn run(args: &[String]) -> Result<()> {
    let (path, strategy) = strategy_args(args, "repair")?;
    let input = fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
    let request: RepairRequest = serde_json::from_str(&input)?;
    let response = match strategy {
        Strategy::Greedy => ubu_planning_core::repair(request, &CpuStrategy),
        Strategy::Chunked => ubu_planning_core::repair(request, &ChunkedSweepStrategy::default()),
    };
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}
