use std::fs;

use ubu_planning_core::{
    AffectDirection, AffectLegitimizationMode, AffectObservation, AffectObservationValue,
    AffectProfile, AffectTolerance, PlanningRequest,
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
    assert!(first.plan_candidates.len() <= 16);
}
