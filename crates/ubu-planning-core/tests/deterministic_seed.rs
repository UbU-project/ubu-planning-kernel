use std::fs;

use ubu_planning_core::PlanningRequest;
use ubu_planning_cpu::CpuStrategy;

#[test]
fn cpu_strategy_is_deterministic_for_same_request() {
    let input = fs::read_to_string(format!(
        "{}/../../fixtures/planning/valid/dependency-chain.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let request: PlanningRequest = serde_json::from_str(&input).unwrap();

    let first = ubu_planning_core::plan(request.clone(), &CpuStrategy);
    let second = ubu_planning_core::plan(request, &CpuStrategy);

    assert_eq!(first.plan, second.plan);
}
