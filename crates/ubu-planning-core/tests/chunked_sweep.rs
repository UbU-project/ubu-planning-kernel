use serde_json::{json, Value};
use ubu_planning_core::{
    legitimization::{full_legitimize, semi_legitimize},
    Plan, PlannerStrategy, PlanningRequest, SemiLegitimizationResult,
};
use ubu_planning_cpu::{
    chunked::sweep_plans, skeleton::build_skeleton, ChunkedSweepStrategy, CpuStrategy,
};

fn request(tasks: Vec<Value>, end: u64) -> PlanningRequest {
    serde_json::from_value(json!({
        "schema_version": "planning-kernel-contract/0.1", "request_id": "chunked",
        "mode": "fresh_generation", "rng_seed": 42, "n_rollouts": 64,
        "time_window": {"start": 0, "end": end},
        "topological_order": tasks.iter().map(|t| t["id"].clone()).collect::<Vec<_>>(), "tasks": tasks
    })).unwrap()
}
fn task(id: &str, duration: u64, value: f64) -> Value {
    json!({"id": id, "duration": {"type": "fixed", "seconds": duration}, "value": value, "priority": 1.0})
}
fn ranged(mut task: Value, start: u64, end: u64) -> Value {
    task["window"] = json!({"start": start, "end": end});
    task
}
fn dependent(mut task: Value, dep: &str) -> Value {
    task["depends_on"] = json!([dep]);
    task
}
fn fixed(id: &str, start: u64, duration: u64) -> Value {
    let mut task = ranged(task(id, duration, 1.0), start, start + duration);
    task["static_anchor"] = json!({"start": start});
    task
}
fn placements(plan: &Plan) -> Vec<(&str, u64, u64)> {
    plan.steps
        .iter()
        .map(|s| (s.task_id.as_str(), s.start, s.end))
        .collect()
}
fn assert_valid(
    request: &PlanningRequest,
    plan: &Plan,
    unplaced: &[ubu_planning_core::UnplacedTask],
) {
    assert!(ubu_planning_core::validate_plan(plan).is_valid, "{plan:?}");
    let full = full_legitimize(plan, None, None);
    assert_ne!(
        semi_legitimize(plan, request, &full).result,
        SemiLegitimizationResult::RejectObvious,
        "{request:?}\n{plan:?}"
    );
    let mut ids: Vec<_> = plan.steps.iter().map(|s| &s.task_id).collect();
    ids.extend(unplaced.iter().map(|u| &u.task_ref));
    ids.sort();
    let mut expected: Vec<_> = request.tasks().iter().map(|t| &t.id).collect();
    expected.sort();
    assert_eq!(ids, expected);
}

#[test]
fn most_constrained_rescues_narrow_slot() {
    let request = request(
        vec![
            task("a-wide", 10, 1.0),
            ranged(task("b-narrow", 10, 0.1), 0, 10),
        ],
        100,
    );
    assert!(!CpuStrategy
        .generate_candidates(&request)
        .unplaced
        .is_empty());
    let plans = sweep_plans(&request, &ChunkedSweepStrategy::default())
        .unwrap()
        .plans;
    assert_eq!(
        placements(&plans[0]),
        vec![("b-narrow", 0, 10), ("a-wide", 10, 20)]
    );
    assert!(
        sweep_plans(
            &request,
            &ChunkedSweepStrategy {
                alternatives_per_chunk: 1,
                beam_width: 16
            }
        )
        .unwrap()
        .unplaced
        .len()
            == 1
    );
}

#[test]
fn look_ahead_across_static() {
    let request = request(
        vec![
            task("a-flex", 30, 1.0),
            ranged(task("b-tight", 30, 0.1), 0, 50),
            fixed("s", 40, 20),
        ],
        100,
    );
    assert!(!CpuStrategy
        .generate_candidates(&request)
        .unplaced
        .is_empty());
    let plans = sweep_plans(&request, &ChunkedSweepStrategy::default())
        .unwrap()
        .plans;
    assert_eq!(
        placements(&plans[0]),
        vec![("b-tight", 0, 30), ("s", 40, 60), ("a-flex", 60, 90)]
    );
}

