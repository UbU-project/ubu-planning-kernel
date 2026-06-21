use std::collections::{BTreeMap, BTreeSet};

use crate::diagnostics::{Diagnostic, DiagnosticCode};
use crate::request::{DurationModel, PlanningRequest, TaskSpec, MAX_N_ROLLOUTS, MAX_ROLLOUT_TOP_K};
use crate::response::{PlanCandidate, ProbabilityQuality, RolloutDiagnostics};
use crate::scoring::{assign_roles, compare_candidates};

const LOAD_NORM_CAP: f64 = 0.95;
const ROBUSTNESS_PERCENTILE: f64 = 0.10;
const WILSON_Z: f64 = 1.959_963_984_540_054;
const JITTER_START: f64 = 1.0e-12;
const JITTER_MAX: f64 = 1.0e-8;

pub struct RolloutResult {
    pub candidates: Vec<PlanCandidate>,
    pub diagnostics: Vec<Diagnostic>,
}

struct CorrelationFactor {
    lower: Vec<Vec<f64>>,
    quality: ProbabilityQuality,
    warnings: Vec<String>,
}

/// Runs Stage 4 only over the Stage 3 top-K finalists while retaining the full
/// bounded Stage 3 candidate set in the response.
pub fn rollout_and_rerank(
    request: &PlanningRequest,
    mut candidates: Vec<PlanCandidate>,
) -> Result<RolloutResult, Diagnostic> {
    let n_rollouts = request.effective_n_rollouts();
    let top_k = request.effective_rollout_top_k().min(candidates.len());
    let stage_seed = request.rng_seed.wrapping_add(3);
    let mut diagnostics = Vec::new();

    if request.n_rollouts > MAX_N_ROLLOUTS {
        diagnostics.push(Diagnostic::new(
            DiagnosticCode::RolloutDegraded,
            format!(
                "n_rollouts {} exceeds the sample cap; using {MAX_N_ROLLOUTS}",
                request.n_rollouts
            ),
        ));
    }
    if request.top_k > MAX_ROLLOUT_TOP_K {
        diagnostics.push(Diagnostic::new(
            DiagnosticCode::RolloutDegraded,
            format!(
                "top_k {} exceeds the finalist cap; using {MAX_ROLLOUT_TOP_K}",
                request.top_k
            ),
        ));
    }

    if n_rollouts == 0 || top_k == 0 {
        return Ok(RolloutResult {
            candidates,
            diagnostics,
        });
    }

    let factor = correlation_factor(request)?;
    for warning in &factor.warnings {
        diagnostics.push(Diagnostic::new(
            DiagnosticCode::RolloutDegraded,
            warning.clone(),
        ));
    }
    if factor.quality != ProbabilityQuality::Full {
        diagnostics.push(Diagnostic::new(
            DiagnosticCode::RolloutDegraded,
            format!("rollout probability quality is {:?}", factor.quality),
        ));
    }

    for candidate in candidates.iter_mut().take(top_k) {
        rollout_candidate(request, candidate, n_rollouts, stage_seed, &factor);
    }
    for candidate in candidates.iter_mut().skip(top_k) {
        candidate.probability_summary.probability_quality = ProbabilityQuality::NotEstimated;
    }

    assign_roles(&mut candidates);
    let (finalists, non_finalists) = candidates.split_at_mut(top_k);
    // Keep the two cohorts distinct: rollout-grounded finalists always precede
    // C-1-scored non-finalists. Within each cohort, compare_candidates orders by
    // composite descending and uses candidate_id ascending as its final tiebreak.
    finalists.sort_by(compare_candidates);
    non_finalists.sort_by(compare_candidates);
    for (index, candidate) in candidates.iter_mut().enumerate() {
        candidate.rank = index + 1;
    }

    Ok(RolloutResult {
        candidates,
        diagnostics,
    })
}

