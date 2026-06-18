use std::fs;

use serde_json::Value;
use ubu_planning_core::{
    DiagnosticCode, Plan, PlanStatus, PlanStep, PlanningMode, PlanningRequest, RepairContext,
    RepairScope, StaticAnchor, TaskGraph, TaskSpec, TimeWindow, PLANNING_SCHEMA_VERSION,
};
use ubu_planning_cpu::CpuStrategy;

#[test]
fn emitted_plan_uses_canonical_steps_field() {
    let input = fs::read_to_string(format!(
        "{}/../../fixtures/planning/valid/static-anchor.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let request: PlanningRequest = serde_json::from_str(&input).unwrap();

    let response = ubu_planning_core::plan(request, &CpuStrategy);
    let plan = response.plan.expect("valid fixture should produce a plan");
    let value = serde_json::to_value(plan).unwrap();

    assert!(value.get("steps").is_some());
    assert!(value.get("tasks").is_none());
    assert!(value.get("supersedes_plan_id").is_none());
    assert_eq!(value["steps"][0]["start"], Value::from(5));
    assert_eq!(value["steps"][0]["end"], Value::from(7));
    assert_eq!(value["steps"][0]["static_anchor"], Value::from(true));
}

#[test]
fn provided_topological_order_must_respect_dependencies() {
    let input = fs::read_to_string(format!(
        "{}/../../fixtures/planning/valid/dependency-chain.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let mut request: PlanningRequest = serde_json::from_str(&input).unwrap();
    request.task_graph.topological_order = vec![
        "task-b".to_string(),
        "task-a".to_string(),
        "task-c".to_string(),
    ];

    let response = ubu_planning_core::plan(request, &CpuStrategy);

    assert!(response.plan.is_none());
    assert!(response
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == DiagnosticCode::SkeletonFailure));
}

#[test]
fn static_anchor_collision_returns_skeleton_failure() {
    let request = PlanningRequest {
        schema_version: Some(PLANNING_SCHEMA_VERSION.to_string()),
        request_id: "static-collision".to_string(),
        mode: PlanningMode::FreshGeneration,
        rng_seed: 0,
        time_window: Some(TimeWindow { start: 0, end: 10 }),
        task_graph: TaskGraph {
            tasks: vec![
                task_with_anchor("task-a", 4, &[], Some(0)),
                task_with_anchor("task-b", 2, &[], Some(2)),
            ],
            topological_order: vec!["task-a".to_string(), "task-b".to_string()],
        },
        repair_context: None,
        affect_profile: None,
        affect_observation: None,
        prior_plan: None,
    };

    let response = ubu_planning_core::plan(request, &CpuStrategy);

    assert!(response.plan.is_none());
    assert!(response
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == DiagnosticCode::SkeletonFailure));
}

#[test]
fn repair_supersedes_prior_plan_and_preserves_past_and_in_progress_steps() {
    let prior_plan = Plan {
        plan_id: "plan-prior".to_string(),
        status: PlanStatus::Candidate,
        supersedes_plan_id: None,
        steps: vec![
            step("task-a", 0, 2, &[], false),
            step("task-b", 2, 8, &["task-a"], false),
            step("task-c", 14, 15, &["task-b"], false),
        ],
    };
    let request = PlanningRequest {
        schema_version: Some(PLANNING_SCHEMA_VERSION.to_string()),
        request_id: "repair-skeleton".to_string(),
        mode: PlanningMode::Repair,
        rng_seed: 7,
        time_window: Some(TimeWindow { start: 5, end: 20 }),
        task_graph: TaskGraph {
            tasks: vec![
                task_with_anchor("task-a", 2, &[], None),
                task_with_anchor("task-b", 6, &["task-a"], None),
                task_with_anchor("task-c", 3, &["task-b"], None),
            ],
            topological_order: vec![
                "task-a".to_string(),
                "task-b".to_string(),
                "task-c".to_string(),
            ],
        },
        repair_context: Some(RepairContext {
            prior_plan_id: "plan-prior".to_string(),
            last_legitimate_plan_ref: Some("snapshot-prior".to_string()),
            observed_divergence_refs: vec!["task-c".to_string()],
            repair_scope: RepairScope::RemainingWindow,
        }),
        affect_profile: None,
        affect_observation: None,
        prior_plan: Some(prior_plan),
    };

    let response = ubu_planning_core::plan(request, &CpuStrategy);
    let plan = response.plan.expect("repair should produce a plan");

    assert_eq!(plan.supersedes_plan_id.as_deref(), Some("plan-prior"));
    assert_eq!(plan.steps[0], step("task-a", 0, 2, &[], false));
    assert_eq!(plan.steps[1], step("task-b", 2, 8, &["task-a"], false));
    assert_eq!(plan.steps[2], step("task-c", 8, 11, &["task-b"], false));
}

fn task_with_anchor(
    id: &str,
    duration: u64,
    depends_on: &[&str],
    static_anchor: Option<u64>,
) -> TaskSpec {
    TaskSpec {
        id: id.to_string(),
        duration,
        depends_on: depends_on
            .iter()
            .map(|dependency| dependency.to_string())
            .collect(),
        window: None,
        static_anchor: static_anchor.map(|start| StaticAnchor { start }),
    }
}

fn step(task_id: &str, start: u64, end: u64, depends_on: &[&str], static_anchor: bool) -> PlanStep {
    PlanStep {
        task_id: task_id.to_string(),
        start,
        end,
        depends_on: depends_on
            .iter()
            .map(|dependency| dependency.to_string())
            .collect(),
        static_anchor,
    }
}
