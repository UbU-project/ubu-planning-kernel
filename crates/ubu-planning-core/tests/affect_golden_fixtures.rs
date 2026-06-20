use std::collections::BTreeSet;
use std::fs;

use serde::{Deserialize, Serialize};
use ubu_planning_core::strategy::PlannerStrategy;
use ubu_planning_core::{
    legitimization, AffectDirection, AffectTolerance, LegitimizationReport, PlanningRequest,
};
use ubu_planning_cpu::CpuStrategy;

const AFFECT_GOLDEN_CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/planning/golden/affect-legitimization.json"
);

#[derive(Debug, Deserialize)]
struct AffectGoldenCorpus {
    schema_version: String,
    human_reviewed_then_frozen: bool,
    #[allow(dead_code)]
    description: String,
    cases: Vec<AffectGoldenCase>,
}

#[derive(Debug, Deserialize)]
struct AffectGoldenCase {
    name: String,
    covers: Vec<String>,
    #[allow(dead_code)]
    review_warning: Option<String>,
    request: PlanningRequest,
    expected_plan_present: bool,
    expected_legitimization: LegitimizationReport,
}

#[test]
fn affect_goldens_match_byte_exact_legitimization_reports() {
    let input = fs::read_to_string(AFFECT_GOLDEN_CORPUS).expect("read affect golden corpus");
    let corpus: AffectGoldenCorpus = serde_json::from_str(&input).expect("parse affect corpus");

    assert_eq!(
        corpus.schema_version,
        "planning-golden-corpus/affect-legitimization.v1"
    );
    assert!(
        corpus.human_reviewed_then_frozen,
        "affect golden corpus must be marked human-reviewed-then-frozen"
    );
    assert!(!corpus.cases.is_empty(), "affect corpus must contain cases");
    assert_required_coverage(&corpus.cases);
    assert_bootstrap_priors_are_marked(&corpus.cases);

    for case in corpus.cases {
        let generated = CpuStrategy.generate_candidates(&case.request);
        let plan = generated
            .plans
            .first()
            .expect("affect golden request must generate a skeleton");
        let actual = legitimization::full_legitimize(
            plan,
            case.request.affect_profile.as_ref(),
            case.request.affect_observation.as_ref(),
        );
        assert_eq!(
            actual.validation.is_valid, case.expected_plan_present,
            "golden case '{}' plan presence changed",
            case.name
        );
        let actual_legitimization = actual.report;

        let expected_bytes = serde_json::to_vec(&case.expected_legitimization)
            .expect("serialize expected legitimization");
        let actual_bytes =
            serde_json::to_vec(&actual_legitimization).expect("serialize actual legitimization");

        assert_eq!(
            actual_bytes,
            expected_bytes,
            "affect golden case '{}' did not match\n{}",
            case.name,
            readable_diff(&case.expected_legitimization, &actual_legitimization)
        );
    }
}

#[test]
fn affect_satisfaction_matches_section_6_formulas() {
    let energy = tolerance(AffectDirection::HigherIsBetter, 5.0, 2.0, 0.5);
    let stress = tolerance(AffectDirection::LowerIsBetter, 5.0, 2.0, 0.5);
    let mood_intensity = tolerance(AffectDirection::LowerIsBetter, 8.0, 1.5, 0.5);

    assert_close(
        legitimization::satisfaction(&energy, 7.0).unwrap(),
        sigmoid((7.0 - 5.0) / 2.0),
    );
    assert_close(
        legitimization::satisfaction(&stress, 3.0).unwrap(),
        sigmoid((5.0 - 3.0) / 2.0),
    );
    assert_close(
        legitimization::satisfaction(&mood_intensity, 9.0).unwrap(),
        sigmoid((8.0 - 9.0) / 1.5),
    );
}

fn assert_required_coverage(cases: &[AffectGoldenCase]) {
    let covered: BTreeSet<_> = cases
        .iter()
        .flat_map(|case| case.covers.iter().map(String::as_str))
        .collect();
    let required = [
        "fully_feasible",
        "energy_violation",
        "stress_violation",
        "mood_intensity_violation",
        "multi_violation",
        "boundary_inside",
        "boundary_outside",
        "enforce_mode",
        "warn_only_mode",
        "same_infeasible_input",
        "bootstrap_default_profile_marked_priors",
        "stale_observation",
        "missing_observation",
    ];

    let missing: Vec<_> = required
        .iter()
        .copied()
        .filter(|cover| !covered.contains(cover))
        .collect();
    assert!(
        missing.is_empty(),
        "affect golden corpus is missing required coverage tags: {missing:?}"
    );
}

fn assert_bootstrap_priors_are_marked(cases: &[AffectGoldenCase]) {
    let bootstrap_cases: Vec<_> = cases
        .iter()
        .filter(|case| {
            case.covers
                .iter()
                .any(|cover| cover == "bootstrap_default_profile_marked_priors")
        })
        .collect();
    assert!(
        !bootstrap_cases.is_empty(),
        "bootstrap default profile case must be present"
    );
    for case in bootstrap_cases {
        let warning = case
            .review_warning
            .as_deref()
            .expect("bootstrap default profile case must carry a review warning");
        assert!(
            warning.contains("marked priors"),
            "bootstrap default profile case must explicitly mark priors"
        );
    }
}

fn readable_diff<T>(expected: &T, actual: &T) -> String
where
    T: Serialize,
{
    let expected = serde_json::to_string_pretty(expected).expect("serialize expected for diff");
    let actual = serde_json::to_string_pretty(actual).expect("serialize actual for diff");
    let expected_lines: Vec<_> = expected.lines().collect();
    let actual_lines: Vec<_> = actual.lines().collect();
    let max_lines = expected_lines.len().max(actual_lines.len());

    let mut diff = String::from("--- expected\n+++ actual\n");
    for index in 0..max_lines {
        let expected_line = expected_lines.get(index).copied();
        let actual_line = actual_lines.get(index).copied();
        if expected_line == actual_line {
            continue;
        }
        if let Some(line) = expected_line {
            diff.push_str("- ");
            diff.push_str(line);
            diff.push('\n');
        }
        if let Some(line) = actual_line {
            diff.push_str("+ ");
            diff.push_str(line);
            diff.push('\n');
        }
    }
    diff
}

fn tolerance(
    direction: AffectDirection,
    location: f64,
    scale: f64,
    threshold: f64,
) -> AffectTolerance {
    AffectTolerance {
        direction,
        location,
        scale,
        threshold,
        freshness_seconds: None,
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