fn rollout_candidate(
    request: &PlanningRequest,
    candidate: &mut PlanCandidate,
    n_rollouts: usize,
    stage_seed: u64,
    factor: &CorrelationFactor,
) {
    let substream_seed = mix64(stage_seed ^ stable_hash(candidate.candidate_id.as_bytes()));
    let mut rng = SplitMix64::new(substream_seed);
    let mut feasible_count = 0usize;
    let mut outcomes = Vec::with_capacity(n_rollouts);

    for _ in 0..n_rollouts {
        let independent: Vec<_> = (0..request.tasks().len())
            .map(|_| rng.standard_normal())
            .collect();
        let normals = multiply_lower(&factor.lower, &independent);
        let durations: Vec<_> = request
            .tasks()
            .iter()
            .zip(normals)
            .map(|(task, normal)| sample_duration(&task.duration, normal))
            .collect();
        let (feasible, outcome) = simulate(request, candidate, &durations);
        feasible_count += usize::from(feasible);
        outcomes.push(outcome);
    }

    outcomes.sort_by(f64::total_cmp);
    let percentile_index = ((n_rollouts - 1) as f64 * ROBUSTNESS_PERCENTILE).floor() as usize;
    let robustness = outcomes[percentile_index];
    let probability = feasible_count as f64 / n_rollouts as f64;
    let (low, high) = wilson_interval(feasible_count, n_rollouts);

    candidate.score_summary.robustness_score = robustness;
    // Probability and lower-tail outcome jointly ground the former C-1 robustness proxy.
    let rollout_grounded_robustness = (robustness + probability) / 2.0;
    candidate.score_summary.total_score =
        recompute_total(candidate, request, rollout_grounded_robustness);
    candidate.probability_summary.display_probability = Some(probability);
    // JSON has no representation for -infinity; use the smallest positive f64
    // as the deterministic serialized floor for log(0).
    candidate.probability_summary.log_probability = Some(probability.max(f64::MIN_POSITIVE).ln());
    candidate.probability_summary.probability_interval_low = Some(low);
    candidate.probability_summary.probability_interval_high = Some(high);
    candidate.probability_summary.probability_quality = factor.quality;
    candidate.rollout_diagnostics = Some(RolloutDiagnostics {
        n_rollouts,
        feasible_rollouts: feasible_count,
        feasibility_frequency: probability,
        robustness_percentile: ROBUSTNESS_PERCENTILE,
        stage_seed,
        probability_quality: factor.quality,
        warnings: factor.warnings.clone(),
    });
}

fn recompute_total(candidate: &PlanCandidate, request: &PlanningRequest, robustness: f64) -> f64 {
    let score = &candidate.score_summary;
    let policy = &request.scoring_policy;
    let weight_sum = policy.utility_weight
        + policy.robustness_weight
        + policy.affect_margin_weight
        + policy.schedule_diversity_weight;
    (score.utility_score * policy.utility_weight
        + robustness * policy.robustness_weight
        + score.affect_margin_score * policy.affect_margin_weight
        + score.schedule_diversity_score * policy.schedule_diversity_weight)
        / weight_sum
}

fn simulate(
    request: &PlanningRequest,
    candidate: &PlanCandidate,
    sampled_durations: &[f64],
) -> (bool, f64) {
    let task_indices: BTreeMap<_, _> = request
        .tasks()
        .iter()
        .enumerate()
        .map(|(index, task)| (task.id.as_str(), index))
        .collect();
    let mut actual_ends = BTreeMap::new();
    let mut previous_end = 0.0f64;
    let mut feasible = true;

    for step in &candidate.schedule.steps {
        let Some(&task_index) = task_indices.get(step.task_id.as_str()) else {
            return (false, 0.0);
        };
        let task = &request.tasks()[task_index];
        let dependency_end = task
            .depends_on
            .iter()
            .filter_map(|dependency| actual_ends.get(dependency.as_str()))
            .copied()
            .fold(0.0f64, f64::max);
        let actual_start = (step.start as f64).max(previous_end).max(dependency_end);
        if let Some(anchor) = &task.static_anchor {
            feasible &= actual_start == anchor.start as f64;
        }
        let actual_end = actual_start + sampled_durations[task_index];
        if let Some(window) = &task.window {
            feasible &= actual_start >= window.start as f64 && actual_end <= window.end as f64;
        }
        if let Some(window) = &request.time_window {
            feasible &= actual_start >= window.start as f64 && actual_end <= window.end as f64;
        }
        actual_ends.insert(task.id.as_str(), actual_end);
        previous_end = actual_end;
    }

    let outcome = request.time_window.as_ref().map_or(0.0, |window| {
        let width = window.end.saturating_sub(window.start).max(1) as f64;
        ((window.end as f64 - previous_end) / width).clamp(0.0, 1.0)
    });
    (feasible, if feasible { outcome } else { 0.0 })
}

