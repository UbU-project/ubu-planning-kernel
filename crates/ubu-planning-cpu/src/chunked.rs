//! Deterministic bounded beam over free chunks between fixed placements.
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use ubu_planning_core::{
    Plan, PlanStatus, PlanStep, PlanningRequest, SkeletonFailureDiagnostic, TaskSpec, TimeWindow,
};

use crate::skeleton::{affix_fixed, plan_id};

pub const DEFAULT_ALTERNATIVES_PER_CHUNK: usize = 3;
pub const DEFAULT_BEAM_WIDTH: usize = 16;
pub const MAX_SWEEP_CANDIDATES: usize = 16;

#[derive(Debug, Clone, Copy)]
pub struct ChunkedSweepStrategy {
    pub alternatives_per_chunk: usize,
    pub beam_width: usize,
}

impl Default for ChunkedSweepStrategy {
    fn default() -> Self {
        Self {
            alternatives_per_chunk: DEFAULT_ALTERNATIVES_PER_CHUNK,
            beam_width: DEFAULT_BEAM_WIDTH,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    ValueFirst,
    MostConstrainedFirst,
    ValueDensity,
}
pub const FILL_RULES: [FillRule; 3] = [
    FillRule::ValueFirst,
    FillRule::MostConstrainedFirst,
    FillRule::ValueDensity,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub start: u64,
    pub end: u64,
}

/// Clip, sort, and union fixed intervals, retaining only nonempty free gaps.
pub fn partition(plan_window: &TimeWindow, fixed: &[(u64, u64)]) -> Vec<Chunk> {
    let mut intervals: Vec<_> = fixed
        .iter()
        .map(|&(start, end)| (start.max(plan_window.start), end.min(plan_window.end)))
        .filter(|(start, end)| start < end)
        .collect();
    intervals.sort_unstable();
    let mut cursor = plan_window.start;
    let mut chunks = Vec::new();
    for (start, end) in intervals {
        if cursor < start {
            chunks.push(Chunk {
                start: cursor,
                end: start,
            });
        }
        cursor = cursor.max(end);
    }
    if cursor < plan_window.end {
        chunks.push(Chunk {
            start: cursor,
            end: plan_window.end,
        });
    }
    chunks
}

struct Unit<'a> {
    task: &'a TaskSpec,
    duration: u64,
    weight: f64,
    release: u64,
    deadline: u64,
    eligible: Vec<usize>,
}

#[derive(Clone)]
struct Branch {
    remaining: BTreeSet<String>,
    placed: BTreeMap<String, PlanStep>,
    utility: f64,
}

type PlacementKey = Vec<(u64, u64, String)>;
fn placement_key<'a>(steps: impl Iterator<Item = &'a PlanStep>) -> PlacementKey {
    let mut key: Vec<_> = steps
        .map(|step| (step.start, step.end, step.task_id.clone()))
        .collect();
    key.sort();
    key
}

fn utility(unit: &Unit<'_>, end: u64, window: &TimeWindow) -> f64 {
    let completion = end.saturating_sub(window.start) as f64
        / window.end.saturating_sub(window.start).max(1) as f64;
    unit.weight * (1.0 - completion.clamp(0.0, 1.0))
}

fn compare_units(left: &Unit<'_>, right: &Unit<'_>, index: usize, rule: FillRule) -> Ordering {
    let forced = |unit: &Unit<'_>| unit.eligible.last() == Some(&index);
    let count = |unit: &Unit<'_>| unit.eligible.iter().filter(|&&i| i >= index).count();
    forced(right)
        .cmp(&forced(left))
        .then_with(|| match rule {
            FillRule::ValueFirst => right
                .weight
                .total_cmp(&left.weight)
                .then(left.deadline.cmp(&right.deadline)),
            FillRule::MostConstrainedFirst => count(left)
                .cmp(&count(right))
                .then(left.deadline.cmp(&right.deadline))
                .then_with(|| right.weight.total_cmp(&left.weight)),
            FillRule::ValueDensity => (right.weight / right.duration as f64)
                .total_cmp(&(left.weight / left.duration as f64))
                .then(left.deadline.cmp(&right.deadline)),
        })
        .then(left.task.id.cmp(&right.task.id))
}

