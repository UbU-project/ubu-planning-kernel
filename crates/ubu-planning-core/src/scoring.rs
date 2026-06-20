use std::collections::BTreeMap;

use crate::explanations::explain_plan;
use crate::legitimization::FullLegitimization;
use crate::request::PlanningRequest;
use crate::response::{
    CandidateRole, FeasibilitySummary, Plan, PlanCandidate, ProbabilitySummary, ScoreSummary,
    SemiLegitimizationSummary,
};

pub struct ScoringInput {
    pub schedule: Plan,
    pub full_legitimization: FullLegitimization,
    pub semi_legitimization: SemiLegitimizationSummary,
}

pub fn score_and_rank(request: &PlanningRequest, inputs: Vec<ScoringInput>) -> Vec<PlanCandidate> {
    if inputs.is_empty() {
        return Vec::new();
    }

    let utility_scores: Vec<_> = inputs
        .iter()
        .map(|input| utility_score(&input.schedule, request))
        .collect();
    let utility_reference = best_index(&utility_scores, &inputs);
    let reference_schedule = inputs[utility_reference].schedule.clone();

    let mut candidates: Vec<_> = inputs
        .into_iter()
        .zip(utility_scores)
        .map(|(input, utility_score)| {
            let robustness_score = robustness_score(&input.schedule, request);
            let affect_margin_score = input
                .full_legitimization
                .report
                .affect_margin
                .map_or(1.0, |margin| ((margin + 1.0) / 2.0).clamp(0.0, 1.0));
            let schedule_diversity_score =
                schedule_distance(&input.schedule, &reference_schedule, request);
            let total_score = weighted_total(
                request,
                utility_score,
                robustness_score,
                affect_margin_score,
                schedule_diversity_score,
            );
            let minimum_affect_score = input
                .full_legitimization
                .report
                .dimensions
                .values()
                .map(|dimension| dimension.satisfaction)
                .min_by(f64::total_cmp);
            let explanation_fragments = explain_plan(&input.schedule).fragments;

            PlanCandidate {
                candidate_id: input.schedule.plan_id.clone(),
                rank: 0,
                candidate_role: CandidateRole::Other,
                schedule: input.schedule,
                score_summary: ScoreSummary {
                    utility_score,
                    robustness_score,
                    affect_margin_score,
                    schedule_diversity_score,
                    total_score,
                },
                feasibility_summary: FeasibilitySummary {
                    hard_constraints_assumed_satisfied_by_engine: true,
                    affect_feasible: input.full_legitimization.report.affect_feasible,
                    minimum_affect_score,
                    violated_affect_dimensions: input
                        .full_legitimization
                        .report
                        .violated_dimensions,
                },
                semi_legitimization_summary: input.semi_legitimization,
                // C-2 owns probability estimation. Keep the contract object present and empty.
                probability_summary: ProbabilitySummary::default(),
                rollout_diagnostics: None,
                explanation_fragments,
                validation_hints: Vec::new(),
            }
        })
        .collect();

    assign_roles(&mut candidates);
    candidates.sort_by(compare_candidates);
    for (index, candidate) in candidates.iter_mut().enumerate() {
        candidate.rank = index + 1;
    }
    candidates
}

fn utility_score(schedule: &Plan, request: &PlanningRequest) -> f64 {
    let Some(window) = &request.time_window else {
        return 0.0;
    };
    let window_length = window.end.saturating_sub(window.start).max(1) as f64;
    let task_by_id: BTreeMap<_, _> = request
        .tasks()
        .iter()
        .map(|task| (task.id.as_str(), task))
        .collect();
    let total_weight: f64 = task_by_id
        .values()
        .map(|task| task.value * task.priority)
        .sum();
    if total_weight <= 0.0 {
        return 0.0;
    }

    let utility: f64 = schedule
        .steps
        .iter()
        .filter_map(|step| {
            task_by_id
                .get(step.task_id.as_str())
                .map(|task| (step, task))
        })
        .map(|(step, task)| {
            let completion = step.end.saturating_sub(window.start) as f64 / window_length;
            task.value * task.priority * (1.0 - completion.clamp(0.0, 1.0))
        })
        .sum();
    (utility / total_weight).clamp(0.0, 1.0)
}

