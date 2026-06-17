use std::collections::BTreeSet;
use std::fs;

use serde::Deserialize;
use ubu_planning_core::{Plan, PlanningRequest, PlanningResponse};
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
    expected_response: PlanningResponse,
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
        let actual_bytes = serde_json::to_vec(&actual).expect("serialize actual response");
        let expected_bytes =
            serde_json::to_vec(&case.expected_response).expect("serialize expected response");

        assert_eq!(
            actual_bytes,
            expected_bytes,
            "golden case '{}' did not match\n{}",
            case.name,
            readable_diff(&case.expected_response, &actual)
        );
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

fn readable_diff(expected: &PlanningResponse, actual: &PlanningResponse) -> String {
    let expected =
        serde_json::to_string_pretty(expected).expect("serialize expected response for diff");
    let actual = serde_json::to_string_pretty(actual).expect("serialize actual response for diff");
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
