use std::collections::{HashMap, HashSet};

use crate::diagnostics::{Diagnostic, DiagnosticCode};
use crate::request::{PlanningMode, PlanningRequest, PLANNING_SCHEMA_VERSION};
use crate::response::{Plan, ValidationResult};

pub fn validate_schema_version(schema_version: Option<&str>) -> Option<Diagnostic> {
    match schema_version {
        None => Some(Diagnostic::new(
            DiagnosticCode::MissingSchemaVersion,
            "planning request must include schema_version",
        )),
        Some(PLANNING_SCHEMA_VERSION) => None,
        Some(schema_version) => Some(Diagnostic::new(
            DiagnosticCode::UnknownSchemaVersion,
            format!("unknown planning schema_version '{schema_version}'"),
        )),
    }
}

pub fn validate_planning_request(request: &PlanningRequest) -> ValidationResult {
    let mut diagnostics = Vec::new();
    if let Some(diagnostic) = validate_schema_version(request.schema_version.as_deref()) {
        diagnostics.push(diagnostic);
    }

    if request.tasks().is_empty() {
        diagnostics.push(Diagnostic::new(
            DiagnosticCode::EmptyRequest,
            "planning request must include at least one task",
        ));
    }

    if matches!(request.mode, PlanningMode::Repair) && request.repair_context.is_none() {
        diagnostics.push(Diagnostic::new(
            DiagnosticCode::SkeletonFailure,
            "repair planning request must include repair_context",
        ));
    }

    if let Some(time_window) = &request.time_window {
        if !time_window.is_possible() {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::ImpossibleWindow,
                "planning request time_window must have start before end",
            ));
        }
    }

    let mut ids = HashSet::new();
    for task in request.tasks() {
        if !ids.insert(task.id.clone()) {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::DuplicateTaskId,
                format!("duplicate task id '{}'", task.id),
            ));
        }
        if let Err(reason) = task.validate_contract() {
            diagnostics.push(Diagnostic::new(DiagnosticCode::ImpossibleWindow, reason));
        }
        let duration = task.duration.placement_seconds();
        if let Some(window) = &task.window {
            if !window.is_possible() || window.end.saturating_sub(window.start) < duration {
                diagnostics.push(Diagnostic::new(
                    DiagnosticCode::ImpossibleWindow,
                    format!("task '{}' cannot fit inside its time window", task.id),
                ));
            }
        }
        if let Some(anchor) = &task.static_anchor {
            if let Some(window) = &task.window {
                let Some(anchor_end) = anchor.start.checked_add(duration) else {
                    diagnostics.push(Diagnostic::new(
                        DiagnosticCode::ImpossibleWindow,
                        format!("task '{}' static anchor overflows its duration", task.id),
                    ));
                    continue;
                };
                if anchor.start < window.start || anchor_end > window.end {
                    diagnostics.push(Diagnostic::new(
                        DiagnosticCode::ImpossibleWindow,
                        format!("task '{}' static anchor violates its time window", task.id),
                    ));
                }
            }
        }
    }

    let weights = [
        request.scoring_policy.utility_weight,
        request.scoring_policy.robustness_weight,
        request.scoring_policy.affect_margin_weight,
        request.scoring_policy.schedule_diversity_weight,
    ];
    if weights
        .iter()
        .any(|weight| !weight.is_finite() || *weight < 0.0)
        || weights.iter().all(|weight| *weight == 0.0)
    {
        diagnostics.push(Diagnostic::new(
            DiagnosticCode::ImpossibleWindow,
            "scoring_policy weights must be finite and non-negative, with at least one positive weight",
        ));
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
    if candidate.steps.is_empty() {
        diagnostics.push(Diagnostic::new(
            DiagnosticCode::EmptyPlan,
            "plan must include at least one scheduled task",
        ));
    }

    let mut seen = HashSet::new();
    let mut positions = HashMap::new();
    for (index, task) in candidate.steps.iter().enumerate() {
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

    for (index, task) in candidate.steps.iter().enumerate() {
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
    for task in request.tasks() {
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
        .tasks()
        .iter()
        .any(|task| visit(&task.id, &graph, &mut marks))
}
