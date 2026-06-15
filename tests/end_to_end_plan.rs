use std::fs;

use ubu_planning_core::{DiagnosticCode, PlanningRequest, PLANNING_SCHEMA_VERSION};
use ubu_planning_cpu::CpuStrategy;

#[test]
fn valid_fixture_produces_plan() {
    let input = fs::read_to_string("fixtures/planning/valid/dependency-chain.json").unwrap();
    let request: PlanningRequest = serde_json::from_str(&input).unwrap();
    let response = ubu_planning_core::plan(request, &CpuStrategy);
    assert_eq!(response.schema_version, PLANNING_SCHEMA_VERSION);
    let plan = response.plan.expect("valid fixture should produce a plan");
    let task_ids: Vec<_> = plan.tasks.iter().map(|task| task.task_id.as_str()).collect();
    assert_eq!(task_ids, ["task-a", "task-b", "task-c"]);
}

#[test]
fn missing_schema_version_returns_diagnostic() {
    let input = fs::read_to_string("fixtures/planning/invalid/missing-schema-version.json").unwrap();
    let request: PlanningRequest = serde_json::from_str(&input).unwrap();
    let response = ubu_planning_core::plan(request, &CpuStrategy);

    assert!(response.plan.is_none());
    assert!(response
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == DiagnosticCode::MissingSchemaVersion));
}

#[test]
fn unknown_schema_version_returns_diagnostic() {
    let input = fs::read_to_string("fixtures/planning/invalid/unknown-schema-version.json").unwrap();
    let request: PlanningRequest = serde_json::from_str(&input).unwrap();
    let response = ubu_planning_core::plan(request, &CpuStrategy);

    assert!(response.plan.is_none());
    assert!(response
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == DiagnosticCode::UnknownSchemaVersion));
}
