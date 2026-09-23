use serde_json::{json, Value};
use ubu_planning_core::{
    coverage::*, plan, HorizonPolicy, PlanCandidate, PlanningRequest, ResponseStatus,
};
use ubu_planning_cpu::CpuStrategy;

fn fixed(id: &str, seconds: u64) -> Value {
    json!({"id":id,"duration":{"type":"fixed","seconds":seconds}})
}
fn commitment(id: &str, start: u64) -> Value {
    json!({"id":id,"duration":{"type":"fixed","seconds":300},"static_anchor":{"start":start},"window":{"start":start,"end":start+300}})
}
fn uncertain(id: &str) -> Value {
    json!({"id":id,"duration":{"type":"shifted_lognormal_p95","min_seconds":600,"mode_seconds":1200,"p95_seconds":3600}})
}
fn request(tasks: Vec<Value>, end: u64) -> PlanningRequest {
    serde_json::from_value(json!({"schema_version":"planning-kernel-contract/0.1","request_id":"coverage","rng_seed":42,"n_rollouts":1000,"time_window":{"start":0,"end":end},"tasks":tasks})).unwrap()
}
fn candidate(request: PlanningRequest) -> PlanCandidate {
    let response = plan(request, &CpuStrategy);
    assert_eq!(response.status, ResponseStatus::Ok, "{response:?}");
    response
        .plan_candidates
        .into_iter()
        .find(|c| c.candidate_id == "plan-coverage-000000000000002a")
        .unwrap()
}
#[test]
fn comfortable_day_covers_every_sample() {
    let candidate = candidate(request(vec![fixed("a", 600), commitment("b", 2400)], 6000));
    let coverage = candidate.coverage.unwrap();
    assert_eq!(coverage.coverage_estimate, 1.0);
    assert_eq!(coverage.uncovered_mass_estimate, 0.0);
    assert_eq!(coverage.coverage_scope, CoverageScope::ReactiveHorizon);
    assert_eq!(coverage.coverage_threshold_used, 0.99);
    assert!(!coverage.coverage_below_threshold);
    let summary = coverage.outcome_continuation_summary;
    assert_eq!(summary.uncovered_outcome_count, 0);
    assert_eq!(summary.boundaries.len(), 1);
    assert_eq!(summary.quantization_rule, LATENESS_QUANTIZATION_RULE);
    assert!(!summary.budget_limited);
    assert_eq!(coverage.coverage_confidence.method, "wilson");
    assert_eq!(coverage.coverage_confidence.n_rollouts, 1000);
}
#[test]
fn uncertain_work_attributes_uncovered_mass_to_commitment() {
    let candidate = candidate(request(vec![uncertain("a"), commitment("b", 2400)], 6000));
    let coverage = candidate.coverage.unwrap();
    assert!(coverage.coverage_estimate > 0.0 && coverage.coverage_estimate < 1.0);
    assert!(coverage.coverage_below_threshold);
    assert_eq!(
        coverage.coverage_estimate + coverage.uncovered_mass_estimate,
        1.0
    );
    assert!(
        coverage.coverage_confidence.lower <= coverage.coverage_estimate
            && coverage.coverage_confidence.upper >= coverage.coverage_estimate
    );
    assert!(
        coverage.coverage_estimate >= candidate.probability_summary.display_probability.unwrap()
    );
    let boundary = &coverage.outcome_continuation_summary.boundaries[0];
    assert_eq!(boundary.boundary_task_ref, "b");
    assert!(boundary.uncovered_outcome_count > 0);
    assert!(boundary
        .heaviest_uncovered_digest
        .as_ref()
        .unwrap()
        .starts_with("outcome-"));
    assert!((boundary.uncovered_mass - coverage.uncovered_mass_estimate).abs() < 1e-12);
}
#[test]
fn lateness_rounds_up_and_zero_is_distinct() {
    for (seconds, expected) in [
        (-1.0, 0),
        (0.0, 0),
        (0.001, 1),
        (59.9, 1),
        (60.0, 1),
        (60.001, 2),
        (120.0, 2),
    ] {
        assert_eq!(lateness_bucket(seconds), expected);
    }
    // A mixed merged state is not certified, but only its failed samples add mass.
    let state: OutcomeState=serde_json::from_value(json!({"boundary_index":0,"boundary_start":2400,"boundary_task_ref":"b","completed":[],"lateness_bucket":0})).unwrap();
    let mut counter = CoverageCounter::default();
    counter.observe([state.clone()], true);
    counter.observe([state], false);
    let summary = counter.summarize(CoverageScope::ReactiveHorizon, 0.99, false);
    assert_eq!(summary.coverage_estimate, 0.5);
    assert_eq!(
        summary.outcome_continuation_summary.covered_outcome_count,
        0
    );
    assert_eq!(
        summary.outcome_continuation_summary.uncovered_outcome_count,
        1
    );
    assert_eq!(
        summary.outcome_continuation_summary.boundaries[0].uncovered_mass,
        0.5
    );
}
#[test]
fn digest_is_stable_and_names_state_contents() {
    let state: OutcomeState=serde_json::from_value(json!({"boundary_index":0,"boundary_start":2400,"boundary_task_ref":"commitment","completed":["a"],"lateness_bucket":0})).unwrap();
    assert_eq!(state.digest(), "outcome-7a4a917239a307f4");
    assert_eq!(state.digest(), state.clone().digest());
    let mut changed = state.clone();
    changed.completed.insert("b".into());
    assert_ne!(state.digest(), changed.digest());
    changed = state.clone();
    changed.lateness_bucket = 1;
    assert_ne!(state.digest(), changed.digest());
    changed = state.clone();
    changed.completed.clear();
    assert_ne!(state.digest(), changed.digest());
    let mut first = CoverageCounter::default();
    let mut reversed = CoverageCounter::default();
    first.observe([state.clone()], false);
    first.observe([changed.clone()], false);
    reversed.observe([changed.clone()], false);
    reversed.observe([state.clone()], false);
    let a = first.summarize(CoverageScope::ReactiveHorizon, 0.99, false);
    assert_eq!(
        a,
        reversed.summarize(CoverageScope::ReactiveHorizon, 0.99, false)
    );
    assert_eq!(
        a.outcome_continuation_summary.boundaries[0].heaviest_uncovered_digest,
        Some(state.digest().min(changed.digest()))
    );
}
#[test]
fn reactive_horizon_selects_boundaries() {
    let tasks = vec![
        fixed("a", 600),
        commitment("b", 2400),
        commitment("c", 14400),
    ];
    let mut narrow = request(tasks.clone(), 18000);
    narrow.horizon_policy =
        serde_json::from_value(json!({"reactive_horizon_seconds":2400})).unwrap();
    assert_eq!(
        candidate(narrow)
            .coverage
            .unwrap()
            .outcome_continuation_summary
            .boundaries
            .len(),
        1
    );
    let mut wide = request(tasks, 18000);
    wide.horizon_policy = serde_json::from_value(
        json!({"reactive_horizon_seconds":20000,"branch_coverage_target":0.9}),
    )
    .unwrap();
    let coverage = candidate(wide).coverage.unwrap();
    assert_eq!(coverage.outcome_continuation_summary.boundaries.len(), 2);
    assert_eq!(coverage.coverage_threshold_used, 0.9);
}
#[test]
fn optional_omission_is_a_certified_continuation() {
    let mut slow = uncertain("a");
    slow["mandatory"] = json!(true);
    let optional = json!({"id":"c","duration":{"type":"shifted_lognormal_p95","min_seconds":60,"mode_seconds":600,"p95_seconds":2400},"window":{"start":2700,"end":3300}});
    let candidate = candidate(request(vec![slow, commitment("b", 2400), optional], 6000));
    let probability = candidate.probability_summary.display_probability.unwrap();
    let coverage = candidate.coverage.unwrap();
    assert!(coverage.coverage_estimate > probability);
    assert_eq!(
        coverage.outcome_continuation_summary.boundaries[0].boundary_task_ref,
        "b"
    );
    println!(
        "P1B21_GAP display_probability={probability} coverage_estimate={} gap={}",
        coverage.coverage_estimate,
        coverage.coverage_estimate - probability
    );
}
#[test]
fn protected_only_continuation_equals_feasibility() {
    let mut slow = uncertain("a");
    slow["mandatory"] = json!(true);
    let candidate = candidate(request(vec![slow, commitment("b", 2400)], 6000));
    assert_eq!(
        candidate.coverage.unwrap().coverage_estimate,
        candidate.probability_summary.display_probability.unwrap()
    );
}
#[test]
fn disabled_rollout_has_no_coverage_and_policy_is_validated() {
    let mut req = request(vec![fixed("a", 600), commitment("b", 2400)], 6000);
    req.n_rollouts = 0;
    let response = plan(req.clone(), &CpuStrategy);
    assert!(response
        .plan_candidates
        .iter()
        .all(|c| c.coverage.is_none()));
    assert_eq!(
        serde_json::from_value::<HorizonPolicy>(json!({})).unwrap(),
        HorizonPolicy::default()
    );
    for policy in [
        json!({"reactive_horizon_seconds":0}),
        json!({"branch_coverage_target":0}),
        json!({"branch_coverage_target":1.1}),
        json!({"branch_coverage_target":-0.1}),
    ] {
        req.horizon_policy = serde_json::from_value(policy).unwrap();
        assert_eq!(
            plan(req.clone(), &CpuStrategy).status,
            ResponseStatus::Rejected
        );
    }
    req.horizon_policy = HorizonPolicy {
        reactive_horizon_seconds: 1,
        branch_coverage_target: f64::NAN,
    };
    assert!(req.horizon_policy.validate().is_err());
}
