use std::collections::BTreeSet;
use std::fs;

use serde::Deserialize;
use ubu_planning_core::{Diagnostic, Plan, PlanningRequest};
use ubu_planning_cpu::CpuStrategy;

const GOLDEN_CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/planning/golden/skeleton-phase-a.json"
);

#[derive(Debug, Deserialize)]
struct GoldenCorpus {
    schema_version: String,
    human_reviewed_then_frozen: bool,
    cases: Vec<GoldenCase>,
}

#[derive(Debug, Deserialize)]
struct GoldenCase {
    name: String,
    #[allow(dead_code)]
    covers: Vec<String>,
    request: PlanningRequest,
    #[serde(default)]
    prior_plan: Option<Plan>,
    expected_response: PhaseAExpectedResponse,
}

#[derive(Debug, Deserialize)]
struct PhaseAExpectedResponse {
    #[serde(default)]
    plan: Option<Plan>,
    #[serde(default)]
    diagnostics: Vec<Diagnostic>,
}

#[test]
fn phase_a_goldens_match_byte_exact_responses() {
    let input = fs::read_to_string(GOLDEN_CORPUS).expect("read golden corpus");
    let corpus: GoldenCorpus = serde_json::from_str(&input).expect("parse golden corpus");

    assert_eq!(corpus.schema_version, "planning-golden-corpus/phase-a.v1");
    assert!(
        corpus.human_reviewed_then_frozen,
        "golden corpus must be marked human-reviewed-then-frozen"
    );
    assert!(!corpus.cases.is_empty(), "golden corpus must contain cases");
    assert_required_coverage(&corpus.cases);

    for case in corpus.cases {
        let mut request = case.request;
        request.prior_plan = case.prior_plan;

        let actual = ubu_planning_core::plan(request, &CpuStrategy);
        if let Some(expected_plan) = case.expected_response.plan {
            let baseline = actual
                .plan_candidates
                .iter()
                .find(|candidate| candidate.candidate_id == expected_plan.plan_id)
                .unwrap_or_else(|| panic!("golden case '{}' omitted its baseline", case.name));
            assert_eq!(
                baseline.schedule, expected_plan,
                "golden case '{}' baseline changed",
                case.name
            );
        } else {
            assert!(
                actual.plan_candidates.is_empty(),
                "golden case '{}' unexpectedly produced candidates",
                case.name
            );
        }
        let expected_codes: Vec<_> = case
            .expected_response
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code != ubu_planning_core::DiagnosticCode::NotYetImplemented
            })
            .map(|diagnostic| &diagnostic.code)
            .collect();
        for expected_code in expected_codes {
            assert!(actual
                .diagnostics
                .iter()
                .any(|diagnostic| &diagnostic.code == expected_code));
        }
    }
}

fn assert_required_coverage(cases: &[GoldenCase]) {
    let covered: BTreeSet<_> = cases
        .iter()
        .flat_map(|case| case.covers.iter().map(String::as_str))
        .collect();
    let required = [
        "linear_chain",
        "diamond",
        "wide_fan_out",
        "wide_fan_in",
        "disconnected_components",
        "static_anchor",
        "time_window_overflow",
        "missing_starting_state",
        "impossible_dependency",
        "cyclic_dependency",
        "static_collision",
        "insufficient_window",
        "repair_scope_local",
        "repair_scope_remaining_window",
        "repair_scope_full_window",
        "preserve_completed",
        "preserve_in_progress",
        "preserve_user_override",
    ];

    let missing: Vec<_> = required
        .iter()
        .copied()
        .filter(|cover| !covered.contains(cover))
        .collect();
    assert!(
        missing.is_empty(),
        "golden corpus is missing required coverage tags: {missing:?}"
    );
}
