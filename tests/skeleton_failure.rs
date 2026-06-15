use ubu_planning_core::{
    DiagnosticCode, PlanningRequest, TaskSpec, TimeWindow, PLANNING_SCHEMA_VERSION,
};
use ubu_planning_cpu::CpuStrategy;

#[test]
fn impossible_after_dependency_window_returns_skeleton_failure() {
    let request = PlanningRequest {
        schema_version: Some(PLANNING_SCHEMA_VERSION.to_string()),
        request_id: "skeleton-failure".to_string(),
        tasks: vec![
            TaskSpec {
                id: "task-a".to_string(),
                duration: 10,
                depends_on: Vec::new(),
                window: None,
                static_anchor: None,
                affect_required: false,
                affect_current: false,
            },
            TaskSpec {
                id: "task-b".to_string(),
                duration: 1,
                depends_on: vec!["task-a".to_string()],
                window: Some(TimeWindow { start: 0, end: 5 }),
                static_anchor: None,
                affect_required: false,
                affect_current: false,
            },
        ],
    };

    let response = ubu_planning_core::plan(request, &CpuStrategy);

    assert!(response.plan.is_none());
    assert!(response
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == DiagnosticCode::SkeletonFailure));
}
