use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use ubu_planning_core::diagnostics::SkeletonFailureDiagnostic;
use ubu_planning_core::request::{
    PlanningMode, PlanningRequest, RepairScope, TaskSpec, TimeWindow,
};
use ubu_planning_core::response::{Plan, PlanStatus, PlanStep};

#[derive(Debug, Clone)]
struct OccupiedInterval {
    task_id: String,
    start: u64,
    end: u64,
}

pub fn build_skeleton(request: &PlanningRequest) -> Result<Plan, SkeletonFailureDiagnostic> {
    let plan_window = request
        .time_window
        .as_ref()
        .ok_or_else(|| SkeletonFailureDiagnostic {
            task_id: None,
            reason: "missing time_window starting state".to_string(),
        })?;
    if !plan_window.is_possible() {
        return Err(SkeletonFailureDiagnostic {
            task_id: None,
            reason: "time_window has no available duration".to_string(),
        });
    }

    let ordered_tasks = requested_or_computed_order(request)?;
    let task_by_id: HashMap<_, _> = request
        .tasks()
        .iter()
        .map(|task| (task.id.clone(), task))
        .collect();
    let preserved = preserved_steps(request, plan_window, &task_by_id);
    let mut occupied = Vec::new();
    let mut scheduled_by_id = HashMap::new();

    for step in preserved.values() {
        push_occupied(&mut occupied, step)?;
    }

    for task_id in &ordered_tasks {
        if let Some(step) = preserved.get(task_id) {
            validate_preserved_step(step, &scheduled_by_id)?;
            scheduled_by_id.insert(task_id.clone(), step.clone());
            continue;
        }

        let task = task_by_id
            .get(task_id)
            .ok_or_else(|| SkeletonFailureDiagnostic {
                task_id: Some(task_id.clone()),
                reason: "task disappeared during skeleton generation".to_string(),
            })?;
        let dependency_end = dependency_end(task, &scheduled_by_id)?;
        let earliest_start = plan_window.start.max(dependency_end);
        let step = place_task(task, plan_window, earliest_start, &occupied)?;
        push_occupied(&mut occupied, &step)?;
        scheduled_by_id.insert(task_id.clone(), step);
    }

    let mut steps = Vec::with_capacity(ordered_tasks.len());
    for task_id in ordered_tasks {
        let step = scheduled_by_id
            .remove(&task_id)
            .ok_or_else(|| SkeletonFailureDiagnostic {
                task_id: Some(task_id.clone()),
                reason: "task was not scheduled".to_string(),
            })?;
        steps.push(step);
    }

    Ok(Plan {
        plan_id: plan_id(request),
        status: PlanStatus::Candidate,
        supersedes_plan_id: request
            .repair_context
            .as_ref()
            .map(|context| context.prior_plan_id.clone()),
        steps,
    })
}

fn plan_id(request: &PlanningRequest) -> String {
    match request.mode {
        PlanningMode::FreshGeneration => {
            format!("plan-{}-{:016x}", request.request_id, request.rng_seed)
        }
        PlanningMode::Repair => {
            format!(
                "plan-{}-{:016x}-repair",
                request.request_id, request.rng_seed
            )
        }
    }
}

fn requested_or_computed_order(
    request: &PlanningRequest,
) -> Result<Vec<String>, SkeletonFailureDiagnostic> {
    let computed = topological_order(request.tasks())?;
    if request.topological_order().is_empty() {
        return Ok(computed);
    }

    validate_provided_topological_order(request)?;
    Ok(request.topological_order().to_vec())
}

fn validate_provided_topological_order(
    request: &PlanningRequest,
) -> Result<(), SkeletonFailureDiagnostic> {
    let task_ids: HashSet<_> = request
        .tasks()
        .iter()
        .map(|task| task.id.as_str())
        .collect();
    let order = request.topological_order();
    if order.len() != task_ids.len() {
        return Err(SkeletonFailureDiagnostic {
            task_id: None,
            reason: "provided topological_order length does not match task graph".to_string(),
        });
    }

    let mut seen = HashSet::new();
    for task_id in order {
        if !task_ids.contains(task_id.as_str()) {
            return Err(SkeletonFailureDiagnostic {
                task_id: Some(task_id.clone()),
                reason: "provided topological_order contains an unknown task".to_string(),
            });
        }
        if !seen.insert(task_id.as_str()) {
            return Err(SkeletonFailureDiagnostic {
                task_id: Some(task_id.clone()),
                reason: "provided topological_order contains a duplicate task".to_string(),
            });
        }
    }

    let positions: HashMap<_, _> = order
        .iter()
        .enumerate()
        .map(|(index, task_id)| (task_id.as_str(), index))
        .collect();
    for task in request.tasks() {
        let task_position =
            positions
                .get(task.id.as_str())
                .copied()
                .ok_or_else(|| SkeletonFailureDiagnostic {
                    task_id: Some(task.id.clone()),
                    reason: "provided topological_order omits a task".to_string(),
                })?;
        for dependency in &task.depends_on {
            let dependency_position =
                positions.get(dependency.as_str()).copied().ok_or_else(|| {
                    SkeletonFailureDiagnostic {
                        task_id: Some(task.id.clone()),
                        reason: format!(
                            "provided topological_order omits dependency '{dependency}'"
                        ),
                    }
                })?;
            if dependency_position >= task_position {
                return Err(SkeletonFailureDiagnostic {
                    task_id: Some(task.id.clone()),
                    reason: format!(
                        "provided topological_order places dependency '{dependency}' after task"
                    ),
                });
            }
        }
    }

    Ok(())
}

