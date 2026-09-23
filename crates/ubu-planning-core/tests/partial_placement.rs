use serde_json::{json, Value};
use ubu_planning_core::{
    plan, HorizonExtension, PlanningRequest, PlanningResponse, ResponseStatus, SelectionRank,
    UnplacedReason,
};
use ubu_planning_cpu::{ChunkedSweepStrategy, CpuStrategy};
fn task(id: &str, seconds: u64, value: f64) -> Value {
    json!({"id":id,"duration":{"type":"fixed","seconds":seconds},"value":value})
}
fn request(tasks: Vec<Value>, end: u64) -> PlanningRequest {
    serde_json::from_value(json!({"schema_version":"planning-kernel-contract/0.1","request_id":"partial","n_rollouts":0,"time_window":{"start":0,"end":end},"tasks":tasks})).unwrap()
}
fn sweep(tasks: Vec<Value>, end: u64) -> PlanningResponse {
    plan(request(tasks, end), &ChunkedSweepStrategy::default())
}
fn check(response: &PlanningResponse, id: &str, reason: UnplacedReason) {
    assert_eq!(response.status, ResponseStatus::Partial, "{response:?}");
    assert!(response.plan_candidates.iter().all(|c| c
        .schedule
        .steps
        .iter()
        .all(|s| s.task_id != id)));
    assert_eq!(
        response
            .unplaced_tasks
            .iter()
            .find(|u| u.task_ref == id)
            .unwrap()
            .reason,
        reason
    );
}
#[test]
fn no_gap_large_enough() {
    let mut fixed = task("fixed", 4, 0.0);
    fixed["static_anchor"] = json!({"start":3});
    check(
        &sweep(vec![fixed, task("optional", 5, 1.0)], 10),
        "optional",
        UnplacedReason::NoEligibleChunkLargeEnough,
    );
}
#[test]
fn prerequisite_closes_allowed_window() {
    let mut child = task("child", 2, 1.0);
    child["depends_on"] = json!(["parent"]);
    child["window"] = json!({"start":0,"end":5});
    let response = sweep(vec![task("parent", 6, 1.0), child], 10);
    check(&response, "child", UnplacedReason::OutsideAllowedWindow);
    assert!(response
        .default_plan()
        .unwrap()
        .steps
        .iter()
        .any(|s| s.task_id == "parent"));
}
#[test]
fn protected_work_consumes_capacity() {
    let mut required = task("required", 10, 0.0);
    required["mandatory"] = json!(true);
    check(
        &sweep(vec![required, task("optional", 10, 1.0)], 10),
        "optional",
        UnplacedReason::InsufficientTotalCapacity,
    );
}
#[test]
fn optional_work_loses_ranking() {
    check(
        &sweep(
            vec![task("a-preferred", 10, 1.0), task("b-other", 10, 0.1)],
            10,
        ),
        "b-other",
        UnplacedReason::OmittedLowerValue,
    );
}
#[test]
fn mandatory_survives_value_fill_rules() {
    let mut required = task("z-required", 10, 0.0);
    required["mandatory"] = json!(true);
    required["window"] = json!({"start":0,"end":10});
    let response = sweep(vec![task("a-valuable", 10, 5.0), required], 20);
    assert_eq!(response.status, ResponseStatus::Ok);
    for candidate in response.plan_candidates {
        assert!(candidate
            .schedule
            .steps
            .iter()
            .any(|s| s.task_id == "z-required" && s.end <= 10));
    }
}
#[test]
fn omission_order_uses_value_deadline_and_id() {
    let rank = |id: &str, value, deadline| SelectionRank {
        task_id: id.into(),
        value,
        deadline_or_latest_finish: deadline,
    };
    let mut report = vec![
        rank("small", 1.0, Some(10)),
        rank("large", 1.0, Some(10)),
        rank("late", 1.0, Some(20)),
        rank("none", 1.0, None),
        rank("low", 0.1, Some(1)),
    ]
    .into_iter()
    .map(|r| {
        ubu_planning_core::UnplacedTask::new(r, UnplacedReason::OmittedLowerValue, vec![], vec![])
    })
    .collect::<Vec<_>>();
    ubu_planning_core::sort_report(&mut report);
    assert_eq!(
        report
            .iter()
            .map(|u| u.task_ref.as_str())
            .collect::<Vec<_>>(),
        ["low", "none", "late", "small", "large"]
    );
    use std::cmp::Ordering::*;
    let compare = ubu_planning_core::compare_omissions;
    assert_eq!(compare(&[], &[rank("a", 1.0, None)]), Less);
    assert_eq!(
        compare(&[rank("z", 1.0, None)], &[rank("a", 1.0, None)]),
        Less
    );
    assert_eq!(
        compare(
            &[rank("a", 1.0, None)],
            &[rank("a", 1.0, None), rank("z", 0.1, None)]
        ),
        Less
    );
}
#[test]
fn partial_wire_round_trip_and_safe_choices() {
    let response = sweep(vec![task("a", 10, 0.91), task("b", 10, 0.1)], 10);
    let wire = serde_json::to_string(&response).unwrap();
    assert_eq!(
        serde_json::from_str::<PlanningResponse>(&wire).unwrap(),
        response
    );
    assert_eq!(
        serde_json::from_str::<Value>(&wire).unwrap()["status"],
        "partial"
    );
    for entry in response.unplaced_tasks {
        assert!(!entry.safe_alternatives.is_empty());
        assert_ne!(
            entry.horizon_extension,
            HorizonExtension::AttemptedExhausted
        );
        assert_eq!(entry.diagnostic_id, format!("unplaced-{}", entry.task_ref));
    }
}
#[test]
fn complete_response_is_ok() {
    let response = sweep(vec![task("a", 10, 1.0)], 20);
    assert_eq!(response.status, ResponseStatus::Ok);
    assert!(response.unplaced_tasks.is_empty());
}
#[test]
fn repair_reports_optional_unplaced_work() {
    let repair=serde_json::from_value(json!({"schema_version":"planning-kernel-contract/0.1","request_id":"repair-partial","candidate":{"plan_id":"prior","status":"candidate","steps":[]},"time_window":{"start":0,"end":10},"tasks":[task("a",10,1.0),task("b",10,0.1)]})).unwrap();
    let response = ubu_planning_core::repair(repair, &ChunkedSweepStrategy::default());
    assert_eq!(response.status, ResponseStatus::Partial);
    assert!(response.repaired_plan.is_some());
    assert_eq!(response.unplaced_tasks[0].task_ref, "b");
    assert_eq!(
        serde_json::from_str::<ubu_planning_core::RepairResponse>(
            &serde_json::to_string(&response).unwrap()
        )
        .unwrap(),
        response
    );
}
#[test]
fn greedy_optional_omission_has_no_chunks() {
    let response = plan(
        request(vec![task("a", 10, 1.0), task("b", 10, 0.1)], 10),
        &CpuStrategy,
    );
    check(&response, "b", UnplacedReason::OmittedLowerValue);
    assert!(response.unplaced_tasks[0].eligible_chunk_refs.is_empty());
}
#[test]
fn greedy_protected_occupancy_is_insufficient_capacity() {
    let mut required = task("a", 10, 0.0);
    required["mandatory"] = json!(true);
    let response = plan(
        request(vec![required, task("b", 10, 1.0)], 10),
        &CpuStrategy,
    );
    check(&response, "b", UnplacedReason::InsufficientTotalCapacity);
}
#[test]
fn all_response_floats_survive_bit_for_bit() {
    let mut request = request(vec![task("a", 3, 0.91), task("b", 2, 0.17)], 10);
    request.n_rollouts = 64;
    let response = plan(request, &ChunkedSweepStrategy::default());
    let parsed: PlanningResponse =
        serde_json::from_str(&serde_json::to_string(&response).unwrap()).unwrap();
    fn floats(value: &Value, result: &mut Vec<u64>) {
        match value {
            Value::Number(n) if n.is_f64() => result.push(n.as_f64().unwrap().to_bits()),
            Value::Array(a) => {
                for v in a {
                    floats(v, result)
                }
            }
            Value::Object(o) => {
                for v in o.values() {
                    floats(v, result)
                }
            }
            _ => {}
        }
    }
    let mut before = vec![];
    let mut after = vec![];
    floats(&serde_json::to_value(&response).unwrap(), &mut before);
    floats(&serde_json::to_value(&parsed).unwrap(), &mut after);
    assert!(!before.is_empty());
    assert_eq!(before, after);
    assert_eq!(response, parsed);
}
