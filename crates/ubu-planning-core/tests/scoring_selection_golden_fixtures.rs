use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use ubu_planning_core::strategy::PlannerStrategy;
use ubu_planning_core::{legitimization, CandidateRole, PlanningRequest, SemiLegitimizationResult};
use ubu_planning_cpu::CpuStrategy;

const GOLDEN_CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/planning/golden/scoring-selection-c1.json"
);
const POST_P7_KERNEL_REVISION: &str = "2ae6552f6157c44f6490e4c99aaf4a418fc70e67";
const MAX_CANDIDATES: usize = 16;

#[derive(Debug, Deserialize)]
struct GoldenCorpus {
    schema_version: String,
    human_reviewed_then_frozen: bool,
    kernel_revision: String,
    #[allow(dead_code)]
    description: String,
    cases: Vec<GoldenCase>,
}

#[derive(Debug, Deserialize)]
struct GoldenCase {
    name: String,
    covers: Vec<String>,
    request: PlanningRequest,
    expected_generated_candidate_count: usize,
    expected_semi_rejected_count: usize,
    expected_selected_candidate_id: String,
    expected_plan_candidates: Box<RawValue>,
}

#[test]
fn scoring_and_selection_goldens_match_byte_exact_ranked_candidates() {
    let input = fs::read_to_string(GOLDEN_CORPUS).expect("read C-1 golden corpus");
    let corpus: GoldenCorpus = serde_json::from_str(&input).expect("parse C-1 golden corpus");

    assert_eq!(
        corpus.schema_version,
        "planning-golden-corpus/scoring-selection-c1.v1"
    );
    assert!(
        corpus.human_reviewed_then_frozen,
        "C-1 golden corpus must be marked human-reviewed-then-frozen"
    );
    assert_eq!(corpus.kernel_revision, POST_P7_KERNEL_REVISION);
    assert_required_coverage(&corpus.cases);

    let mut weighting_winners = BTreeMap::new();
    let mut observed_roles = BTreeSet::new();
    let mut observed_exact_score_tie = false;

    for case in corpus.cases {
        let first_generated = CpuStrategy.generate_candidates(&case.request);
        let second_generated = CpuStrategy.generate_candidates(&case.request);
        assert_byte_exact(
            &case.name,
            "generated candidate set changed between identical fixed-seed runs",
            &first_generated.plans,
            &second_generated.plans,
        );
        assert_eq!(
            first_generated.plans.len(),
            case.expected_generated_candidate_count,
            "golden case '{}' generated candidate count changed",
            case.name
        );
        assert!(
            first_generated.plans.len() <= MAX_CANDIDATES,
            "golden case '{}' exceeded the candidate cap",
            case.name
        );
        if case.covers.iter().any(|cover| cover == "cap_sixteen") {
            assert_eq!(first_generated.plans.len(), MAX_CANDIDATES);
        }

        let semi_rejected_count = first_generated
            .plans
            .iter()
            .filter(|candidate| {
                let full = legitimization::full_legitimize(
                    candidate,
                    case.request.affect_profile.as_ref(),
                    case.request.affect_observation.as_ref(),
                );
                legitimization::semi_legitimize(candidate, &case.request, &full).result
                    == SemiLegitimizationResult::RejectObvious
            })
            .count();
        assert_eq!(
            semi_rejected_count, case.expected_semi_rejected_count,
            "golden case '{}' semi-legitimization prune count changed",
            case.name
        );

        let first = ubu_planning_core::plan(case.request.clone(), &CpuStrategy);
        let second = ubu_planning_core::plan(case.request, &CpuStrategy);
        assert_byte_exact(
            &case.name,
            "ranked candidates changed between identical fixed-seed runs",
            &first.plan_candidates,
            &second.plan_candidates,
        );
        assert_raw_json_byte_exact(
            &case.name,
            "ranked candidates differ from frozen golden",
            &case.expected_plan_candidates,
            &first.plan_candidates,
        );
        assert_eq!(
            first
                .plan_candidates
                .first()
                .map(|candidate| candidate.candidate_id.as_str()),
            Some(case.expected_selected_candidate_id.as_str()),
            "golden case '{}' rank-1 selection changed",
            case.name
        );
        assert!(
            first.plan_candidates.iter().all(|candidate| {
                candidate.probability_summary.display_probability.is_none()
                    && candidate.probability_summary.log_probability.is_none()
                    && candidate.probability_summary.probability_interval.is_none()
                    && candidate.probability_summary.provenance_refs.is_empty()
            }),
            "golden case '{}' must remain C-1-only with empty probability summaries",
            case.name
        );

        observed_roles.extend(first.plan_candidates.iter().map(|candidate| {
            match candidate.candidate_role {
                CandidateRole::HighestUtility => "highest_utility",
                CandidateRole::MostRobust => "most_robust",
                CandidateRole::MostScheduleDiverse => "most_schedule_diverse",
                CandidateRole::Other => "other",
            }
        }));
        if case.covers.iter().any(|cover| cover == "exact_score_tie") {
            observed_exact_score_tie = first.plan_candidates.windows(2).any(|pair| {
                pair[0].score_summary == pair[1].score_summary
                    && pair[0].candidate_id < pair[1].candidate_id
            });
        }
        for weighting in ["utility_heavy", "robustness_heavy", "diversity_heavy"] {
            if case.covers.iter().any(|cover| cover == weighting) {
                weighting_winners.insert(weighting, case.expected_selected_candidate_id.clone());
            }
        }
    }

    assert_eq!(weighting_winners.len(), 3);
    assert_eq!(
        weighting_winners.values().collect::<BTreeSet<_>>().len(),
        3,
        "utility, robustness, and diversity weightings must select different rank-1 candidates"
    );
    assert_eq!(
        observed_roles,
        BTreeSet::from([
            "highest_utility",
            "most_robust",
            "most_schedule_diverse",
            "other",
        ])
    );
    assert!(
        observed_exact_score_tie,
        "tie fixture must contain adjacent equal scores resolved by candidate_id ascending"
    );
}

