use std::collections::{BTreeMap, BTreeSet, HashMap};

use ubu_planning_core::diagnostics::SkeletonFailureDiagnostic;
use ubu_planning_core::request::{PlanningRequest, TaskSpec};
use ubu_planning_core::response::{Plan, PlanStatus, ScheduledTask};

pub fn build_skeleton(request: &PlanningRequest) -> Result<Plan, SkeletonFailureDiagnostic> {
    let ordered_tasks = topological_order(&request.tasks)?;
    let task_by_id: HashMap<_, _> = request
        .tasks
        .iter()
        .map(|task| (task.id.clone(), task))
        .collect();
    let mut cursor = 0_u64;
    let mut scheduled = Vec::with_capacity(ordered_tasks.len());

    for task_id in ordered_tasks {
        let task = task_by_id
            .get(&task_id)
            .ok_or_else(|| SkeletonFailureDiagnostic {
                task_id: Some(task_id.clone()),
                reason: "task disappeared during skeleton generation".to_string(),
            })?;
        let start = task
            .static_anchor
            .as_ref()
            .map_or(cursor, |anchor| anchor.start)
            .max(task.window.as_ref().map_or(0, |window| window.start));
        let end = start + task.duration;
        if let Some(window) = &task.window {
            if end > window.end {
                return Err(SkeletonFailureDiagnostic {
                    task_id: Some(task.id.clone()),
                    reason: "task cannot fit after dependencies in its window".to_string(),
                });
            }
        }
        scheduled.push(ScheduledTask {
            task_id: task.id.clone(),
            start,
            end,
            depends_on: task.depends_on.clone(),
            static_anchor: task.static_anchor.is_some(),
        });
        cursor = end;
    }

    Ok(Plan {
        plan_id: format!("plan-{}", request.request_id),
        status: PlanStatus::Candidate,
        tasks: scheduled,
    })
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