fn fill(
    branch: &Branch,
    units: &BTreeMap<String, Unit<'_>>,
    fixed: &BTreeMap<String, PlanStep>,
    chunk: &Chunk,
    index: usize,
    rule: FillRule,
    window: &TimeWindow,
) -> Result<Branch, String> {
    let mut child = branch.clone();
    let mut failed = BTreeSet::new();
    loop {
        let ready = child
            .remaining
            .iter()
            .filter(|id| {
                let unit = &units[*id];
                unit.eligible.contains(&index)
                    && !failed.contains(*id)
                    && unit
                        .task
                        .depends_on
                        .iter()
                        .all(|dep| fixed.contains_key(dep) || child.placed.contains_key(dep))
            })
            .min_by(|left, right| compare_units(&units[*left], &units[*right], index, rule))
            .cloned();
        let Some(id) = ready else {
            break;
        };
        let unit = &units[&id];
        let dependency_end = unit
            .task
            .depends_on
            .iter()
            .map(|dep| {
                fixed
                    .get(dep)
                    .or_else(|| child.placed.get(dep))
                    .unwrap()
                    .end
            })
            .max()
            .unwrap_or(0);
        let mut start = chunk.start.max(unit.release).max(dependency_end);
        let limit = chunk.end.min(unit.deadline);
        let end = loop {
            let Some(end) = start.checked_add(unit.duration).filter(|&end| end <= limit) else {
                break None;
            };
            // Time order is essential: an id-ordered scan can skip an earlier gap.
            let overlap = child
                .placed
                .values()
                .filter(|step| start < step.end && end > step.start)
                .min_by_key(|step| (step.start, step.end, &step.task_id));
            if let Some(step) = overlap {
                start = step.end;
            } else {
                break Some(end);
            }
        };
        if let Some(end) = end {
            child.remaining.remove(&id);
            child.utility += utility(unit, end, window);
            child.placed.insert(
                id.clone(),
                PlanStep {
                    task_id: id,
                    start,
                    end,
                    depends_on: unit.task.depends_on.clone(),
                    static_anchor: false,
                },
            );
        } else {
            failed.insert(id);
        }
    }
    if let Some(id) = child
        .remaining
        .iter()
        .find(|id| units[*id].eligible.last() == Some(&index))
    {
        Err(id.clone())
    } else {
        Ok(child)
    }
}

fn look_ahead(
    branch: &Branch,
    units: &BTreeMap<String, Unit<'_>>,
    chunks: &[Chunk],
    index: usize,
) -> bool {
    if branch
        .remaining
        .iter()
        .any(|id| !units[id].eligible.iter().any(|&i| i > index))
    {
        return false;
    }
    let mut capacity = 0_u128;
    for (j, chunk) in chunks.iter().enumerate().skip(index + 1) {
        capacity += u128::from(chunk.end - chunk.start);
        let required: u128 = branch
            .remaining
            .iter()
            .filter(|id| units[*id].eligible.last().is_some_and(|&last| last <= j))
            .map(|id| u128::from(units[id].duration))
            .sum();
        if required > capacity {
            return false;
        }
    }
    true
}

fn optimistic_score(
    branch: &Branch,
    units: &BTreeMap<String, Unit<'_>>,
    chunks: &[Chunk],
    index: usize,
    window: &TimeWindow,
) -> f64 {
    // BTreeSet iteration fixes the summation order to task id.
    branch.utility
        + branch
            .remaining
            .iter()
            .map(|id| {
                let unit = &units[id];
                let next = *unit
                    .eligible
                    .iter()
                    .find(|&&i| i > index)
                    .expect("look-ahead retained a later chunk");
                utility(
                    unit,
                    chunks[next]
                        .start
                        .max(unit.release)
                        .saturating_add(unit.duration),
                    window,
                )
            })
            .sum::<f64>()
}