#[test]
fn value_first_within_chunk() {
    let request = request(vec![task("a-low", 10, 0.1), task("b-high", 10, 1.0)], 100);
    let plans = sweep_plans(&request, &ChunkedSweepStrategy::default())
        .unwrap()
        .plans;
    assert_eq!(
        placements(&plans[0]),
        vec![("b-high", 0, 10), ("a-low", 10, 20)]
    );
}

#[test]
fn dependencies_propagate_to_fixed_and_movable_dependents() {
    let request = request(
        vec![
            task("a", 10, 0.1),
            dependent(task("b", 10, 1.0), "a"),
            dependent(fixed("s", 15, 10), "a"),
        ],
        60,
    );
    let plans = ChunkedSweepStrategy::default()
        .generate_candidates(&request)
        .plans;
    assert!(!plans.is_empty());
    for plan in plans {
        assert_valid(&request, &plan, &[]);
        let a = plan.steps.iter().find(|s| s.task_id == "a").unwrap();
        let b = plan.steps.iter().find(|s| s.task_id == "b").unwrap();
        assert!(a.end <= 15 && a.end <= b.start);
    }
}

#[test]
fn capacity_look_ahead_prunes_higher_scoring_dead_end() {
    let request = request(
        vec![
            task("a-long", 20, 1.0),
            ranged(task("b-short", 10, 0.1), 0, 40),
            ranged(task("c-short", 10, 0.1), 0, 40),
            fixed("s1", 20, 10),
            fixed("s2", 40, 10),
        ],
        100,
    );
    assert!(!CpuStrategy
        .generate_candidates(&request)
        .unplaced
        .is_empty());
    let plans = sweep_plans(
        &request,
        &ChunkedSweepStrategy {
            beam_width: 1,
            ..Default::default()
        },
    )
    .unwrap()
    .plans;
    assert_eq!(
        placements(&plans[0]),
        vec![
            ("b-short", 0, 10),
            ("c-short", 10, 20),
            ("s1", 20, 30),
            ("s2", 40, 50),
            ("a-long", 50, 70)
        ]
    );
}

#[test]
fn state_merging_keeps_higher_utility_order() {
    let request = request(vec![task("q", 30, 1.0), task("p", 10, 0.9)], 100);
    let plans = sweep_plans(&request, &ChunkedSweepStrategy::default())
        .unwrap()
        .plans;
    assert_eq!(plans.len(), 1);
    assert_eq!(placements(&plans[0]), vec![("p", 0, 10), ("q", 10, 40)]);
}

#[test]
fn contested_slot_omits_optional_task() {
    let request = request(
        vec![
            ranged(task("a", 30, 1.0), 0, 40),
            ranged(task("b", 30, 1.0), 0, 40),
        ],
        100,
    );
    let result = ChunkedSweepStrategy::default().generate_candidates(&request);
    assert!(!result.plans.is_empty());
    assert_eq!(result.unplaced.len(), 1);
    assert_eq!(result.unplaced[0].task_ref, "b");
    for plan in &result.plans {
        assert_valid(&request, plan, &result.unplaced);
    }
}

