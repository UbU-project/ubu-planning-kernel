use serde_json::{json, Value};
use ubu_planning_core::PlanningRequest;
use ubu_planning_cpu::{skeleton::build_skeleton, CpuStrategy};

fn request(tasks: Vec<Value>) -> PlanningRequest {
    serde_json::from_value(json!({
        "schema_version": "planning-kernel-contract/0.1",
        "request_id": "static-placement", "mode": "fresh_generation", "rng_seed": 0,
        "n_rollouts": 64, "time_window": {"start": 0, "end": 100},
        "topological_order": tasks.iter().map(|t| t["id"].clone()).collect::<Vec<_>>(),
        "tasks": tasks
    }))
    .unwrap()
}

fn task(id: &str, duration: u64) -> Value {
    json!({"id": id, "duration": {"type": "fixed", "seconds": duration}})
}

#[test]
fn earlier_dynamic_task_plans_around_later_static_anchor() {
    let mut fixed = task("static", 10);
    fixed["static_anchor"] = json!({"start": 0});
    let plan = build_skeleton(&request(vec![task("dynamic", 10), fixed])).unwrap();
    assert_eq!(
        plan.plan
            .steps
            .iter()
            .map(|s| (s.task_id.as_str(), s.start, s.end))
            .collect::<Vec<_>>(),
        vec![("dynamic", 10, 20), ("static", 0, 10)]
    );
}

#[test]
fn static_dependency_must_finish_before_anchor() {
    let mut fixed = task("static", 10);
    fixed["static_anchor"] = json!({"start": 5});
    fixed["depends_on"] = json!(["dynamic"]);
    let failure = build_skeleton(&request(vec![task("dynamic", 10), fixed])).unwrap_err();
    assert_eq!(failure.task_id.as_deref(), Some("static"));
    assert_eq!(
        failure.reason,
        "static anchor collides with dependencies or window start"
    );
}

#[test]
fn static_collision_preserves_exact_golden_message() {
    let mut a = task("task-a", 4);
    a["static_anchor"] = json!({"start": 0});
    let mut b = task("task-b", 2);
    b["static_anchor"] = json!({"start": 2});
    let response = ubu_planning_core::plan(request(vec![a, b]), &CpuStrategy);
    assert!(response.plan_candidates.is_empty());
    assert!(response.diagnostics.iter().any(|d| d.message ==
        "Could not build deterministic skeleton for task task-b: static anchor collides with scheduled task 'task-a'"));
}

#[test]
fn rollouts_use_time_order_without_changing_emitted_order() {
    let mut fixed = task("static", 10);
    fixed["static_anchor"] = json!({"start": 50});
    let mut dynamic = task("dynamic", 10);
    dynamic["window"] = json!({"start": 0, "end": 20});
    let request = request(vec![fixed, dynamic]);
    let response = ubu_planning_core::plan(request.clone(), &CpuStrategy);
    assert!(!response.plan_candidates.is_empty());
    let baseline = response
        .plan_candidates
        .iter()
        .find(|c| c.schedule.steps[0].task_id == "static")
        .expect("topological emission remains available");
    assert_eq!(baseline.schedule.steps[0].start, 50);
    assert!(baseline.schedule.steps[1].end <= 20);
    let scored =
        ubu_planning_core::rollout::rollout_and_rerank(&request, vec![baseline.clone()]).unwrap();
    for candidate in scored.candidates {
        assert_eq!(
            candidate.rollout_diagnostics.unwrap().feasibility_frequency,
            1.0
        );
    }
}
