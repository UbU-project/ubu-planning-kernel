use std::collections::BTreeMap;

use ubu_planning_core::{
    legitimization, AffectDirection, AffectLegitimizationMode, AffectObservation,
    AffectObservationValue, AffectProfile, AffectTolerance, DiagnosticCode, LegitimizationResult,
    Plan, PlanStatus, ScheduledTask,
};

#[test]
fn semi_legitimization_stub_stays_explicit() {
    let semi = legitimization::semi_legitimize(&plan_starting_at(0));

    assert!(!semi.is_valid);
    assert_eq!(semi.diagnostics[0].code, DiagnosticCode::NotYetImplemented);
}

#[test]
fn sigmoid_satisfaction_uses_dimension_direction() {
    let energy = tolerance(AffectDirection::HigherIsBetter, 5.0, 2.0, 0.5, None);
    let stress = tolerance(AffectDirection::LowerIsBetter, 5.0, 2.0, 0.5, None);
    let mood_intensity = tolerance(AffectDirection::LowerIsBetter, 4.0, 1.0, 0.5, None);

    assert_close(
        legitimization::satisfaction(&energy, 7.0).unwrap(),
        sigmoid(1.0),
    );
    assert_close(
        legitimization::satisfaction(&stress, 3.0).unwrap(),
        sigmoid(1.0),
    );
    assert_close(
        legitimization::satisfaction(&mood_intensity, 6.0).unwrap(),
        sigmoid(-2.0),
    );
}

#[test]
fn enforce_fails_infeasible_affect_candidate() {
    let profile = profile(
        AffectLegitimizationMode::Enforce,
        [(
            "energy",
            tolerance(AffectDirection::HigherIsBetter, 5.0, 1.0, 0.5, None),
        )],
    );
    let observation = observation([("energy", observed(4.0, 0))]);

    let full =
        legitimization::full_legitimize(&plan_starting_at(0), Some(&profile), Some(&observation));

    assert!(!full.validation.is_valid);
    assert_eq!(full.report.result, LegitimizationResult::Failed);
    assert!(!full.report.affect_feasible);
    assert_eq!(full.report.violated_dimensions, ["energy"]);
    assert!(full.report.affect_margin.unwrap() < 0.0);
}

#[test]
fn warn_only_records_infeasible_affect_without_failing() {
    let profile = profile(
        AffectLegitimizationMode::WarnOnly,
        [(
            "stress",
            tolerance(AffectDirection::LowerIsBetter, 5.0, 1.0, 0.5, None),
        )],
    );
    let observation = observation([("stress", observed(6.0, 0))]);

    let full =
        legitimization::full_legitimize(&plan_starting_at(0), Some(&profile), Some(&observation));

    assert!(full.validation.is_valid);
    assert_eq!(full.report.result, LegitimizationResult::Failed);
    assert!(!full.report.affect_feasible);
    assert_eq!(full.report.violated_dimensions, ["stress"]);
}

#[test]
fn stale_observation_is_reported_and_still_evaluated() {
    let profile = profile(
        AffectLegitimizationMode::Enforce,
        [(
            "energy",
            tolerance(
                AffectDirection::HigherIsBetter,
                5.0,
                1.0,
                0.5,
                Some(5),
            ),
        )],
    );
    let observation = observation([("energy", observed(6.0, 0))]);

    let full =
        legitimization::full_legitimize(&plan_starting_at(6), Some(&profile), Some(&observation));

    assert!(full.validation.is_valid);
    assert_eq!(full.report.result, LegitimizationResult::Passed);
    assert_eq!(full.report.stale_dimensions, ["energy"]);
    assert_eq!(full.validation.diagnostics[0].code, DiagnosticCode::StaleAffect);
    assert!(full.report.dimensions["energy"].satisfaction > 0.5);
}

fn plan_starting_at(start: u64) -> Plan {
    Plan {
        plan_id: "plan-legitimization".to_string(),
        status: PlanStatus::Candidate,
        supersedes_plan_id: None,
        steps: vec![ScheduledTask {
            task_id: "task-a".to_string(),
            start,
            end: start + 1,
            depends_on: Vec::new(),
            static_anchor: false,
        }],
    }
}

fn profile<const N: usize>(
    mode: AffectLegitimizationMode,
    dimensions: [(&str, AffectTolerance); N],
) -> AffectProfile {
    AffectProfile {
        mode,
        dimensions: dimensions
            .into_iter()
            .map(|(dimension, tolerance)| (dimension.to_string(), tolerance))
            .collect(),
    }
}

fn observation<const N: usize>(dimensions: [(&str, AffectObservationValue); N]) -> AffectObservation {
    AffectObservation {
        dimensions: dimensions
            .into_iter()
            .map(|(dimension, value)| (dimension.to_string(), value))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn tolerance(
    direction: AffectDirection,
    location: f64,
    scale: f64,
    threshold: f64,
    freshness_seconds: Option<u64>,
) -> AffectTolerance {
    AffectTolerance {
        direction,
        location,
        scale,
        threshold,
        freshness_seconds,
    }
}

fn observed(value: f64, observed_at: u64) -> AffectObservationValue {
    AffectObservationValue {
        value,
        observed_at,
        source_kind: "self_report".to_string(),
    }
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-12,
        "expected {actual} to be close to {expected}"
    );
}