#[test]
fn repair_preserves_history_and_in_progress_steps() {
    let mut request = request(
        vec![
            task("task-a", 2, 1.0),
            dependent(task("task-b", 6, 1.0), "task-a"),
            dependent(task("task-c", 3, 1.0), "task-b"),
            task("task-d", 4, 1.0),
        ],
        20,
    );
    request.mode = ubu_planning_core::PlanningMode::Repair;
    request.time_window.as_mut().unwrap().start = 5;
    request.repair_context = Some(
        serde_json::from_value(
            json!({"prior_plan_id":"plan-prior","repair_scope":"remaining_window"}),
        )
        .unwrap(),
    );
    request.prior_plan = Some(
        serde_json::from_value(json!({"plan_id":"plan-prior","status":"candidate","steps":[
            {"task_id":"task-a","start":0,"end":2},
            {"task_id":"task-b","start":2,"end":8,"depends_on":["task-a"]},
            {"task_id":"task-c","start":14,"end":17,"depends_on":["task-b"]}
        ]}))
        .unwrap(),
    );
    let result = ChunkedSweepStrategy::default().generate_candidates(&request);
    assert!(!result.plans.is_empty());
    for plan in result.plans {
        assert_valid(&request, &plan, &[]);
        assert_eq!(plan.supersedes_plan_id.as_deref(), Some("plan-prior"));
        assert_eq!(
            &plan.steps[..2],
            &request.prior_plan.as_ref().unwrap().steps[..2]
        );
        assert!(plan.steps[2..].iter().all(|s| s.start >= 8));
    }
    assert!(
        !ubu_planning_core::plan(request, &ChunkedSweepStrategy::default())
            .plan_candidates
            .is_empty()
    );
}

#[test]
fn candidate_set_retains_distinct_greedy_and_reproducible_delays() {
    let equal = request(vec![task("a", 10, 1.0), task("b", 10, 1.0)], 100);
    let strategy = ChunkedSweepStrategy::default();
    let plans = strategy.generate_candidates(&equal).plans;
    assert!(plans.len() >= 4 && plans.len() <= 16);
    assert!(plans[1..].iter().all(|p| p.plan_id.contains("-d")));
    assert_eq!(plans, strategy.generate_candidates(&equal).plans);
    let distinct = request(vec![task("a", 10, 0.1), task("b", 10, 1.0)], 100);
    let plans = strategy.generate_candidates(&distinct).plans;
    assert_eq!(
        plans
            .iter()
            .filter(|p| p.plan_id.ends_with("-greedy"))
            .count(),
        1
    );
    assert!(plans.len() <= 16);
    for p in plans {
        assert_valid(&distinct, &p, &[]);
    }
    // Restricting K makes the sweep omit work; the complete greedy baseline replaces it.
    let narrow = request(vec![task("narrow", 10, 0.1), task("wide", 10, 1.0)], 20);
    let mut narrow = narrow;
    narrow.task_graph.tasks[0].window = Some(ubu_planning_core::TimeWindow { start: 0, end: 10 });
    let strategy = ChunkedSweepStrategy {
        alternatives_per_chunk: 1,
        ..Default::default()
    };
    assert!(!sweep_plans(&narrow, &strategy).unwrap().unplaced.is_empty());
    assert!(strategy.generate_candidates(&narrow).plans[0]
        .plan_id
        .ends_with("-greedy"));
}

#[test]
fn greedy_fallback_delays_use_time_order_for_window_limits() {
    // The first candidate is greedy, with emission order c, a, b but time order a, b, c.
    let mut request = request(
        vec![
            ranged(task("c", 10, 1.0), 50, 70),
            ranged(task("a", 10, 0.1), 0, 10),
            task("b", 10, 1.0),
        ],
        100,
    );
    request.rng_seed = 12;
    let strategy = ChunkedSweepStrategy {
        alternatives_per_chunk: 1,
        ..Default::default()
    };
    assert!(!sweep_plans(&request, &strategy)
        .unwrap()
        .unplaced
        .is_empty());
    let plans = strategy.generate_candidates(&request).plans;
    assert!(plans.len() > 1 && plans[0].plan_id.ends_with("-greedy"));
    for plan in plans {
        assert_valid(&request, &plan, &[]);
    }
}

#[test]
fn fixed_dependencies_reject_before_search_with_exact_messages() {
    let request = request(
        vec![fixed("a", 30, 10), dependent(fixed("b", 10, 10), "a")],
        100,
    );
    let error = sweep_plans(&request, &ChunkedSweepStrategy::default()).unwrap_err();
    assert_eq!(error.task_id.as_deref(), Some("b"));
    assert_eq!(
        error.reason,
        "static anchor collides with dependencies or window start"
    );
    let mut request = request;
    request.mode = ubu_planning_core::PlanningMode::Repair;
    request.repair_context = Some(
        serde_json::from_value(json!({"prior_plan_id":"prior","repair_scope":"local"})).unwrap(),
    );
    request.prior_plan=Some(serde_json::from_value(json!({"plan_id":"prior","status":"candidate","steps":[
        {"task_id":"a","start":30,"end":40}, {"task_id":"b","start":10,"end":20,"depends_on":["a"]}
    ]})).unwrap());
    let error = sweep_plans(&request, &ChunkedSweepStrategy::default()).unwrap_err();
    assert_eq!(
        error.reason,
        "preserved placement starts before dependency 'a' completes"
    );
}