/// Exact §3 shifted-log-normal transformation. Fixed durations are delta draws.
pub fn sample_duration(model: &DurationModel, standard_normal: f64) -> f64 {
    match model {
        DurationModel::Fixed { seconds } => *seconds as f64,
        DurationModel::ShiftedLognormalP95 {
            min_seconds,
            mode_seconds,
            p95_seconds,
        } => {
            let a = (*mode_seconds - *min_seconds) as f64;
            let b = (*p95_seconds - *min_seconds) as f64;
            let z95 = 1.644_853_626_951_472_2;
            let sigma = (-z95 + (z95 * z95 + 4.0 * (b / a).ln()).sqrt()) / 2.0;
            let mu = a.ln() + sigma * sigma;
            *min_seconds as f64 + (mu + sigma * standard_normal).exp()
        }
    }
}

fn correlation_factor(request: &PlanningRequest) -> Result<CorrelationFactor, Diagnostic> {
    let (matrix, warnings) = build_correlation_matrix(request.tasks())
        .map_err(|message| Diagnostic::new(DiagnosticCode::RolloutValidation, message))?;
    factor_matrix(
        &matrix,
        request.tasks().len(),
        request.strict_validation,
        warnings,
    )
}

fn factor_matrix(
    matrix: &[Vec<f64>],
    dimension: usize,
    strict_validation: bool,
    warnings: Vec<String>,
) -> Result<CorrelationFactor, Diagnostic> {
    match cholesky(matrix) {
        Ok(lower) => Ok(CorrelationFactor {
            lower,
            quality: ProbabilityQuality::Full,
            warnings,
        }),
        Err(_) => {
            let mut jitter = JITTER_START;
            while jitter <= JITTER_MAX {
                let mut jittered = matrix.to_vec();
                for (index, row) in jittered.iter_mut().enumerate() {
                    row[index] += jitter;
                }
                if let Ok(lower) = cholesky(&jittered) {
                    let mut warnings = warnings;
                    warnings.push(format!(
                        "correlation Cholesky required diagonal jitter {jitter:e}"
                    ));
                    return Ok(CorrelationFactor {
                        lower,
                        quality: ProbabilityQuality::DegradedNumericJitter,
                        warnings,
                    });
                }
                jitter *= 10.0;
            }
            if strict_validation {
                Err(Diagnostic::new(
                    DiagnosticCode::RolloutValidation,
                    "correlation matrix factorization failed after bounded diagonal jitter",
                ))
            } else {
                let mut warnings = warnings;
                warnings.push(
                    "correlation factorization failed; using independent duration samples"
                        .to_string(),
                );
                Ok(CorrelationFactor {
                    lower: identity(dimension),
                    quality: ProbabilityQuality::DegradedIndependence,
                    warnings,
                })
            }
        }
    }
}