pub fn sweep_plans(
    request: &PlanningRequest,
    strategy: &ChunkedSweepStrategy,
) -> Result<Vec<Plan>, SkeletonFailureDiagnostic> {
    let placements = affix_fixed(request)?;
    let window = placements.plan_window;
    let fixed: BTreeMap<_, _> = placements
        .preserved
        .iter()
        .chain(placements.affixed.iter())
        .map(|(id, step)| (id.clone(), step.clone()))
        .collect();
    let chunks = partition(
        window,
        &placements
            .occupied
            .iter()
            .map(|interval| (interval.start, interval.end))
            .collect::<Vec<_>>(),
    );
    let tasks: BTreeMap<_, _> = request
        .tasks()
        .iter()
        .map(|task| (task.id.clone(), task))
        .collect();
    for id in &placements.ordered_tasks {
        if let Some(step) = fixed.get(id) {
            let dependencies = if placements.preserved.contains_key(id) {
                &step.depends_on
            } else {
                &tasks[id].depends_on
            };
            for dep in dependencies {
                if fixed
                    .get(dep)
                    .is_some_and(|prerequisite| prerequisite.end > step.start)
                {
                    return Err(SkeletonFailureDiagnostic {
                        task_id: Some(id.clone()),
                        reason: if placements.preserved.contains_key(id) {
                            format!(
                                "preserved placement starts before dependency '{dep}' completes"
                            )
                        } else {
                            "static anchor collides with dependencies or window start".to_string()
                        },
                    });
                }
            }
        }
    }
    let mut units: BTreeMap<String, Unit<'_>> = BTreeMap::new();
    for id in &placements.ordered_tasks {
        if fixed.contains_key(id) {
            continue;
        }
        let task = tasks[id];
        let mut release = window
            .start
            .max(task.window.as_ref().map_or(window.start, |w| w.start));
        for dep in &task.depends_on {
            let end = if let Some(step) = fixed.get(dep) {
                step.end
            } else {
                let prerequisite = &units[dep];
                prerequisite.release.saturating_add(prerequisite.duration)
            };
            release = release.max(end);
        }
        units.insert(
            id.clone(),
            Unit {
                task,
                duration: task.duration.placement_seconds(),
                weight: task.value * task.priority,
                release,
                deadline: window
                    .end
                    .min(task.window.as_ref().map_or(window.end, |w| w.end)),
                eligible: Vec::new(),
            },
        );
    }
    // Propagate deadlines back through both fixed and movable dependents.
    for id in placements.ordered_tasks.iter().rev() {
        let (latest_start, dependencies) = if let Some(step) = fixed.get(id) {
            (
                step.start,
                if placements.preserved.contains_key(id) {
                    &step.depends_on
                } else {
                    &tasks[id].depends_on
                },
            )
        } else {
            let unit = &units[id];
            (
                unit.deadline.saturating_sub(unit.duration),
                &unit.task.depends_on,
            )
        };
        for dep in dependencies {
            if let Some(unit) = units.get_mut(dep) {
                unit.deadline = unit.deadline.min(latest_start);
            }
        }
    }
    for (id, unit) in &mut units {
        unit.eligible = chunks
            .iter()
            .enumerate()
            .filter_map(|(index, chunk)| {
                chunk
                    .start
                    .max(unit.release)
                    .checked_add(unit.duration)
                    .is_some_and(|end| end <= chunk.end.min(unit.deadline))
                    .then_some(index)
            })
            .collect();
        if unit.eligible.is_empty() {
            return Err(SkeletonFailureDiagnostic {
                task_id: Some(id.clone()),
                reason: "no chunk can hold the task inside its window".to_string(),
            });
        }
    }
    let mut beam = vec![Branch {
        remaining: units.keys().cloned().collect(),
        placed: BTreeMap::new(),
        utility: 0.0,
    }];
    for (index, chunk) in chunks.iter().enumerate() {
        let mut merged: BTreeMap<BTreeSet<String>, Branch> = BTreeMap::new();
        let mut stranded = BTreeSet::new();
        for branch in &beam {
            let mut seen = BTreeSet::new();
            for &rule in FILL_RULES
                .iter()
                .take(strategy.alternatives_per_chunk.clamp(1, 3))
            {
                let child = match fill(branch, &units, &fixed, chunk, index, rule, window) {
                    Ok(child) => child,
                    Err(id) => {
                        stranded.insert(id);
                        continue;
                    }
                };
                let key = placement_key(child.placed.values());
                if !seen.insert(key.clone()) || !look_ahead(&child, &units, &chunks, index) {
                    continue;
                }
                match merged.entry(child.remaining.clone()) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(child);
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        let old = entry.get();
                        if child
                            .utility
                            .total_cmp(&old.utility)
                            .then_with(|| placement_key(old.placed.values()).cmp(&key))
                            .is_gt()
                        {
                            entry.insert(child);
                        }
                    }
                }
            }
        }
        beam = merged.into_values().collect();
        beam.sort_by(|left, right| {
            optimistic_score(right, &units, &chunks, index, window)
                .total_cmp(&optimistic_score(left, &units, &chunks, index, window))
                .then_with(|| {
                    placement_key(left.placed.values()).cmp(&placement_key(right.placed.values()))
                })
        });
        beam.truncate(strategy.beam_width.max(1));
        if beam.is_empty() {
            return Err(SkeletonFailureDiagnostic {
                task_id: stranded.pop_first(),
                reason: format!("chunked sweep cannot place every task by chunk {index}"),
            });
        }
    }
    let base = plan_id(request);
    Ok(beam
        .into_iter()
        .take(MAX_SWEEP_CANDIDATES)
        .enumerate()
        .map(|(index, branch)| {
            let mut steps: Vec<_> = fixed
                .values()
                .cloned()
                .chain(branch.placed.into_values())
                .collect();
            steps.sort_by(|left, right| {
                (left.start, left.end, &left.task_id).cmp(&(right.start, right.end, &right.task_id))
            });
            Plan {
                plan_id: if index == 0 {
                    base.clone()
                } else {
                    format!("{base}-s{index:02}")
                },
                status: PlanStatus::Candidate,
                supersedes_plan_id: request
                    .repair_context
                    .as_ref()
                    .map(|context| context.prior_plan_id.clone()),
                steps,
            }
        })
        .collect())
}