fn preserved_steps<'a>(
    request: &'a PlanningRequest,
    plan_window: &TimeWindow,
    task_by_id: &HashMap<String, &'a TaskSpec>,
) -> HashMap<String, PlanStep> {
    if request.mode != PlanningMode::Repair {
        return HashMap::new();
    }

    let Some(prior_plan) = &request.prior_plan else {
        return HashMap::new();
    };
    let divergence_refs: HashSet<_> = request
        .repair_context
        .as_ref()
        .map(|context| {
            context
                .observed_divergence_refs
                .iter()
                .map(String::as_str)
                .collect()
        })
        .unwrap_or_default();
    let repair_scope = request
        .repair_context
        .as_ref()
        .map(|context| context.repair_scope)
        .unwrap_or(RepairScope::RemainingWindow);

    prior_plan
        .steps
        .iter()
        .filter(|step| task_by_id.contains_key(&step.task_id))
        .filter(|step| should_preserve_step(step, plan_window, repair_scope, &divergence_refs))
        .map(|step| (step.task_id.clone(), step.clone()))
        .collect()
}

fn should_preserve_step(
    step: &PlanStep,
    plan_window: &TimeWindow,
    repair_scope: RepairScope,
    divergence_refs: &HashSet<&str>,
) -> bool {
    let completed_before_window = step.end <= plan_window.start;
    let in_progress_at_window_start =
        step.start < plan_window.start && step.end > plan_window.start;
    let user_override = step.static_anchor;
    let outside_local_repair =
        repair_scope == RepairScope::Local && !divergence_refs.contains(step.task_id.as_str());

    completed_before_window || in_progress_at_window_start || user_override || outside_local_repair
}

fn dependency_end(
    task: &TaskSpec,
    scheduled_by_id: &HashMap<String, PlanStep>,
) -> Result<u64, SkeletonFailureDiagnostic> {
    let mut dependency_end = 0_u64;
    for dependency in &task.depends_on {
        let dependency_step =
            scheduled_by_id
                .get(dependency)
                .ok_or_else(|| SkeletonFailureDiagnostic {
                    task_id: Some(task.id.clone()),
                    reason: format!("dependency '{dependency}' was not scheduled first"),
                })?;
        dependency_end = dependency_end.max(dependency_step.end);
    }
    Ok(dependency_end)
}

fn validate_preserved_step(
    step: &PlanStep,
    scheduled_by_id: &HashMap<String, PlanStep>,
) -> Result<(), SkeletonFailureDiagnostic> {
    for dependency in &step.depends_on {
        let dependency_step =
            scheduled_by_id
                .get(dependency)
                .ok_or_else(|| SkeletonFailureDiagnostic {
                    task_id: Some(step.task_id.clone()),
                    reason: format!("preserved dependency '{dependency}' was not scheduled first"),
                })?;
        if dependency_step.end > step.start {
            return Err(SkeletonFailureDiagnostic {
                task_id: Some(step.task_id.clone()),
                reason: format!(
                    "preserved placement starts before dependency '{dependency}' completes"
                ),
            });
        }
    }
    Ok(())
}

fn place_task(
    task: &TaskSpec,
    plan_window: &TimeWindow,
    earliest_start: u64,
    occupied: &[OccupiedInterval],
) -> Result<PlanStep, SkeletonFailureDiagnostic> {
    let duration = task.duration.placement_seconds();
    let window = task_effective_window(task, plan_window)?;
    let minimum_start = earliest_start.max(window.start);

    if let Some(anchor) = &task.static_anchor {
        return anchored_step(task, anchor.start, minimum_start, window.end, occupied);
    }

    let mut start = minimum_start;
    loop {
        let Some(end) = start.checked_add(duration) else {
            return Err(SkeletonFailureDiagnostic {
                task_id: Some(task.id.clone()),
                reason: "task duration overflows its start".to_string(),
            });
        };
        if end > window.end {
            return Err(SkeletonFailureDiagnostic {
                task_id: Some(task.id.clone()),
                reason: "insufficient available window for deterministic skeleton placement"
                    .to_string(),
            });
        }
        if let Some(overlap) = first_overlap(start, end, occupied) {
            start = overlap.end.max(start.saturating_add(1));
            continue;
        }
        return Ok(plan_step(task, start, end));
    }
}