fn assert_required_coverage(cases: &[GoldenCase]) {
    let covered: BTreeSet<_> = cases
        .iter()
        .flat_map(|case| case.covers.iter().map(String::as_str))
        .collect();
    let required = [
        "abundant_slack",
        "cap_sixteen",
        "tightly_constrained",
        "semi_legitimization_pruned",
        "reject_obvious",
        "utility_heavy",
        "robustness_heavy",
        "diversity_heavy",
        "weighting_sensitive",
        "deterministic_tiebreak",
        "exact_score_tie",
        "candidate_role_highest_utility",
        "candidate_role_most_robust",
        "candidate_role_most_schedule_diverse",
        "candidate_role_other",
    ];
    let missing: Vec<_> = required
        .iter()
        .copied()
        .filter(|cover| !covered.contains(cover))
        .collect();
    assert!(
        missing.is_empty(),
        "C-1 golden corpus is missing required coverage tags: {missing:?}"
    );
}

fn assert_byte_exact<T>(case_name: &str, message: &str, expected: &T, actual: &T)
where
    T: Serialize,
{
    let expected_bytes = serde_json::to_vec(expected).expect("serialize expected bytes");
    let actual_bytes = serde_json::to_vec(actual).expect("serialize actual bytes");
    assert_eq!(
        actual_bytes,
        expected_bytes,
        "golden case '{case_name}' {message}\n{}",
        readable_diff(expected, actual)
    );
}

fn assert_raw_json_byte_exact<T>(case_name: &str, message: &str, expected: &RawValue, actual: &T)
where
    T: Serialize,
{
    let expected_bytes = compact_json_lexically(expected.get());
    let canonical_actual = serde_json::to_value(actual).expect("canonicalize actual JSON");
    let actual_bytes = serde_json::to_vec(&canonical_actual).expect("serialize actual bytes");
    assert_eq!(
        actual_bytes,
        expected_bytes,
        "golden case '{case_name}' {message}\n{}",
        byte_diff(&expected_bytes, &actual_bytes)
    );
}

fn compact_json_lexically(json: &str) -> Vec<u8> {
    let mut compact = Vec::with_capacity(json.len());
    let mut in_string = false;
    let mut escaped = false;
    for byte in json.bytes() {
        if in_string {
            compact.push(byte);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
            compact.push(byte);
        } else if !byte.is_ascii_whitespace() {
            compact.push(byte);
        }
    }
    compact
}

fn byte_diff(expected: &[u8], actual: &[u8]) -> String {
    let offset = expected
        .iter()
        .zip(actual)
        .position(|(expected, actual)| expected != actual)
        .unwrap_or_else(|| expected.len().min(actual.len()));
    let start = offset.saturating_sub(80);
    let expected_end = (offset + 80).min(expected.len());
    let actual_end = (offset + 80).min(actual.len());
    format!(
        "first differing byte: {offset}\n--- expected\n{}\n+++ actual\n{}",
        String::from_utf8_lossy(&expected[start..expected_end]),
        String::from_utf8_lossy(&actual[start..actual_end]),
    )
}

fn readable_diff<T>(expected: &T, actual: &T) -> String
where
    T: Serialize,
{
    let expected = serde_json::to_string_pretty(expected).expect("serialize expected for diff");
    let actual = serde_json::to_string_pretty(actual).expect("serialize actual for diff");
    let expected_lines: Vec<_> = expected.lines().collect();
    let actual_lines: Vec<_> = actual.lines().collect();
    let mut diff = String::from("--- expected\n+++ actual\n");

    for index in 0..expected_lines.len().max(actual_lines.len()) {
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
