//! Shared mandatory/fixed protection and dependency closure.
use std::collections::{BTreeMap, BTreeSet};
use ubu_planning_core::{PlanningRequest, SelectionRank, TaskSpec};

pub fn selection_rank(task: &TaskSpec) -> SelectionRank {
    SelectionRank {
        value: task.value,
        deadline_or_latest_finish: task.window.as_ref().map(|w| w.end),
        task_id: task.id.clone(),
    }
}
pub fn protected_tasks(
    request: &PlanningRequest,
    also_protected: impl IntoIterator<Item = String>,
) -> BTreeSet<String> {
    let tasks: BTreeMap<_, _> = request.tasks().iter().map(|t| (t.id.clone(), t)).collect();
    let mut protected: BTreeSet<_> = also_protected
        .into_iter()
        .chain(
            request
                .tasks()
                .iter()
                .filter(|t| t.mandatory || t.static_anchor.is_some())
                .map(|t| t.id.clone()),
        )
        .collect();
    let mut pending: Vec<_> = protected.iter().cloned().collect();
    while let Some(id) = pending.pop() {
        if let Some(task) = tasks.get(&id) {
            for dep in &task.depends_on {
                if protected.insert(dep.clone()) {
                    pending.push(dep.clone());
                }
            }
        }
    }
    protected
}
pub fn dependent_index(request: &PlanningRequest) -> BTreeMap<String, BTreeSet<String>> {
    let mut index: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for task in request.tasks() {
        for dep in &task.depends_on {
            index
                .entry(dep.clone())
                .or_default()
                .insert(task.id.clone());
        }
    }
    index
}
pub fn dependents_of(
    index: &BTreeMap<String, BTreeSet<String>>,
    roots: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut found = roots.clone();
    let mut pending: Vec<_> = roots.iter().cloned().collect();
    while let Some(id) = pending.pop() {
        if let Some(children) = index.get(&id) {
            for child in children {
                if found.insert(child.clone()) {
                    pending.push(child.clone());
                }
            }
        }
    }
    found.difference(roots).cloned().collect()
}
