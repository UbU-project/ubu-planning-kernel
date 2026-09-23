//! Deterministic bounded beam over free chunks between fixed placements.
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use ubu_planning_core::{
    compare_omissions, ChunkRef, Plan, PlanStatus, PlanStep, PlanningRequest, SelectionRank,
    SkeletonFailureDiagnostic, TaskSpec, TimeWindow, UnplacedReason, UnplacedTask,
};

use crate::skeleton::{affix_fixed, plan_id};

pub const DEFAULT_ALTERNATIVES_PER_CHUNK: usize = 4;
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
    ProtectedFirst,
}
pub const FILL_RULES: [FillRule; 4] = [
    FillRule::ValueFirst,
    FillRule::MostConstrainedFirst,
    FillRule::ValueDensity,
    FillRule::ProtectedFirst,
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

#[derive(Clone)]
struct Unit<'a> {
    task: &'a TaskSpec,
    duration: u64,
    weight: f64,
    release: u64,
    deadline: u64,
    base_deadline: u64,
    protected: bool,
    rank: SelectionRank,
    eligible: Vec<usize>,
}

#[derive(Clone)]
struct Branch {
    remaining: BTreeSet<String>,
    omitted: BTreeMap<String, UnplacedReason>,
    excluded: BTreeSet<String>,
    eligible_at_omission: BTreeMap<String, Vec<ChunkRef>>,
    placed: BTreeMap<String, PlanStep>,
    utility: f64,
}

