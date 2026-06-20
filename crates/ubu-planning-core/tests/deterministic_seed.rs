use std::fs;

use ubu_planning_core::{
    AffectDirection, AffectLegitimizationMode, AffectObservation, AffectObservationValue,
    AffectProfile, AffectTolerance, CorrelationGroup, DurationModel, PlanningRequest,
};
use ubu_planning_cpu::CpuStrategy;

#[test]
fn cpu_strategy_is_deterministic_for_same_request() {
    let input = fs::read_to_string(format!(
        "{}/../../fixtures/planning/valid/dependency-chain.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let mut request: PlanningRequest = serde_json::from_str(&input).unwrap();
    request.affect_profile = Some(AffectProfile {
        mode: AffectLegitimizationMode::Enforce,
        dimensions: [
            (
                "energy".to_string(),
                AffectTolerance {
                    direction: AffectDirection::HigherIsBetter,
                    location: 5.0,
                    scale: 1.0,
                    threshold: 0.5,
                    freshness_seconds: Some(60),
                },
            ),
            (
                "mood_intensity".to_string(),
                AffectTolerance {
                    direction: AffectDirection::LowerIsBetter,
                    location: 5.0,
                    scale: 1.0,
                    threshold: 0.5,
                    freshness_seconds: Some(60),
                },
            ),
        ]
        .into_iter()
        .collect(),
    });
    request.affect_observation = Some(AffectObservation {
        dimensions: [
            (
                "energy".to_string(),
                AffectObservationValue {
                    value: 6.0,
                    observed_at: 0,
                    source_kind: "self_report".to_string(),
                },
            ),
            (
                "mood_intensity".to_string(),
                AffectObservationValue {
                    value: 4.0,
                    observed_at: 0,
                    source_kind: "self_report".to_string(),
                },
            ),
        ]
        .into_iter()
        .collect(),
    });
    for (index, task) in request.task_graph.tasks.iter_mut().enumerate() {
        task.duration = DurationModel::ShiftedLognormalP95 {
            min_seconds: 0,
            mode_seconds: 1,
            p95_seconds: 2 + index as u64,
        };
        task.correlation_groups = vec![CorrelationGroup {
            group: "shared-load".to_string(),
            strength: 0.7,
        }];
    }
    request.n_rollouts = 750;
    request.top_k = 3;
    let expected_stage_seed = request.rng_seed + 3;

    let first = ubu_planning_core::plan(request.clone(), &CpuStrategy);
    let second = ubu_planning_core::plan(request, &CpuStrategy);

    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    assert_eq!(first.plan_candidates, second.plan_candidates);
    assert_eq!(
        serde_json::to_vec(&first.plan_candidates).unwrap(),
        serde_json::to_vec(&second.plan_candidates).unwrap()
    );
    assert!(first.plan_candidates.len() <= 3);
    assert!(first.plan_candidates.iter().all(|candidate| {
        candidate.probability_summary.display_probability.is_some()
            && candidate
                .rollout_diagnostics
                .as_ref()
                .is_some_and(|diagnostics| {
                    diagnostics.n_rollouts == 750 && diagnostics.stage_seed == expected_stage_seed
                })
    }));
}
