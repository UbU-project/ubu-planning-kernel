use std::fs;

use ubu_planning_core::PlanningRequest;
use ubu_planning_cpu::CpuStrategy;

#[test]
fn valid_fixture_produces_plan() {
    let input = fs::read_to_string("fixtures/planning/valid/dependency-chain.json").unwrap();
    let request: PlanningRequest = serde_json::from_str(&input).unwrap();
    let response = ubu_planning_core::plan(request, &CpuStrategy);
    let plan = response.plan.expect("valid fixture should produce a plan");
    let task_ids: Vec<_> = plan.tasks.iter().map(|task| task.task_id.as_str()).collect();
    assert_eq!(task_ids, ["task-a", "task-b", "task-c"]);
}