struct Sweep<'a> {
    units: BTreeMap<String, Unit<'a>>,
    fixed: BTreeMap<String, PlanStep>,
    chunks: Vec<Chunk>,
    dependents: BTreeMap<String, BTreeSet<String>>,
    window: &'a TimeWindow,
    order: Vec<String>,
}
#[derive(Debug)]
pub struct SweepOutcome {
    pub plans: Vec<Plan>,
    pub unplaced: Vec<UnplacedTask>,
}
impl<'a> Sweep<'a> {
    fn recompute_eligibility(&self, branch: &Branch) -> BTreeMap<String, Unit<'a>> {
        let mut units = self.units.clone();
        for unit in units.values_mut() {
            unit.deadline = unit.base_deadline;
        }
        for id in self.order.iter().rev() {
            if branch.excluded.contains(id) {
                continue;
            }
            let (start, deps) = if let Some(step) = self.fixed.get(id) {
                (step.start, step.depends_on.clone())
            } else {
                let unit = &units[id];
                (
                    unit.deadline.saturating_sub(unit.duration),
                    unit.task.depends_on.clone(),
                )
            };
            for dep in deps {
                if let Some(unit) = units.get_mut(&dep) {
                    unit.deadline = unit.deadline.min(start);
                }
            }
        }
        for unit in units.values_mut() {
            unit.eligible = self
                .chunks
                .iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    c.start
                        .max(unit.release)
                        .checked_add(unit.duration)
                        .is_some_and(|end| end <= c.end.min(unit.deadline))
                        .then_some(i)
                })
                .collect();
        }
        units
    }
    fn exclude(&self, branch: &mut Branch, id: &str, reason: UnplacedReason) {
        let units = self.recompute_eligibility(branch);
        let roots = BTreeSet::from([id.to_owned()]);
        let excluded = crate::protection::dependents_of(&self.dependents, &roots)
            .union(&roots)
            .cloned()
            .collect::<BTreeSet<_>>();
        for task in &excluded {
            if branch.excluded.contains(task) {
                continue;
            }
            if let Some(unit) = units.get(task) {
                branch.eligible_at_omission.insert(
                    task.clone(),
                    unit.eligible
                        .iter()
                        .map(|&index| ChunkRef {
                            index,
                            start: self.chunks[index].start,
                            end: self.chunks[index].end,
                        })
                        .collect(),
                );
            }
            branch.remaining.remove(task);
        }
        branch.omitted.insert(id.into(), reason);
        branch.excluded.extend(excluded);
    }
    fn strand_reason(&self, branch: &Branch, unit: &Unit<'_>) -> UnplacedReason {
        if branch.placed.values().any(|step| {
            !self.units[&step.task_id].protected
                && unit
                    .eligible
                    .iter()
                    .any(|&i| step.start < self.chunks[i].end && step.end > self.chunks[i].start)
        }) {
            UnplacedReason::OmittedLowerValue
        } else {
            UnplacedReason::InsufficientTotalCapacity
        }
    }
    fn omission_ranks(&self, branch: &Branch) -> Vec<SelectionRank> {
        branch
            .excluded
            .iter()
            .map(|id| self.units[id].rank.clone())
            .collect()
    }
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
        .then_with(|| {
            if forced(left) && forced(right) {
                right.protected.cmp(&left.protected)
            } else {
                Ordering::Equal
            }
        })
        .then_with(|| match rule {
            FillRule::ProtectedFirst => right
                .protected
                .cmp(&left.protected)
                .then_with(|| left.rank.compare_protection(&right.rank)),
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
    sweep: &Sweep<'_>,
    index: usize,
    rule: FillRule,
) -> Result<Branch, String> {
    let mut units = sweep.recompute_eligibility(branch);
    let fixed = &sweep.fixed;
    let chunk = &sweep.chunks[index];
    let window = sweep.window;
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
    loop {
        let id = child
            .remaining
            .iter()
            .filter(|id| units[*id].eligible.last().is_none_or(|&last| last <= index))
            .min_by_key(|id| {
                (
                    !units[*id]
                        .task
                        .depends_on
                        .iter()
                        .all(|dep| fixed.contains_key(dep) || child.placed.contains_key(dep)),
                    *id,
                )
            })
            .cloned();
        let Some(id) = id else {
            break;
        };
        if units[&id].protected {
            return Err(id);
        }
        let reason = sweep.strand_reason(&child, &units[&id]);
        sweep.exclude(&mut child, &id, reason);
        units = sweep.recompute_eligibility(&child);
    }
    Ok(child)
}

fn project_omissions(branch: &mut Branch, sweep: &Sweep<'_>, index: usize) -> bool {
    let mut units = sweep.recompute_eligibility(branch);
    loop {
        let stranded = branch
            .remaining
            .iter()
            .find(|id| !units[*id].eligible.iter().any(|&i| i > index))
            .cloned();
        let Some(id) = stranded else {
            break;
        };
        if units[&id].protected {
            return false;
        }
        let reason = sweep.strand_reason(branch, &units[&id]);
        sweep.exclude(branch, &id, reason);
        units = sweep.recompute_eligibility(branch);
    }
    let mut capacity = 0_u128;
    for (j, chunk) in sweep.chunks.iter().enumerate().skip(index + 1) {
        capacity += u128::from(chunk.end - chunk.start);
        loop {
            let due: Vec<_> = branch
                .remaining
                .iter()
                .filter(|id| units[*id].eligible.last().is_some_and(|&last| last <= j))
                .cloned()
                .collect();
            let required: u128 = due.iter().map(|id| u128::from(units[id].duration)).sum();
            if required <= capacity {
                break;
            }
            let Some(id) = due
                .iter()
                .filter(|id| !units[*id].protected)
                .max_by(|a, b| units[*a].rank.compare_protection(&units[*b].rank))
            else {
                return false;
            };
            let reason = sweep.strand_reason(branch, &units[id]);
            sweep.exclude(branch, id, reason);
            units = sweep.recompute_eligibility(branch);
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
                let Some(&next) = unit.eligible.iter().find(|&&i| i > index) else {
                    return 0.0;
                };
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
) -> Result<SweepOutcome, SkeletonFailureDiagnostic> {
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
    let protected = crate::protection::protected_tasks(request, fixed.keys().cloned());
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
                base_deadline: window
                    .end
                    .min(task.window.as_ref().map_or(window.end, |w| w.end)),
                protected: protected.contains(id),
                rank: crate::protection::selection_rank(task),
                eligible: Vec::new(),
            },
        );
    }
    let sweep = Sweep {
        units,
        fixed,
        chunks,
        dependents: crate::protection::dependent_index(request),
        window,
        order: placements.ordered_tasks,
    };
    let mut root = Branch {
        remaining: sweep.units.keys().cloned().collect(),
        placed: BTreeMap::new(),
        utility: 0.0,
        omitted: BTreeMap::new(),
        excluded: BTreeSet::new(),
        eligible_at_omission: BTreeMap::new(),
    };
    for id in &sweep.order {
        if !root.remaining.contains(id) {
            continue;
        }
        let unit = &sweep.units[id];
        if unit
            .release
            .checked_add(unit.duration)
            .is_none_or(|end| end > unit.base_deadline)
        {
            if unit.protected {
                return Err(SkeletonFailureDiagnostic {
                    task_id: Some(id.clone()),
                    reason: "task has insufficient available window".into(),
                });
            }
            sweep.exclude(&mut root, id, UnplacedReason::OutsideAllowedWindow);
        }
    }
    loop {
        let units = sweep.recompute_eligibility(&root);
        let id = root
            .remaining
            .iter()
            .filter(|id| units[*id].eligible.is_empty())
            .min_by_key(|id| {
                (
                    sweep
                        .dependents
                        .get(*id)
                        .is_some_and(|deps| deps.iter().any(|d| root.remaining.contains(d))),
                    *id,
                )
            })
            .cloned();
        let Some(id) = id else {
            break;
        };
        if units[&id].protected {
            return Err(SkeletonFailureDiagnostic {
                task_id: Some(id),
                reason: "no chunk can hold the task inside its window".into(),
            });
        }
        sweep.exclude(&mut root, &id, UnplacedReason::NoEligibleChunkLargeEnough);
    }
    let mut beam = vec![root];
    for index in 0..sweep.chunks.len() {
        let mut merged: BTreeMap<(BTreeSet<String>, BTreeSet<String>), Branch> = BTreeMap::new();
        let mut stranded = BTreeSet::new();
        for branch in &beam {
            let mut seen = BTreeSet::new();
            for &rule in FILL_RULES
                .iter()
                .take(strategy.alternatives_per_chunk.clamp(1, FILL_RULES.len()))
            {
                let mut child = match fill(branch, &sweep, index, rule) {
                    Ok(child) => child,
                    Err(id) => {
                        stranded.insert(id);
                        continue;
                    }
                };
                let key = placement_key(child.placed.values());
                if !project_omissions(&mut child, &sweep, index)
                    || !seen.insert((child.excluded.clone(), key.clone()))
                {
                    continue;
                }
                match merged.entry((child.excluded.clone(), child.remaining.clone())) {
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
            compare_omissions(&sweep.omission_ranks(left), &sweep.omission_ranks(right))
                .then_with(|| {
                    optimistic_score(
                        right,
                        &sweep.recompute_eligibility(right),
                        &sweep.chunks,
                        index,
                        window,
                    )
                    .total_cmp(&optimistic_score(
                        left,
                        &sweep.recompute_eligibility(left),
                        &sweep.chunks,
                        index,
                        window,
                    ))
                })
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
    let best = &beam[0];
    if best.placed.is_empty() && sweep.fixed.is_empty() {
        return Err(SkeletonFailureDiagnostic {
            task_id: None,
            reason: "partial placement left no Task in the Plan".into(),
        });
    }
    let excluded = best.excluded.clone();
    let unplaced = crate::skeleton::unplaced_report(
        request,
        &best.omitted,
        &best.excluded,
        &sweep.dependents,
        &best.eligible_at_omission,
    );
    let base = plan_id(request);
    let plans = beam
        .into_iter()
        .filter(|branch| branch.excluded == excluded)
        .take(MAX_SWEEP_CANDIDATES)
        .enumerate()
        .map(|(index, branch)| {
            let mut steps: Vec<_> = sweep
                .fixed
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
        .collect();
    Ok(SweepOutcome { plans, unplaced })
}

impl ubu_planning_core::PlannerStrategy for ChunkedSweepStrategy {
    fn generate_candidates(&self, request: &PlanningRequest) -> ubu_planning_core::CandidateSet {
        let sweep = sweep_plans(request, self);
        let greedy = crate::skeleton::build_skeleton(request);
        let (mut plans, mut unplaced) = match sweep {
            Ok(outcome) => (outcome.plans, outcome.unplaced),
            Err(diagnostic) => {
                if greedy.is_err() {
                    return ubu_planning_core::CandidateSet {
                        plans: Vec::new(),
                        unplaced: Vec::new(),
                        diagnostics: vec![diagnostic.into()],
                    };
                }
                (Vec::new(), Vec::new())
            }
        };
        if let Ok(outcome) = greedy {
            let ranks = |report: &[UnplacedTask]| {
                report
                    .iter()
                    .map(|u| u.selection_rank.clone())
                    .collect::<Vec<_>>()
            };
            let order = if plans.is_empty() {
                Ordering::Less
            } else {
                compare_omissions(&ranks(&outcome.unplaced), &ranks(&unplaced))
            };
            if order.is_lt() {
                plans.clear();
                unplaced = outcome.unplaced;
            }
            if !order.is_gt() {
                let mut baseline = outcome.plan;
                let key = placement_key(baseline.steps.iter());
                if !plans.iter().any(|p| placement_key(p.steps.iter()) == key) {
                    baseline.plan_id.push_str("-greedy");
                    if plans.len() == MAX_SWEEP_CANDIDATES {
                        plans.pop();
                    }
                    plans.push(baseline);
                }
            }
        }
        add_tail_delays(request, &mut plans);
        ubu_planning_core::CandidateSet {
            unplaced,
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
            (4, 16)
        );
        assert_eq!(MAX_SWEEP_CANDIDATES, 16);
        assert_eq!(
            FILL_RULES,
            [
                FillRule::ValueFirst,
                FillRule::MostConstrainedFirst,
                FillRule::ValueDensity,
                FillRule::ProtectedFirst
            ]
        );
    }
}
