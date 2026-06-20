use std::fs;

use ubu_planning_core::{DurationModel, PlanningRequest, SemiLegitimizationResult, TaskSpec};
use ubu_planning_cpu::CpuStrategy;

#[test]
fn duration_and_correlation_contract_rejects_invalid_input_at_deserialization() {
    let invalid_ordering = r#"{
        "id":"bad-duration",
        "duration":{"type":"shifted_lognormal_p95","min_seconds":3,"mode_seconds":3,"p95_seconds":8}
    }"#;
    let error = serde_json::from_str::<TaskSpec>(invalid_ordering).unwrap_err();
    assert!(error.to_string().contains("task 'bad-duration'"));
    assert!(error
        .to_string()
        .contains("min_seconds < mode_seconds < p95_seconds"));

    let duplicate_groups = r#"{
        "id":"bad-correlation",
        "duration":{"type":"fixed","seconds":3},
        "correlation_groups":[
            {"group":"context-switching","strength":0.2},
            {"group":"context-switching","strength":0.8}
        ]
    }"#;
    let error = serde_json::from_str::<TaskSpec>(duplicate_groups).unwrap_err();
    assert!(error.to_string().contains("task 'bad-correlation'"));
    assert!(error.to_string().contains("duplicate correlation group"));

    assert!(TaskSpec::new(
        "zero-fixed".to_string(),
        DurationModel::Fixed { seconds: 0 }
    )
    .is_err());

    let valid = r#"{
        "id":"variable-duration",
        "duration":{"type":"shifted_lognormal_p95","min_seconds":0,"mode_seconds":4,"p95_seconds":9},
        "correlation_groups":[{"group":"context-switching","strength":0.5}]
    }"#;
    let task: TaskSpec = serde_json::from_str(valid).unwrap();
    assert_eq!(task.duration.placement_seconds(), 4);
    assert_eq!(task.correlation_groups[0].strength, 0.5);
}

#[test]
fn zero_rollouts_keeps_proxy_and_reports_not_estimated() {
    let input = fs::read_to_string(format!(
        "{}/../../fixtures/planning/valid/dependency-chain.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let mut request: PlanningRequest = serde_json::from_str(&input).unwrap();
    request.n_rollouts = 0;
    let response = ubu_planning_core::plan(request, &CpuStrategy);

    assert!(response.plan_candidates.len() > 3);
    assert!(response.plan_candidates.iter().all(|candidate| {
        candidate.probability_summary.probability_quality
            == ubu_planning_core::ProbabilityQuality::NotEstimated
            && candidate.probability_summary.display_probability.is_none()
            && candidate.rollout_diagnostics.is_none()
    }));
}

#[test]
fn stage_four_response_is_bounded_ranked_and_probability_populated() {
    let input = fs::read_to_string(format!(
        "{}/../../fixtures/planning/valid/dependency-chain.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let request: PlanningRequest = serde_json::from_str(&input).unwrap();
    let mut no_rollout_request = request.clone();
    no_rollout_request.n_rollouts = 0;
    let proxy_default = ubu_planning_core::plan(no_rollout_request, &CpuStrategy)
        .plan_candidates
        .remove(0)
        .candidate_id;
    let response = ubu_planning_core::plan(request, &CpuStrategy);

    assert!(!response.plan_candidates.is_empty());
    assert!(response.plan_candidates.len() <= 3);
    assert_eq!(response.plan_candidates[0].rank, 1);
    assert_ne!(response.plan_candidates[0].candidate_id, proxy_default);
    assert!(response
        .plan_candidates
        .windows(2)
        .all(|pair| pair[0].score_summary.total_score >= pair[1].score_summary.total_score));
    assert!(response.plan_candidates.iter().all(|candidate| {
        candidate.probability_summary.display_probability.is_some()
            && candidate.probability_summary.log_probability.is_some()
            && candidate
                .probability_summary
                .probability_interval_low
                .is_some()
            && candidate
                .probability_summary
                .probability_interval_high
                .is_some()
            && candidate.rollout_diagnostics.is_some()
            && candidate.semi_legitimization_summary.result
                != SemiLegitimizationResult::RejectObvious
    }));
}