fn utility(request: &PlanningRequest, plan: &Plan) -> f64 {
    let window = request.time_window.as_ref().unwrap();
    plan.steps
        .iter()
        .map(|step| {
            let task = request
                .tasks()
                .iter()
                .find(|t| t.id == step.task_id)
                .unwrap();
            task.value
                * task.priority
                * (1.0
                    - (step.end.saturating_sub(window.start) as f64
                        / (window.end - window.start).max(1) as f64)
                        .clamp(0.0, 1.0))
        })
        .sum()
}
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 32) % bound
    }
}

#[test]
fn randomized_candidates_are_hard_valid_and_never_lose_to_greedy() {
    let mut rng = Lcg(0x17_cafe);
    let mut sweep_only = 0;
    let mut greedy_only = 0;
    let cases = 3000;
    for case in 0..cases {
        let n = 2 + rng.next(7);
        let statics = rng.next(4);
        let mut tasks = Vec::new();
        for i in 0..n {
            let duration = 5 + rng.next(26);
            let mut t = task(&format!("t{i}"), duration, (1 + rng.next(10)) as f64 / 10.0);
            if rng.next(3) != 0 {
                let start = rng.next(80);
                let end = (start + duration + rng.next(61)).min(120);
                t = ranged(t, start, end);
            }
            if i > 0 && rng.next(3) == 0 {
                t = dependent(t, &format!("t{}", rng.next(i)));
            }
            tasks.push(t);
        }
        for i in 0..statics {
            let mut t = fixed(&format!("s{i}"), 20 + i * 35, 5 + rng.next(11));
            if rng.next(3) == 0 {
                t = dependent(t, &format!("t{}", rng.next(n)));
            }
            tasks.push(t);
        }
        let request = request(tasks, 120);
        let greedy = build_skeleton(&request).ok();
        let set = ChunkedSweepStrategy::default().generate_candidates(&request);
        assert!(set.plans.len() <= 16);
        for plan in &set.plans {
            assert_valid(&request, plan, &set.unplaced);
        }
        if let Some(greedy) = greedy {
            if set.plans.is_empty() {
                greedy_only += 1;
            }
            let best = set
                .plans
                .iter()
                .map(|p| utility(&request, p))
                .max_by(f64::total_cmp)
                .unwrap_or(f64::NEG_INFINITY);
            assert!(!set.plans.is_empty(), "greedy survived case {case}");
            if set.unplaced.iter().map(|u| &u.task_ref).collect::<Vec<_>>()
                == greedy
                    .unplaced
                    .iter()
                    .map(|u| &u.task_ref)
                    .collect::<Vec<_>>()
            {
                assert!(
                    best + 1e-12 >= utility(&request, &greedy.plan),
                    "case {case}"
                );
            }
        } else if !set.plans.is_empty() {
            sweep_only += 1;
        }
    }
    println!("P1B-17 property: cases={cases}, sweep-only={sweep_only}, greedy-only={greedy_only}");
    assert_eq!(greedy_only, 0);
    assert!(sweep_only > 0);
}

#[test]
fn pipeline_ranks_fourteen_task_request() {
    let mut tasks: Vec<_> = (0..12)
        .map(|i| task(&format!("task-{i:02}"), 5, (i + 1) as f64 / 12.0))
        .collect();
    tasks.extend([fixed("s1", 30, 10), fixed("s2", 70, 10)]);
    let response = ubu_planning_core::plan(request(tasks, 120), &ChunkedSweepStrategy::default());
    assert!(response.plan_candidates.len() > 1);
    for (i, candidate) in response.plan_candidates.iter().enumerate() {
        assert_eq!(candidate.rank, i + 1);
        assert_eq!(candidate.schedule.steps.len(), 14);
        assert_eq!(
            candidate
                .rollout_diagnostics
                .as_ref()
                .unwrap()
                .feasibility_frequency,
            1.0
        );
    }
}

