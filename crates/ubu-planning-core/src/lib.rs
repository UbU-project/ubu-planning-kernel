pub mod diagnostics;
pub mod explanations;
pub mod graph;
pub mod legitimization;
pub mod request;
pub mod response;
pub mod scoring;
pub mod strategy;
pub mod validation;

pub use diagnostics::{Diagnostic, DiagnosticCode, SkeletonFailureDiagnostic};
pub use explanations::{explain_plan, ExplanationBundle, ExplanationFragment};
pub use graph::{DependencyEdge, TaskId};
pub use request::{PlanningRequest, RepairRequest, StaticAnchor, TaskSpec, TimeWindow};
pub use response::{
    Plan, PlanStatus, PlanningResponse, RepairResponse, ScheduledTask, ValidationResult,
};
pub use strategy::{CandidateSet, PlannerStrategy};
pub use validation::validate_plan;

pub fn plan(request: PlanningRequest, strategy: &impl PlannerStrategy) -> PlanningResponse {
    let request_id = request.request_id.clone();
    let request_validation = validation::validate_planning_request(&request);
    if !request_validation.is_valid {
        return PlanningResponse::failure(request_id, request_validation.diagnostics);
    }

    let candidates = strategy.generate_candidates(&request);
    if candidates.plans.is_empty() {
        return PlanningResponse::failure(request_id, candidates.diagnostics);
    }

    let mut diagnostics = candidates.diagnostics;
    for candidate in candidates.plans {
        let validation = validate_plan(&candidate);
        if validation.is_valid {
            let semi = legitimization::semi_legitimize(&candidate);
            diagnostics.extend(semi.diagnostics);
            let full = legitimization::full_legitimize(&candidate);
            diagnostics.extend(full.diagnostics);
            return PlanningResponse::success(request_id, candidate, diagnostics);
        }
        diagnostics.extend(validation.diagnostics);
    }

    PlanningResponse::failure(request_id, diagnostics)
}

pub fn repair(request: RepairRequest, strategy: &impl PlannerStrategy) -> RepairResponse {
    let validation = validate_plan(&request.candidate);
    if validation.is_valid {
        return RepairResponse::unchanged(request.request_id, request.candidate);
    }

    let planning_request = PlanningRequest {
        request_id: request.request_id.clone(),
        tasks: request.tasks,
    };
    let response = plan(planning_request, strategy);
    RepairResponse {
        request_id: request.request_id,
        repaired_plan: response.plan,
        diagnostics: response.diagnostics,
    }
}
