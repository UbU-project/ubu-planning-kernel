use ubu_planning_core::{
    DurationModel, PlanningMode, PlanningRequest, ResponseStatus, TaskGraph, TaskSpec, TimeWindow,
    UnplacedReason, PLANNING_SCHEMA_VERSION,
};
use ubu_planning_cpu::CpuStrategy;

#[test]
fn impossible_after_dependency_window_returns_partial() {
    let request = PlanningRequest {
        schema_version: Some(PLANNING_SCHEMA_VERSION.to_string()),
        request_id: "skeleton-failure".to_string(),
        mode: PlanningMode::FreshGeneration,
        rng_seed: 0,
        n_rollouts: 0,
        top_k: 3,
        strict_validation: false,
        time_window: Some(TimeWindow { start: 0, end: 20 }),
        task_graph: TaskGraph {
            tasks: vec![
                TaskSpec {
                    id: "task-a".to_string(),
                    duration: DurationModel::Fixed { seconds: 10 },
                    correlation_groups: Vec::new(),
                    value: 1.0,
                    priority: 1.0,
                    mandatory: false,
                    depends_on: Vec::new(),
                    window: None,
                    static_anchor: None,
                },
                TaskSpec {
                    id: "task-b".to_string(),
                    duration: DurationModel::Fixed { seconds: 1 },
                    correlation_groups: Vec::new(),
                    value: 1.0,
                    priority: 1.0,
                    mandatory: false,
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
        scoring_policy: Default::default(),
        prior_plan: None,
    };

    let response = ubu_planning_core::plan(request, &CpuStrategy);

    assert_eq!(response.status, ResponseStatus::Partial);
    assert_eq!(response.default_plan().unwrap().steps.len(), 1);
    assert_eq!(response.unplaced_tasks[0].task_ref, "task-b");
    assert_eq!(
        response.unplaced_tasks[0].reason,
        UnplacedReason::OutsideAllowedWindow
    );
    assert!(response.diagnostics.is_empty());
}
