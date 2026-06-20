use std::fs;

use ubu_planning_core::{
    AffectLegitimizationMode, DiagnosticCode, PlanningRequest, PLANNING_SCHEMA_VERSION,
};
use ubu_planning_cpu::CpuStrategy;

#[test]
fn valid_fixture_produces_plan() {
    let input = fs::read_to_string(format!(
        "{}/../../fixtures/planning/valid/dependency-chain.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let request: PlanningRequest = serde_json::from_str(&input).unwrap();
    let response = ubu_planning_core::plan(request, &CpuStrategy);
    assert_eq!(response.schema_version, PLANNING_SCHEMA_VERSION);
    let plan = &response.plan_candidates[0].schedule;
    let task_ids: Vec<_> = plan
        .steps
        .iter()
        .map(|task| task.task_id.as_str())
        .collect();
    assert_eq!(task_ids, ["task-a", "task-b", "task-c"]);
}

#[test]
fn missing_schema_version_returns_diagnostic() {
    let input = fs::read_to_string(format!(
        "{}/../../fixtures/planning/invalid/missing-schema-version.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let request: PlanningRequest = serde_json::from_str(&input).unwrap();
    let response = ubu_planning_core::plan(request, &CpuStrategy);

    assert!(response.plan_candidates.is_empty());
    assert!(response
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == DiagnosticCode::MissingSchemaVersion));
}

#[test]
fn unknown_schema_version_returns_diagnostic() {
    let input = fs::read_to_string(format!(
        "{}/../../fixtures/planning/invalid/unknown-schema-version.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let request: PlanningRequest = serde_json::from_str(&input).unwrap();
    let response = ubu_planning_core::plan(request, &CpuStrategy);

    assert!(response.plan_candidates.is_empty());
    assert!(response
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == DiagnosticCode::UnknownSchemaVersion));
}

#[test]
fn warn_only_affect_violation_records_legitimization_without_rejecting_plan() {
    let input = fs::read_to_string(format!(
        "{}/../../fixtures/planning/valid/affect-break-required.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let request: PlanningRequest = serde_json::from_str(&input).unwrap();
    let response = ubu_planning_core::plan(request, &CpuStrategy);
    assert!(!response.plan_candidates.is_empty());
    let feasibility = &response.plan_candidates[0].feasibility_summary;
    assert!(!feasibility.affect_feasible);
    assert_eq!(feasibility.violated_affect_dimensions, ["energy"]);
}

#[test]
fn enforce_affect_violation_rejects_plan_with_legitimization_report() {
    let input = fs::read_to_string(format!(
        "{}/../../fixtures/planning/valid/affect-break-required.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let mut request: PlanningRequest = serde_json::from_str(&input).unwrap();
    request.affect_profile.as_mut().unwrap().mode = AffectLegitimizationMode::Enforce;

    let response = ubu_planning_core::plan(request, &CpuStrategy);
    assert!(response.plan_candidates.is_empty());
}

#[test]
fn stale_affect_observation_is_reported_without_substitution() {
    let input = fs::read_to_string(format!(
        "{}/../../fixtures/planning/invalid/stale-affect.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let request: PlanningRequest = serde_json::from_str(&input).unwrap();
    let response = ubu_planning_core::plan(request, &CpuStrategy);
    assert!(response.plan_candidates.is_empty());
    assert!(response
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == DiagnosticCode::StaleAffect));
}
