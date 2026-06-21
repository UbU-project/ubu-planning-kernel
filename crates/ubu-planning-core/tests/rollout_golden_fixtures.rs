use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use ubu_planning_core::{PlanningRequest, ProbabilityQuality};
use ubu_planning_cpu::CpuStrategy;

const GOLDEN_CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/planning/golden/rollout-c2.json"
);
const POST_P9_KERNEL_REVISION: &str = "3fd25a93d725300212d15da8c00e113ccd8a648b";

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
    expected: Box<RawValue>,
}

#[derive(Debug, Serialize, PartialEq)]
struct RolloutProjection {
    candidate_order: Vec<String>,
    pre_rollout_default: ProxySummary,
    rollout_summaries: Vec<RolloutSummary>,
}

#[derive(Debug, Serialize, PartialEq)]
struct ProxySummary {
    candidate_id: String,
    robustness_score: f64,
    total_score: f64,
}

#[derive(Debug, Serialize, PartialEq)]
struct RolloutSummary {
    candidate_id: String,
    rank: usize,
    robustness_score: f64,
    total_score: f64,
    display_probability: f64,
    probability_interval_low: f64,
    probability_interval_high: f64,
    probability_quality: ProbabilityQuality,
    feasible_rollouts: usize,
    feasibility_frequency: f64,
    stage_seed: u64,
}

#[test]
fn rollout_goldens_match_byte_exact_fixed_seed_projections() {
    let input = fs::read_to_string(GOLDEN_CORPUS).expect("read rollout golden corpus");
    let corpus: GoldenCorpus = serde_json::from_str(&input).expect("parse rollout golden corpus");

    assert_eq!(
        corpus.schema_version,
        "planning-golden-corpus/rollout-c2.v1"
    );
    assert!(
        corpus.human_reviewed_then_frozen,
        "rollout corpus must be marked human-reviewed-then-frozen"
    );
    assert_eq!(corpus.kernel_revision, POST_P9_KERNEL_REVISION);
    assert_required_coverage(&corpus.cases);

    let mut frequencies = BTreeMap::new();
    for case in corpus.cases {
        let first = ubu_planning_core::plan(case.request.clone(), &CpuStrategy);
        let second = ubu_planning_core::plan(case.request.clone(), &CpuStrategy);
        assert_byte_exact(
            &case.name,
            "full response changed between identical fixed-seed runs",
            &first,
            &second,
        );

        let mut pre_rollout_request = case.request.clone();
        pre_rollout_request.n_rollouts = 0;
        let pre_rollout = ubu_planning_core::plan(pre_rollout_request, &CpuStrategy);
        let actual = project(&first, &pre_rollout);
        assert_raw_json_byte_exact(
            &case.name,
            "rollout projection differs from frozen golden",
            &case.expected,
            &actual,
        );

        if case.request.n_rollouts == 0 {
            assert!(first.plan_candidates.iter().all(|candidate| {
                candidate.probability_summary.probability_quality
                    == ProbabilityQuality::NotEstimated
                    && candidate.probability_summary.display_probability.is_none()
                    && candidate.rollout_diagnostics.is_none()
            }));
        } else {
            assert!(actual
                .rollout_summaries
                .iter()
                .all(|summary| summary.stage_seed == case.request.rng_seed.wrapping_add(3)));
        }

        if case
            .covers
            .iter()
            .any(|cover| cover == "rerank_changes_default")
        {
            assert_ne!(
                actual.candidate_order.first(),
                Some(&actual.pre_rollout_default.candidate_id),
                "golden case '{}' must change the rank-1 default",
                case.name
            );
        }
        if case.name == "independent-durations" || case.name == "shared-correlated-durations" {
            frequencies.insert(
                case.name.clone(),
                actual.rollout_summaries[0].feasibility_frequency,
            );
        }
    }

    assert_ne!(
        frequencies["independent-durations"], frequencies["shared-correlated-durations"],
        "shared correlation must change feasibility frequency from the independent baseline"
    );
}

fn project(
    response: &ubu_planning_core::PlanningResponse,
    pre_rollout: &ubu_planning_core::PlanningResponse,
) -> RolloutProjection {
    let pre_rollout_default = pre_rollout
        .plan_candidates
        .first()
        .expect("pre-rollout response has a default");
    RolloutProjection {
        candidate_order: response
            .plan_candidates
            .iter()
            .map(|candidate| candidate.candidate_id.clone())
            .collect(),
        pre_rollout_default: ProxySummary {
            candidate_id: pre_rollout_default.candidate_id.clone(),
            robustness_score: pre_rollout_default.score_summary.robustness_score,
            total_score: pre_rollout_default.score_summary.total_score,
        },
        rollout_summaries: response
            .plan_candidates
            .iter()
            .filter_map(|candidate| {
                let diagnostics = candidate.rollout_diagnostics.as_ref()?;
                Some(RolloutSummary {
                    candidate_id: candidate.candidate_id.clone(),
                    rank: candidate.rank,
                    robustness_score: candidate.score_summary.robustness_score,
                    total_score: candidate.score_summary.total_score,
                    display_probability: candidate
                        .probability_summary
                        .display_probability
                        .expect("rolled candidate has display probability"),
                    probability_interval_low: candidate
                        .probability_summary
                        .probability_interval_low
                        .expect("rolled candidate has Wilson lower bound"),
                    probability_interval_high: candidate
                        .probability_summary
                        .probability_interval_high
                        .expect("rolled candidate has Wilson upper bound"),
                    probability_quality: candidate.probability_summary.probability_quality,
                    feasible_rollouts: diagnostics.feasible_rollouts,
                    feasibility_frequency: diagnostics.feasibility_frequency,
                    stage_seed: diagnostics.stage_seed,
                })
            })
            .collect(),
    }
}

fn assert_required_coverage(cases: &[GoldenCase]) {
    let covered: BTreeSet<_> = cases
        .iter()
        .flat_map(|case| case.covers.iter().map(String::as_str))
        .collect();
    let required = [
        "fully_feasible",
        "wilson_nonzero_width",
        "fragile",
        "low_probability",
        "independent_baseline",
        "shared_correlation_group",
        "correlation_changes_frequency",
        "rerank_changes_default",
        "not_estimated",
        "c1_proxy",
        "zero_rollouts",
        "p10_robustness",
        "stage_seed_plus_three",
    ];
    let missing: Vec<_> = required
        .iter()
        .copied()
        .filter(|cover| !covered.contains(cover))
        .collect();
    assert!(
        missing.is_empty(),
        "rollout corpus is missing required coverage tags: {missing:?}"
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
    let actual_bytes = serde_json::to_vec(actual).expect("serialize actual bytes");
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
    let first_difference = expected
        .iter()
        .zip(actual)
        .position(|(left, right)| left != right)
        .unwrap_or_else(|| expected.len().min(actual.len()));
    let start = first_difference.saturating_sub(80);
    let expected_end = (first_difference + 160).min(expected.len());
    let actual_end = (first_difference + 160).min(actual.len());
    format!(
        "first byte difference at offset {first_difference}\n--- expected\n{}\n+++ actual\n{}",
        String::from_utf8_lossy(&expected[start..expected_end]),
        String::from_utf8_lossy(&actual[start..actual_end])
    )
}

fn readable_diff<T: Serialize>(expected: &T, actual: &T) -> String {
    let expected = serde_json::to_string_pretty(expected).expect("pretty-print expected");
    let actual = serde_json::to_string_pretty(actual).expect("pretty-print actual");
    format!("--- expected\n{expected}\n+++ actual\n{actual}")
}
