pub mod diagnostics;
pub mod explanations;
pub mod graph;
pub mod legitimization;
pub mod request;
pub mod response;
pub mod rollout;
pub mod scoring;
pub mod strategy;
pub mod validation;

pub use diagnostics::{Diagnostic, DiagnosticCode, SkeletonFailureDiagnostic};
pub use explanations::{explain_plan, ExplanationBundle, ExplanationFragment};
pub use graph::{DependencyEdge, TaskId};
pub use request::{
    AffectDirection, AffectLegitimizationMode, AffectObservation, AffectObservationValue,
    AffectProfile, AffectTolerance, CorrelationGroup, DurationModel, PlanningMode, PlanningRequest,
    RepairContext, RepairRequest, RepairScope, ScoringPolicy, StaticAnchor, TaskGraph, TaskSpec,
    TimeWindow, DEFAULT_N_ROLLOUTS, DEFAULT_ROLLOUT_TOP_K, MAX_N_ROLLOUTS, MAX_ROLLOUT_TOP_K,
    PLANNING_SCHEMA_VERSION,
};
pub use response::{
    AffectDimensionLegitimization, CandidateRole, FeasibilitySummary, LegitimizationReport,
    LegitimizationResult, Plan, PlanCandidate, PlanStatus, PlanStep, PlanningResponse,
    ProbabilityInterval, ProbabilityQuality, ProbabilitySummary, RepairResponse,
    RolloutDiagnostics, ScheduledTask, ScoreSummary, SemiLegitimizationResult,
    SemiLegitimizationSummary, ValidationResult,
};
pub use strategy::{CandidateSet, PlannerStrategy};
pub use validation::validate_plan;

fn response_schema_version(schema_version: Option<&str>) -> String {
    match schema_version {
        Some(schema_version) if schema_version == PLANNING_SCHEMA_VERSION => {
            schema_version.to_string()
        }
        _ => PLANNING_SCHEMA_VERSION.to_string(),
    }
}

pub fn plan(request: PlanningRequest, strategy: &impl PlannerStrategy) -> PlanningResponse {
    let request_id = request.request_id.clone();
    let response_schema_version = response_schema_version(request.schema_version.as_deref());
    let request_validation = validation::validate_planning_request(&request);
    if !request_validation.is_valid {
        return PlanningResponse::failure(
            response_schema_version,
            request_id,
            request_validation.diagnostics,
        );
    }

    let candidates = strategy.generate_candidates(&request);
    if candidates.plans.is_empty() {
        return PlanningResponse::failure(
            response_schema_version,
            request_id,
            candidates.diagnostics,
        );
    }

    let mut diagnostics = candidates.diagnostics;
    let mut scoring_inputs = Vec::new();
    for candidate in candidates.plans {
        let validation = validate_plan(&candidate);
        if validation.is_valid {
            let full = legitimization::full_legitimize(
                &candidate,
                request.affect_profile.as_ref(),
                request.affect_observation.as_ref(),
            );
            diagnostics.extend(full.validation.diagnostics.clone());
            if !full.validation.is_valid {
                continue;
            }
            let semi = legitimization::semi_legitimize(&candidate, &request, &full);
            if semi.result == response::SemiLegitimizationResult::RejectObvious {
                continue;
            }
            scoring_inputs.push(scoring::ScoringInput {
                schedule: candidate,
                full_legitimization: full,
                semi_legitimization: semi,
            });
            continue;
        }
        diagnostics.extend(validation.diagnostics);
    }

    let plan_candidates = scoring::score_and_rank(&request, scoring_inputs);
    if plan_candidates.is_empty() {
        PlanningResponse::failure(response_schema_version, request_id, diagnostics)
    } else {
        let plan_candidates = match rollout::rollout_and_rerank(&request, plan_candidates) {
            Ok(result) => {
                diagnostics.extend(result.diagnostics);
                result.candidates
            }
            Err(diagnostic) => {
                diagnostics.push(diagnostic);
                return PlanningResponse::failure(response_schema_version, request_id, diagnostics);
            }
        };
        PlanningResponse::success(
            response_schema_version,
            request_id,
            plan_candidates,
            diagnostics,
        )
    }
}

pub fn repair(request: RepairRequest, strategy: &impl PlannerStrategy) -> RepairResponse {
    let response_schema_version = response_schema_version(request.schema_version.as_deref());
    if let Some(diagnostic) = validation::validate_schema_version(request.schema_version.as_deref())
    {
        return RepairResponse::failure(
            response_schema_version,
            request.request_id,
            vec![diagnostic],
        );
    }

    let validation = validate_plan(&request.candidate);
    if validation.is_valid {
        return RepairResponse::unchanged(
            response_schema_version,
            request.request_id,
            request.candidate,
        );
    }

    let repair_context = request.repair_context.unwrap_or_else(|| RepairContext {
        prior_plan_id: request.candidate.plan_id.clone(),
        last_legitimate_plan_ref: None,
        observed_divergence_refs: Vec::new(),
        repair_scope: RepairScope::RemainingWindow,
    });
    let planning_request = PlanningRequest {
        schema_version: request.schema_version,
        request_id: request.request_id.clone(),
        mode: PlanningMode::Repair,
        rng_seed: request.rng_seed,
        n_rollouts: DEFAULT_N_ROLLOUTS,
        top_k: DEFAULT_ROLLOUT_TOP_K,
        strict_validation: false,
        time_window: request
            .time_window
            .or_else(|| candidate_time_window(&request.candidate)),
        task_graph: TaskGraph {
            tasks: request.tasks,
            topological_order: request.topological_order,
        },
        repair_context: Some(repair_context),
        affect_profile: request.affect_profile,
        affect_observation: request.affect_observation,
        scoring_policy: request::ScoringPolicy::default(),
        prior_plan: Some(request.candidate),
    };
    let response = plan(planning_request, strategy);
    RepairResponse {
        schema_version: response.schema_version,
        request_id: request.request_id,
        repaired_plan: response
            .plan_candidates
            .into_iter()
            .next()
            .map(|candidate| candidate.schedule),
        diagnostics: response.diagnostics,
    }
}

fn candidate_time_window(candidate: &Plan) -> Option<TimeWindow> {
    let start = candidate.steps.iter().map(|step| step.start).min()?;
    let end = candidate.steps.iter().map(|step| step.end).max()?;
    (start < end).then_some(TimeWindow { start, end })
}