fn anchored_step(
    task: &TaskSpec,
    start: u64,
    minimum_start: u64,
    window_end: u64,
    occupied: &[OccupiedInterval],
) -> Result<PlanStep, SkeletonFailureDiagnostic> {
    let duration = task.duration.placement_seconds();
    if start < minimum_start {
        return Err(SkeletonFailureDiagnostic {
            task_id: Some(task.id.clone()),
            reason: "static anchor collides with dependencies or window start".to_string(),
        });
    }
    let Some(end) = start.checked_add(duration) else {
        return Err(SkeletonFailureDiagnostic {
            task_id: Some(task.id.clone()),
            reason: "static anchor overflows task duration".to_string(),
        });
    };
    if end > window_end {
        return Err(SkeletonFailureDiagnostic {
            task_id: Some(task.id.clone()),
            reason: "static anchor exceeds available window".to_string(),
        });
    }
    if let Some(overlap) = first_overlap(start, end, occupied) {
        return Err(SkeletonFailureDiagnostic {
            task_id: Some(task.id.clone()),
            reason: format!(
                "static anchor collides with scheduled task '{}'",
                overlap.task_id
            ),
        });
    }
    Ok(plan_step(task, start, end))
}

fn task_effective_window(
    task: &TaskSpec,
    plan_window: &TimeWindow,
) -> Result<TimeWindow, SkeletonFailureDiagnostic> {
    let mut window = plan_window.clone();
    if let Some(task_window) = &task.window {
        window.start = window.start.max(task_window.start);
        window.end = window.end.min(task_window.end);
    }
    if !window.is_possible()
        || window.end.saturating_sub(window.start) < task.duration.placement_seconds()
    {
        return Err(SkeletonFailureDiagnostic {
            task_id: Some(task.id.clone()),
            reason: "task has insufficient available window".to_string(),
        });
    }
    Ok(window)
}

fn plan_step(task: &TaskSpec, start: u64, end: u64) -> PlanStep {
    PlanStep {
        task_id: task.id.clone(),
        start,
        end,
        depends_on: task.depends_on.clone(),
        static_anchor: task.static_anchor.is_some(),
    }
}

fn push_occupied(
    occupied: &mut Vec<OccupiedInterval>,
    step: &PlanStep,
) -> Result<(), SkeletonFailureDiagnostic> {
    if let Some(overlap) = first_overlap(step.start, step.end, occupied) {
        return Err(SkeletonFailureDiagnostic {
            task_id: Some(step.task_id.clone()),
            reason: format!(
                "preserved placement collides with scheduled task '{}'",
                overlap.task_id
            ),
        });
    }

    occupied.push(OccupiedInterval {
        task_id: step.task_id.clone(),
        start: step.start,
        end: step.end,
    });
    occupied.sort_by(|left, right| {
        left.start
            .cmp(&right.start)
            .then(left.end.cmp(&right.end))
            .then(left.task_id.cmp(&right.task_id))
    });
    Ok(())
}

fn first_overlap(start: u64, end: u64, occupied: &[OccupiedInterval]) -> Option<&OccupiedInterval> {
    occupied
        .iter()
        .find(|interval| start < interval.end && end > interval.start)
}

fn topological_order(tasks: &[TaskSpec]) -> Result<Vec<String>, SkeletonFailureDiagnostic> {
    let mut children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut indegree: BTreeMap<String, usize> = BTreeMap::new();
    for task in tasks {
        indegree.entry(task.id.clone()).or_insert(0);
        children.entry(task.id.clone()).or_default();
        for dependency in &task.depends_on {
            children
                .entry(dependency.clone())
                .or_default()
                .insert(task.id.clone());
            *indegree.entry(task.id.clone()).or_insert(0) += 1;
        }
    }

    let mut ready: BTreeSet<String> = indegree
        .iter()
        .filter_map(|(task_id, count)| (*count == 0).then_some(task_id.clone()))
        .collect();
    let mut order = Vec::with_capacity(tasks.len());

    while let Some(task_id) = ready.pop_first() {
        order.push(task_id.clone());
        if let Some(next_tasks) = children.get(&task_id) {
            for next in next_tasks {
                let count = indegree
                    .get_mut(next)
                    .ok_or_else(|| SkeletonFailureDiagnostic {
                        task_id: Some(next.clone()),
                        reason: "missing dependency endpoint".to_string(),
                    })?;
                *count -= 1;
                if *count == 0 {
                    ready.insert(next.clone());
                }
            }
        }
    }

    if order.len() != tasks.len() {
        return Err(SkeletonFailureDiagnostic {
            task_id: None,
            reason: "dependency graph did not produce a complete deterministic order".to_string(),
        });
    }
    Ok(order)
}
