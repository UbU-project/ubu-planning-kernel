//! Deterministic accounting over sampled boundary outcomes (UBU-D0285).
use crate::rollout::wilson_interval;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const LATENESS_BUCKET_SECONDS: u64 = 60;
pub const LATENESS_QUANTIZATION_RULE: &str = "lateness_seconds_ceil_60";
pub fn lateness_bucket(seconds: f64) -> u64 {
    if seconds <= 0.0 {
        0
    } else {
        (seconds / LATENESS_BUCKET_SECONDS as f64).ceil() as u64
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageScope {
    ReactiveHorizon,
    FullWindow,
    RepairScope,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OutcomeState {
    pub boundary_index: usize,
    pub boundary_start: u64,
    pub boundary_task_ref: String,
    pub completed: BTreeSet<String>,
    pub lateness_bucket: u64,
}
impl OutcomeState {
    /// FNV-1a over explicit little-endian integers and length-prefixed UTF-8.
    pub fn digest(&self) -> String {
        fn fold(hash: &mut u64, bytes: &[u8]) {
            for byte in bytes {
                *hash = (*hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        fn number(hash: &mut u64, value: u64) {
            fold(hash, &value.to_le_bytes());
        }
        fn string(hash: &mut u64, value: &str) {
            number(hash, value.len() as u64);
            fold(hash, value.as_bytes());
        }
        let mut hash = 0xcbf2_9ce4_8422_2325;
        number(&mut hash, self.boundary_index as u64);
        number(&mut hash, self.boundary_start);
        string(&mut hash, &self.boundary_task_ref);
        number(&mut hash, self.completed.len() as u64);
        for id in &self.completed {
            string(&mut hash, id);
        }
        number(&mut hash, self.lateness_bucket);
        format!("outcome-{hash:016x}")
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoundarySummary {
    pub boundary_index: usize,
    pub boundary_start: u64,
    pub boundary_task_ref: String,
    pub covered_outcome_count: usize,
    pub uncovered_outcome_count: usize,
    pub uncovered_mass: f64,
    pub heaviest_uncovered_digest: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutcomeContinuationSummary {
    pub covered_outcome_count: usize,
    pub uncovered_outcome_count: usize,
    pub quantization_rule: String,
    pub budget_limited: bool,
    pub boundaries: Vec<BoundarySummary>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageConfidence {
    pub method: String,
    pub n_rollouts: usize,
    pub confidence_level: f64,
    pub lower: f64,
    pub upper: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageSummary {
    pub coverage_scope: CoverageScope,
    pub coverage_estimate: f64,
    pub uncovered_mass_estimate: f64,
    pub coverage_threshold_used: f64,
    pub coverage_below_threshold: bool,
    pub coverage_confidence: CoverageConfidence,
    pub outcome_continuation_summary: OutcomeContinuationSummary,
}
#[derive(Default)]
pub struct CoverageCounter {
    total: usize,
    continued: usize,
    // A zero failure count certifies every sampled visit to this merged state.
    states: BTreeMap<OutcomeState, usize>,
}
impl CoverageCounter {
    pub fn observe(&mut self, states: impl IntoIterator<Item = OutcomeState>, continued: bool) {
        self.total += 1;
        self.continued += usize::from(continued);
        for state in states.into_iter().collect::<BTreeSet<_>>() {
            *self.states.entry(state).or_default() += usize::from(!continued);
        }
    }
    pub fn summarize(
        &self,
        scope: CoverageScope,
        threshold: f64,
        budget_limited: bool,
    ) -> CoverageSummary {
        let mut boundaries: BTreeMap<(usize, u64, String), (BoundarySummary, usize, usize)> =
            BTreeMap::new();
        let mut covered = 0;
        let mut uncovered = 0;
        for (state, &failures) in &self.states {
            let (boundary, heaviest, failed_visits) = boundaries
                .entry((
                    state.boundary_index,
                    state.boundary_start,
                    state.boundary_task_ref.clone(),
                ))
                .or_insert_with(|| {
                    (
                        BoundarySummary {
                            boundary_index: state.boundary_index,
                            boundary_start: state.boundary_start,
                            boundary_task_ref: state.boundary_task_ref.clone(),
                            covered_outcome_count: 0,
                            uncovered_outcome_count: 0,
                            uncovered_mass: 0.0,
                            heaviest_uncovered_digest: None,
                        },
                        0,
                        0,
                    )
                });
            if failures == 0 {
                covered += 1;
                boundary.covered_outcome_count += 1;
            } else {
                uncovered += 1;
                boundary.uncovered_outcome_count += 1;
                *failed_visits += failures;
                let digest = state.digest();
                if failures > *heaviest
                    || (failures == *heaviest
                        && boundary
                            .heaviest_uncovered_digest
                            .as_ref()
                            .is_none_or(|old| &digest < old))
                {
                    *heaviest = failures;
                    boundary.heaviest_uncovered_digest = Some(digest);
                }
            }
        }
        let estimate = if self.total == 0 {
            0.0
        } else {
            self.continued as f64 / self.total as f64
        };
        let (lower, upper) = if self.total == 0 {
            (0.0, 1.0)
        } else {
            wilson_interval(self.continued, self.total)
        };
        CoverageSummary {
            coverage_scope: scope,
            coverage_estimate: estimate,
            uncovered_mass_estimate: 1.0 - estimate,
            coverage_threshold_used: threshold,
            coverage_below_threshold: estimate < threshold,
            coverage_confidence: CoverageConfidence {
                method: "wilson".into(),
                n_rollouts: self.total,
                confidence_level: 0.95,
                lower,
                upper,
            },
            outcome_continuation_summary: OutcomeContinuationSummary {
                covered_outcome_count: covered,
                uncovered_outcome_count: uncovered,
                quantization_rule: LATENESS_QUANTIZATION_RULE.into(),
                budget_limited,
                boundaries: boundaries
                    .into_values()
                    .map(|(mut summary, _, failures)| {
                        summary.uncovered_mass = failures as f64 / self.total as f64;
                        summary
                    })
                    .collect(),
            },
        }
    }
}
