use std::collections::{HashMap, HashSet};

use crate::diagnostics::{Diagnostic, DiagnosticCode};
use crate::request::PlanningRequest;
use crate::response::{Plan, ValidationResult};

pub fn validate_planning_request(request: &PlanningRequest) -> ValidationResult {
    let mut diagnostics = Vec::new();
    if request.tasks.is_empty() {
        diagnostics.push(Diagnostic::new(
            DiagnosticCode::EmptyRequest,
            "planning request must include at least one task",
        ));
    }

    let mut ids = HashSet::new();
    for task in &request.tasks {
        if !ids.insert(task.id.clone()) {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::DuplicateTaskId,
                format!("duplicate task id '{}'", task.id),
            ));
        }
        if task.duration == 0 {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::ImpossibleWindow,
                format!("task '{}' has zero duration", task.id),
            ));
        }
        if let Some(window) = &task.window {
            if !window.is_possible() || window.end.saturating_sub(window.start) < task.duration {
                diagnostics.push(Diagnostic::new(
                    DiagnosticCode::ImpossibleWindow,
                    format!("task '{}' cannot fit inside its time window", task.id),
                ));
            }
        }
        if let Some(anchor) = &task.static_anchor {
            if let Some(window) = &task.window {
                if anchor.start < window.start || anchor.start + task.duration > window.end {
                    diagnostics.push(Diagnostic::new(
                        DiagnosticCode::ImpossibleWindow,
                        format!("task '{}' static anchor violates its time window", task.id),
                    ));
                }
            }
        }
        if task.affect_required && !task.affect_current {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::StaleAffect,
                format!("task '{}' requires current affect context", task.id),
            ));
        }
    }

    for edge in request.dependency_edges() {
        if !ids.contains(&edge.before) {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::MissingDependency,
                format!(
                    "task '{}' depends on missing task '{}'",
                    edge.after, edge.before
                ),
            ));
        }
    }

    if diagnostics.is_empty() && has_cycle(request) {
        diagnostics.push(Diagnostic::new(
            DiagnosticCode::CyclicDependency,
            "dependency graph must be acyclic",
        ));
    }

    if diagnostics.is_empty() {
        ValidationResult::valid()
    } else {
        ValidationResult::invalid(diagnostics)
    }
}

pub fn validate_plan(candidate: &Plan) -> ValidationResult {
    let mut diagnostics = Vec::new();
    if candidate.tasks.is_empty() {
        diagnostics.push(Diagnostic::new(
            DiagnosticCode::EmptyPlan,
            "plan must include at least one scheduled task",
        ));
    }

    let mut seen = HashSet::new();
    let mut positions = HashMap::new();
    for (index, task) in candidate.tasks.iter().enumerate() {
        if !seen.insert(task.task_id.clone()) {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::DuplicatePlanTask,
                format!("plan repeats task '{}'", task.task_id),
            ));
        }
        if task.start >= task.end {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::ImpossibleWindow,
                format!(
                    "scheduled task '{}' has an impossible interval",
                    task.task_id
                ),
            ));
        }
        positions.insert(task.task_id.clone(), index);
    }

    for (index, task) in candidate.tasks.iter().enumerate() {
        for dependency in &task.depends_on {
            match positions.get(dependency) {
                Some(dependency_index) if *dependency_index < index => {}
                Some(_) => diagnostics.push(Diagnostic::new(
                    DiagnosticCode::DependencyOrderViolation,
                    format!(
                        "task '{}' appears before dependency '{}'",
                        task.task_id, dependency
                    ),
                )),
                None => diagnostics.push(Diagnostic::new(
                    DiagnosticCode::PlanMissingTask,
                    format!(
                        "task '{}' references missing dependency '{}'",
                        task.task_id, dependency
                    ),
                )),
            }
        }
    }

    if diagnostics.is_empty() {
        ValidationResult::valid()
    } else {
        ValidationResult::invalid(diagnostics)
    }
}

fn has_cycle(request: &PlanningRequest) -> bool {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Visiting,
        Visited,
    }

    fn visit(
        task_id: &str,
        graph: &HashMap<String, Vec<String>>,
        marks: &mut HashMap<String, Mark>,
    ) -> bool {
        match marks.get(task_id) {
            Some(Mark::Visiting) => return true,
            Some(Mark::Visited) => return false,
            None => {}
        }
        marks.insert(task_id.to_string(), Mark::Visiting);
        if let Some(next) = graph.get(task_id) {
            for child in next {
                if visit(child, graph, marks) {
                    return true;
                }
            }
        }
        marks.insert(task_id.to_string(), Mark::Visited);
        false
    }

    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    for task in &request.tasks {
        graph.entry(task.id.clone()).or_default();
        for dependency in &task.depends_on {
            graph
                .entry(dependency.clone())
                .or_default()
                .push(task.id.clone());
        }
    }

    let mut marks = HashMap::new();
    request
        .tasks
        .iter()
        .any(|task| visit(&task.id, &graph, &mut marks))
}