impl ubu_planning_core::PlannerStrategy for ChunkedSweepStrategy {
    fn generate_candidates(&self, request: &PlanningRequest) -> ubu_planning_core::CandidateSet {
        let sweep = sweep_plans(request, self);
        let greedy = crate::skeleton::build_skeleton(request);
        let mut plans = match sweep {
            Ok(plans) => plans,
            Err(diagnostic) => {
                if greedy.is_err() {
                    return ubu_planning_core::CandidateSet {
                        unplaced: Vec::new(),
                        plans: Vec::new(),
                        diagnostics: vec![diagnostic.into()],
                    };
                }
                Vec::new()
            }
        };
        if let Ok(mut baseline) = greedy {
            let key = placement_key(baseline.steps.iter());
            if !plans
                .iter()
                .any(|plan| placement_key(plan.steps.iter()) == key)
            {
                baseline.plan_id.push_str("-greedy");
                if plans.len() == MAX_SWEEP_CANDIDATES {
                    plans.pop();
                }
                plans.push(baseline);
            }
        }
        add_tail_delays(request, &mut plans);
        ubu_planning_core::CandidateSet {
            unplaced: Vec::new(),
            plans,
            diagnostics: Vec::new(),
        }
    }
}

fn add_tail_delays(request: &PlanningRequest, plans: &mut Vec<Plan>) {
    let Some(first) = plans.first().cloned() else {
        return;
    };
    let Ok(fixed) = affix_fixed(request) else {
        return;
    };
    let chunks = partition(
        fixed.plan_window,
        &fixed
            .occupied
            .iter()
            .map(|interval| (interval.start, interval.end))
            .collect::<Vec<_>>(),
    );
    let mut proposals = Vec::new();
    // Indices reference the first candidate, whose step order may be topological.
    let tails: Vec<Vec<usize>> = chunks
        .iter()
        .map(|chunk| {
            let mut indices: Vec<_> = first
                .steps
                .iter()
                .enumerate()
                .filter(|(_, step)| {
                    !step.static_anchor
                        && !fixed.preserved.contains_key(&step.task_id)
                        && step.start >= chunk.start
                        && step.end <= chunk.end
                })
                .map(|(index, _)| index)
                .collect();
            indices.sort_by_key(|&index| {
                let step = &first.steps[index];
                (step.start, step.end, &step.task_id)
            });
            indices
        })
        .collect();
    for (chunk_index, indices) in tails.iter().enumerate() {
        for pivot in 0..indices.len() {
            let maximum_shift = indices[pivot..]
                .iter()
                .map(|&index| {
                    let step = &first.steps[index];
                    let window_end = request
                        .tasks()
                        .iter()
                        .find(|task| task.id == step.task_id)
                        .and_then(|task| task.window.as_ref())
                        .map_or(chunks[chunk_index].end, |w| {
                            w.end.min(chunks[chunk_index].end)
                        });
                    window_end.saturating_sub(step.end)
                })
                .min()
                .unwrap_or(0);
            for ordinal in 1..=maximum_shift.min(15) {
                let shift = (u128::from(ordinal) * u128::from(maximum_shift)
                    / u128::from(maximum_shift.min(15))) as u64;
                proposals.push((
                    crate::candidate_generation::proposal_key(
                        request.rng_seed,
                        (chunk_index << 16) | pivot,
                        shift,
                    ),
                    chunk_index,
                    pivot,
                    shift,
                ));
            }
        }
    }
    proposals.sort_unstable();
    let mut seen: BTreeSet<_> = plans
        .iter()
        .map(|plan| placement_key(plan.steps.iter()))
        .collect();
    let mut ordinal = 1;
    for (_, chunk_index, pivot, shift) in proposals {
        if plans.len() == MAX_SWEEP_CANDIDATES {
            break;
        }
        let mut candidate = first.clone();
        for &index in &tails[chunk_index][pivot..] {
            candidate.steps[index].start += shift;
            candidate.steps[index].end += shift;
        }
        if !seen.insert(placement_key(candidate.steps.iter())) {
            continue;
        }
        candidate.plan_id = format!("{}-d{ordinal:02}", first.plan_id);
        ordinal += 1;
        plans.push(candidate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_sorts_and_merges_touching_intervals() {
        assert_eq!(
            partition(
                &TimeWindow { start: 0, end: 100 },
                &[(50, 60), (10, 20), (20, 30)]
            ),
            vec![
                Chunk { start: 0, end: 10 },
                Chunk { start: 30, end: 50 },
                Chunk {
                    start: 60,
                    end: 100
                }
            ]
        );
    }

    #[test]
    fn partition_clips_overlaps_and_drops_empty_gaps() {
        let window = TimeWindow { start: 10, end: 90 };
        assert_eq!(
            partition(&window, &[(0, 20), (15, 30), (80, 120), (40, 40)]),
            vec![Chunk { start: 30, end: 80 }]
        );
        assert!(partition(&window, &[(0, 100)]).is_empty());
        assert_eq!(partition(&window, &[]), vec![Chunk { start: 10, end: 90 }]);
    }

    #[test]
    fn defaults_and_rule_order_are_stable() {
        let strategy = ChunkedSweepStrategy::default();
        assert_eq!(
            (strategy.alternatives_per_chunk, strategy.beam_width),
            (3, 16)
        );
        assert_eq!(MAX_SWEEP_CANDIDATES, 16);
        assert_eq!(
            FILL_RULES,
            [
                FillRule::ValueFirst,
                FillRule::MostConstrainedFirst,
                FillRule::ValueDensity
            ]
        );
    }
}
