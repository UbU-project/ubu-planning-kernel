use ubu_planning_core::{
    DiagnosticCode, PlanningMode, PlanningRequest, TaskGraph, TaskSpec, TimeWindow,
    PLANNING_SCHEMA_VERSION,
};
use ubu_planning_cpu::CpuStrategy;

#[test]
fn impossible_after_dependency_window_returns_skeleton_failure() {
    let request = PlanningRequest {
        schema_version: Some(PLANNING_SCHEMA_VERSION.to_string()),
        request_id: "skeleton-failure".to_string(),
        mode: PlanningMode::FreshGeneration,
        rng_seed: 0,
        time_window: Some(TimeWindow { start: 0, end: 20 }),
        task_graph: TaskGraph {
            tasks: vec![
                TaskSpec {
                    id: "task-a".to_string(),
                    duration: 10,
                    depends_on: Vec::new(),
                    window: None,
                    static_anchor: None,
                },
                TaskSpec {
                    id: "task-b".to_string(),
                    duration: 1,
                    depends_on: vec!["task-a".to_string()],
                    window: Some(TimeWindow { start: 0, end: 5 }),
                    static_anchor: None,
                },
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