fn robustness_score(schedule: &Plan, request: &PlanningRequest) -> f64 {
    let Some(window) = &request.time_window else {
        return 0.0;
    };
    let window_length = window.end.saturating_sub(window.start).max(1) as f64;
    let task_by_id: BTreeMap<_, _> = request
        .tasks()
        .iter()
        .map(|task| (task.id.as_str(), task))
        .collect();
    let step_by_id: BTreeMap<_, _> = schedule
        .steps
        .iter()
        .map(|step| (step.task_id.as_str(), step))
        .collect();
    let spread_resilience = schedule
        .steps
        .iter()
        .filter_map(|step| task_by_id.get(step.task_id.as_str()))
        .map(|task| 1.0 - task.duration.relative_spread())
        .sum::<f64>()
        / schedule.steps.len().max(1) as f64;
    let dependency_slack = request
        .tasks()
        .iter()
        .flat_map(|task| {
            task.depends_on.iter().filter_map(|dependency| {
                let dependency = step_by_id.get(dependency.as_str())?;
                let task = step_by_id.get(task.id.as_str())?;
                Some(task.start.saturating_sub(dependency.end) as f64 / window_length)
            })
        })
        .sum::<f64>();
    let edge_count = request.dependency_edges().len();
    let average_dependency_slack = if edge_count == 0 {
        schedule
            .steps
            .iter()
            .map(|step| window.end.saturating_sub(step.end) as f64 / window_length)
            .sum::<f64>()
            / schedule.steps.len().max(1) as f64
    } else {
        dependency_slack / edge_count as f64
    };

    (0.7 * spread_resilience + 0.3 * average_dependency_slack.clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

fn schedule_distance(left: &Plan, right: &Plan, request: &PlanningRequest) -> f64 {
    let Some(window) = &request.time_window else {
        return 0.0;
    };
    let window_length = window.end.saturating_sub(window.start).max(1) as f64;
    let right_starts: BTreeMap<_, _> = right
        .steps
        .iter()
        .map(|step| (step.task_id.as_str(), step.start))
        .collect();
    let differences: Vec<_> = left
        .steps
        .iter()
        .filter_map(|step| {
            right_starts
                .get(step.task_id.as_str())
                .map(|right_start| step.start.abs_diff(*right_start) as f64)
        })
        .collect();
    if differences.is_empty() {
        return 0.0;
    }

    // Phase 1: normalized placement distance from the rank-1-by-utility candidate.
    // TODO(C-2/diversity): replace this proxy with a set-level metric over the whole
    // candidate set: mean pairwise schedule distance combining (a) per-Task start-time
    // dispersion, (b) ordering/permutation differences on the Task sequence (a
    // Kendall-tau-like distance), and (c) slack-distribution differences; normalize
    // to [0,1]. Use it both for schedule_diversity_score and to select a
    // maximally-diverse finalist subset via greedy max-min diversity. Tie this to
    // scoring_policy.schedule_diversity_weight, §5 Stage 3's schedule-diversity
    // policy, and DESIGN §16.3.1 multi-option comparison. Account for the cap-16
    // candidate set; in C-2 this diverse subset should become the rollout finalists.
    (differences.iter().sum::<f64>() / differences.len() as f64 / window_length).clamp(0.0, 1.0)
}

fn weighted_total(
    request: &PlanningRequest,
    utility: f64,
    robustness: f64,
    affect_margin: f64,
    diversity: f64,
) -> f64 {
    let policy = &request.scoring_policy;
    let weight_sum = policy.utility_weight
        + policy.robustness_weight
        + policy.affect_margin_weight
        + policy.schedule_diversity_weight;
    if weight_sum <= 0.0 {
        return 0.0;
    }
    (utility * policy.utility_weight
        + robustness * policy.robustness_weight
        + affect_margin * policy.affect_margin_weight
        + diversity * policy.schedule_diversity_weight)
        / weight_sum
}

pub(crate) fn assign_roles(candidates: &mut [PlanCandidate]) {
    let utility = metric_winner(candidates, |candidate| {
        candidate.score_summary.utility_score
    });
    let robustness = metric_winner(candidates, |candidate| {
        candidate.score_summary.robustness_score
    });
    let diversity = metric_winner(candidates, |candidate| {
        candidate.score_summary.schedule_diversity_score
    });
    for (index, candidate) in candidates.iter_mut().enumerate() {
        candidate.candidate_role = if index == utility {
            CandidateRole::HighestUtility
        } else if index == robustness {
            CandidateRole::MostRobust
        } else if index == diversity {
            CandidateRole::MostScheduleDiverse
        } else {
            CandidateRole::Other
        };
    }
}

fn metric_winner(candidates: &[PlanCandidate], metric: impl Fn(&PlanCandidate) -> f64) -> usize {
    candidates
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| {
            metric(left)
                .total_cmp(&metric(right))
                .then_with(|| right.candidate_id.cmp(&left.candidate_id))
        })
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn best_index(scores: &[f64], inputs: &[ScoringInput]) -> usize {
    scores
        .iter()
        .enumerate()
        .max_by(|(left_index, left), (right_index, right)| {
            left.total_cmp(right).then_with(|| {
                inputs[*right_index]
                    .schedule
                    .plan_id
                    .cmp(&inputs[*left_index].schedule.plan_id)
            })
        })
        .map(|(index, _)| index)
        .unwrap_or(0)
}

/// Composite descending order. Component scores break equal composites, then
/// candidate_id ascending is the final deterministic tiebreak.
pub fn compare_candidates(left: &PlanCandidate, right: &PlanCandidate) -> std::cmp::Ordering {
    right
        .score_summary
        .total_score
        .total_cmp(&left.score_summary.total_score)
        .then_with(|| {
            right
                .score_summary
                .utility_score
                .total_cmp(&left.score_summary.utility_score)
        })
        .then_with(|| {
            right
                .score_summary
                .robustness_score
                .total_cmp(&left.score_summary.robustness_score)
        })
        .then_with(|| {
            right
                .score_summary
                .affect_margin_score
                .total_cmp(&left.score_summary.affect_margin_score)
        })
        .then_with(|| {
            right
                .score_summary
                .schedule_diversity_score
                .total_cmp(&left.score_summary.schedule_diversity_score)
        })
        .then_with(|| left.candidate_id.cmp(&right.candidate_id))
}