#[test]
fn impossible_mandatory_task_rejects() {
    let mut required = task("required", 30, 0.0);
    required["mandatory"] = json!(true);
    let response = ubu_planning_core::plan(
        request(vec![required], 20),
        &ChunkedSweepStrategy::default(),
    );
    assert_eq!(response.status, ubu_planning_core::ResponseStatus::Rejected);
    assert!(response.plan_candidates.is_empty() && response.unplaced_tasks.is_empty());
    assert_eq!(
        response.diagnostics[0].code,
        ubu_planning_core::DiagnosticCode::SkeletonFailure
    );
}
#[test]
fn mandatory_keeps_slot_against_more_valuable_work() {
    let mut required = task("z-required", 10, 0.0);
    required["mandatory"] = json!(true);
    let request = request(vec![task("a-valuable", 10, 1.0), required], 10);
    let set = ChunkedSweepStrategy::default().generate_candidates(&request);
    assert_eq!(set.unplaced[0].task_ref, "a-valuable");
    for plan in &set.plans {
        assert_valid(&request, plan, &set.unplaced);
        assert_eq!(plan.steps[0].task_id, "z-required");
    }
}
#[test]
fn omitted_prerequisite_defers_dependents() {
    let request = request_with_deferred_chain();
    let set = ChunkedSweepStrategy::default().generate_candidates(&request);
    let child = set
        .unplaced
        .iter()
        .find(|u| u.task_ref == "c-child")
        .unwrap();
    assert_eq!(
        child.reason,
        ubu_planning_core::UnplacedReason::DeferredDependency
    );
    assert_eq!(child.deferred_by_task_refs, ["b-loser"]);
    assert_eq!(
        set.unplaced
            .iter()
            .find(|u| u.task_ref == "b-loser")
            .unwrap()
            .affected_dependent_task_refs,
        ["c-child"]
    );
    for plan in &set.plans {
        assert_valid(&request, plan, &set.unplaced);
    }
}
fn request_with_deferred_chain() -> PlanningRequest {
    request(
        vec![
            task("a-winner", 9, 1.0),
            task("b-loser", 5, 0.1),
            dependent(task("c-child", 5, 0.1), "b-loser"),
        ],
        10,
    )
}
#[test]
fn mandatory_prerequisite_is_protected() {
    let mut required = dependent(task("z-required", 5, 0.0), "b-prerequisite");
    required["mandatory"] = json!(true);
    let request = request(
        vec![
            task("a-valuable", 10, 10.0),
            task("b-prerequisite", 5, 0.0),
            required,
        ],
        10,
    );
    let set = ChunkedSweepStrategy::default().generate_candidates(&request);
    assert!(!set.plans.is_empty());
    assert_eq!(set.unplaced[0].task_ref, "a-valuable");
    for plan in &set.plans {
        assert_valid(&request, plan, &set.unplaced);
        assert_eq!(plan.steps.len(), 2);
    }
}
#[test]
fn fixed_protection_and_forward_closure_are_transitive() {
    use std::collections::BTreeSet;
    use ubu_planning_cpu::protection::*;
    let request = request(
        vec![
            task("a", 1, 0.0),
            dependent(task("b", 1, 0.0), "a"),
            dependent(task("c", 1, 0.0), "b"),
            task("d", 1, 0.0),
        ],
        10,
    );
    assert_eq!(
        protected_tasks(&request, ["c".into()]),
        BTreeSet::from(["a".into(), "b".into(), "c".into()])
    );
    assert_eq!(
        dependents_of(&dependent_index(&request), &BTreeSet::from(["a".into()])),
        BTreeSet::from(["b".into(), "c".into()])
    );
}