/// Builds C = L L^T + diag(1 - row_norm(L)^2), then validates its entries.
/// The policy-bearing PSD factorization is performed immediately afterward by
/// `correlation_factor`. No projection or signed loading repair is used.
pub fn build_correlation_matrix(
    tasks: &[TaskSpec],
) -> Result<(Vec<Vec<f64>>, Vec<String>), String> {
    let mut group_names = BTreeSet::new();
    for task in tasks {
        let mut task_groups = BTreeSet::new();
        for group in &task.correlation_groups {
            if !group.strength.is_finite() || !(0.0..=1.0).contains(&group.strength) {
                return Err(format!(
                    "task '{}': correlation strength must be finite and in [0,1]",
                    task.id
                ));
            }
            if !task_groups.insert(group.group.as_str()) {
                return Err(format!(
                    "task '{}': duplicate correlation group '{}'",
                    task.id, group.group
                ));
            }
            group_names.insert(group.group.clone());
        }
    }
    let group_indices: BTreeMap<_, _> = group_names
        .into_iter()
        .enumerate()
        .map(|(index, group)| (group, index))
        .collect();
    let mut loadings = vec![vec![0.0; group_indices.len()]; tasks.len()];
    let mut warnings = Vec::new();
    for (task_index, task) in tasks.iter().enumerate() {
        for group in &task.correlation_groups {
            loadings[task_index][group_indices[&group.group]] = group.strength;
        }
        let norm_squared: f64 = loadings[task_index].iter().map(|value| value * value).sum();
        if norm_squared > LOAD_NORM_CAP * LOAD_NORM_CAP {
            let scale = LOAD_NORM_CAP / norm_squared.sqrt();
            for loading in &mut loadings[task_index] {
                *loading *= scale;
            }
            warnings.push(format!(
                "task '{}' correlation loading norm exceeded 0.95 and was scaled to 0.95",
                task.id
            ));
        }
    }
    let mut matrix = vec![vec![0.0; tasks.len()]; tasks.len()];
    for i in 0..tasks.len() {
        for j in 0..tasks.len() {
            matrix[i][j] = loadings[i]
                .iter()
                .zip(&loadings[j])
                .map(|(left, right)| left * right)
                .sum();
        }
        let row_norm: f64 = loadings[i].iter().map(|value| value * value).sum();
        matrix[i][i] += 1.0 - row_norm;
    }
    validate_correlation_matrix(&matrix)?;
    Ok((matrix, warnings))
}

fn validate_correlation_matrix(matrix: &[Vec<f64>]) -> Result<(), String> {
    let n = matrix.len();
    if matrix.iter().any(|row| row.len() != n) {
        return Err("correlation matrix must be square".to_string());
    }
    for (i, row) in matrix.iter().enumerate() {
        for (j, &value) in row.iter().enumerate() {
            if !value.is_finite() {
                return Err("correlation matrix values must be finite".to_string());
            }
            if value < 0.0 {
                return Err("correlation matrix values must be non-negative".to_string());
            }
            if (value - matrix[j][i]).abs() > 1.0e-12 {
                return Err("correlation matrix must be symmetric".to_string());
            }
        }
        if (matrix[i][i] - 1.0).abs() > 1.0e-12 {
            return Err("correlation matrix diagonal must be one".to_string());
        }
    }
    Ok(())
}

fn cholesky(matrix: &[Vec<f64>]) -> Result<Vec<Vec<f64>>, &'static str> {
    let n = matrix.len();
    let mut lower = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..=i {
            let residual = matrix[i][j] - (0..j).map(|k| lower[i][k] * lower[j][k]).sum::<f64>();
            if i == j {
                if !residual.is_finite() || residual <= 0.0 {
                    return Err("non-positive pivot");
                }
                lower[i][j] = residual.sqrt();
            } else {
                lower[i][j] = residual / lower[j][j];
            }
        }
    }
    Ok(lower)
}

fn identity(n: usize) -> Vec<Vec<f64>> {
    (0..n)
        .map(|i| (0..n).map(|j| f64::from(i == j)).collect())
        .collect()
}

fn multiply_lower(lower: &[Vec<f64>], values: &[f64]) -> Vec<f64> {
    lower
        .iter()
        .enumerate()
        .map(|(i, row)| (0..=i).map(|j| row[j] * values[j]).sum())
        .collect()
}

pub fn wilson_interval(feasible: usize, total: usize) -> (f64, f64) {
    let n = total as f64;
    let p = feasible as f64 / n;
    let z2 = WILSON_Z * WILSON_Z;
    let denominator = 1.0 + z2 / n;
    let center = (p + z2 / (2.0 * n)) / denominator;
    let half = WILSON_Z / denominator * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
    (
        (center - half).clamp(0.0, 1.0),
        (center + half).clamp(0.0, 1.0),
    )
}

struct SplitMix64 {
    state: u64,
    spare_normal: Option<f64>,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self {
            state: seed,
            spare_normal: None,
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        mix64(self.state)
    }

    fn open_unit(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) * (1.0 / ((1u64 << 53) as f64))
    }

    fn standard_normal(&mut self) -> f64 {
        if let Some(value) = self.spare_normal.take() {
            return value;
        }
        let radius = (-2.0 * self.open_unit().ln()).sqrt();
        let angle = std::f64::consts::TAU * self.open_unit();
        self.spare_normal = Some(radius * angle.sin());
        radius * angle.cos()
    }
}

fn mix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn stable_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

// Phase 1 (now): external_event_assumptions defaults to empty and the rollout treats it as
// no external events.
// TODO(P9/external-events, deferred): external_event_assumptions is a §2 list of structured
// stochastic or fixed external events. Each would enter the rollout as an additional stochastic
// factor affecting per-rollout feasibility: a fixed event contributes a deterministic
// availability/blocking time; a stochastic event contributes a sampled occurrence time or an
// in-window probability of occurring. Feasibility frequency must incorporate them (a rollout is
// infeasible if a required external dependency is unavailable when needed or an external blocker
// occurs in-window). Wire this into the same per-finalist substream so determinism holds.
// Reconcile with the skeleton-failure `blocked_external_event` class so external events are
// handled consistently at skeleton time and rollout time (skeleton may mark a hard block;
// rollout estimates the probability of a soft block). Tie occurrence-time sampling to the same
// correlation machinery if events are correlated with Task durations (deferred). Reference §5
// Stage 4 inputs and the §2 `external_event_assumptions` shape.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::CorrelationGroup;

    #[test]
    fn fixed_is_delta_and_shifted_parameters_hit_mode_and_p95() {
        assert_eq!(
            sample_duration(&DurationModel::Fixed { seconds: 7 }, -20.0),
            7.0
        );
        let model = DurationModel::ShiftedLognormalP95 {
            min_seconds: 2,
            mode_seconds: 7,
            p95_seconds: 20,
        };
        let at_p95 = sample_duration(&model, 1.644_853_626_951_472_2);
        assert!((at_p95 - 20.0).abs() < 1.0e-10);
    }

    #[test]
    fn correlation_is_unit_diagonal_symmetric_and_caps_loadings() {
        let mut task = TaskSpec::new("a".to_string(), DurationModel::Fixed { seconds: 1 }).unwrap();
        task.correlation_groups = vec![
            CorrelationGroup {
                group: "x".to_string(),
                strength: 1.0,
            },
            CorrelationGroup {
                group: "y".to_string(),
                strength: 1.0,
            },
        ];
        let (matrix, warnings) = build_correlation_matrix(&[task.clone(), task]).unwrap();
        assert_eq!(matrix[0][0], 1.0);
        assert_eq!(matrix[0][1], matrix[1][0]);
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn wilson_has_width_at_extremes() {
        let (zero_low, zero_high) = wilson_interval(0, 1000);
        let (one_low, one_high) = wilson_interval(1000, 1000);
        assert!(zero_low < 1.0e-15);
        assert!(zero_high > zero_low);
        assert!(one_high > one_low);
        assert!(one_high <= 1.0);
    }

    #[test]
    fn factorization_policy_uses_jitter_then_strict_or_independent_fallback() {
        let full = factor_matrix(&identity(2), 2, true, Vec::new()).unwrap();
        assert_eq!(full.quality, ProbabilityQuality::Full);
        assert_eq!(serde_json::to_string(&full.quality).unwrap(), "\"full\"");

        let singular = vec![vec![1.0, 1.0], vec![1.0, 1.0]];
        let jittered = factor_matrix(&singular, 2, true, Vec::new()).unwrap();
        assert_eq!(jittered.quality, ProbabilityQuality::DegradedNumericJitter);

        let indefinite = vec![vec![1.0, 2.0], vec![2.0, 1.0]];
        assert!(factor_matrix(&indefinite, 2, true, Vec::new()).is_err());
        let degraded = factor_matrix(&indefinite, 2, false, Vec::new()).unwrap();
        assert_eq!(degraded.quality, ProbabilityQuality::DegradedIndependence);
        assert_eq!(degraded.lower, identity(2));
    }
}
