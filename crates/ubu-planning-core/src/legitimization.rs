use std::collections::BTreeMap;

use crate::diagnostics::{Diagnostic, DiagnosticCode};
use crate::request::{
    AffectDirection, AffectLegitimizationMode, AffectObservation, AffectProfile, AffectTolerance,
    PlanningRequest,
};
use crate::response::{
    AffectDimensionLegitimization, LegitimizationReport, LegitimizationResult, Plan,
    SemiLegitimizationResult, SemiLegitimizationSummary, ValidationResult,
};

pub fn semi_legitimize(
    candidate: &Plan,
    request: &PlanningRequest,
    full: &FullLegitimization,
) -> SemiLegitimizationSummary {
    // Preserve Phase B warn-only behavior: affect_budget_ok reflects whether the
    // existing full_legitimize filter admits the candidate, not raw feasibility.
    let affect_budget_ok = full.validation.is_valid;
    let slack_preserved = preserves_slack(candidate, request);
    let result = if !affect_budget_ok || !slack_preserved {
        SemiLegitimizationResult::RejectObvious
    } else if full.report.result == LegitimizationResult::NeedsClarification {
        SemiLegitimizationResult::NeedsFullLegitimization
    } else {
        SemiLegitimizationResult::PassesCheapChecks
    };

    SemiLegitimizationSummary {
        result,
        affect_budget_ok: Some(affect_budget_ok),
        slack_preserved: Some(slack_preserved),
        // TODO(P7/C-1): implement dependency-fragility, user-mode compatibility,
        // local-repair viability, and legitimacy-delta estimate heuristics.
        dependency_fragility_ok: None,
        user_mode_compatible: None,
        local_repair_viable: None,
        legitimacy_delta_estimate: None,
    }
}

fn preserves_slack(candidate: &Plan, request: &PlanningRequest) -> bool {
    let by_id: BTreeMap<_, _> = candidate
        .steps
        .iter()
        .map(|step| (step.task_id.as_str(), step))
        .collect();

    candidate.steps.iter().all(|step| {
        let Some(task) = request.tasks().iter().find(|task| task.id == step.task_id) else {
            return false;
        };
        let plan_window_ok = request.time_window.as_ref().is_none_or(|window| {
            if request.mode == crate::request::PlanningMode::Repair {
                // Repair skeletons deliberately retain completed and in-progress
                // history before the active window.
                step.end <= window.end || step.start < window.start
            } else {
                step.start >= window.start && step.end <= window.end
            }
        });
        let task_window_ok = task
            .window
            .as_ref()
            .is_none_or(|window| step.start >= window.start && step.end <= window.end);
        let anchor_ok = task
            .static_anchor
            .as_ref()
            .is_none_or(|anchor| step.start == anchor.start);
        let dependencies_ok = task.depends_on.iter().all(|dependency| {
            by_id
                .get(dependency.as_str())
                .is_some_and(|dependency_step| dependency_step.end <= step.start)
        });
        plan_window_ok && task_window_ok && anchor_ok && dependencies_ok
    }) && candidate.steps.iter().enumerate().all(|(index, left)| {
        candidate
            .steps
            .iter()
            .skip(index + 1)
            .all(|right| left.end <= right.start || right.end <= left.start)
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct FullLegitimization {
    pub validation: ValidationResult,
    pub report: LegitimizationReport,
}

pub fn full_legitimize(
    candidate: &Plan,
    profile: Option<&AffectProfile>,
    observation: Option<&AffectObservation>,
) -> FullLegitimization {
    let Some(profile) = profile else {
        return passed_without_affect_profile();
    };

    if profile.dimensions.is_empty() {
        return passed_without_affect_profile();
    }

    let evaluation_point = aggregate_evaluation_point(candidate);
    let mut diagnostics = Vec::new();
    let mut dimensions = BTreeMap::new();
    let mut stale_dimensions = Vec::new();
    let mut violated_dimensions = Vec::new();
    let mut affect_margin: Option<f64> = None;
    let mut needs_clarification = evaluation_point.is_none() || observation.is_none();

    let observation_dimensions = observation.map(|observation| &observation.dimensions);

    for (dimension, tolerance) in &profile.dimensions {
        let Some(observation_value) =
            observation_dimensions.and_then(|dimensions| dimensions.get(dimension))
        else {
            needs_clarification = true;
            continue;
        };

        let Some(satisfaction) = satisfaction(tolerance, observation_value.value) else {
            needs_clarification = true;
            continue;
        };

        let margin = satisfaction - tolerance.threshold;
        affect_margin = Some(match affect_margin {
            Some(current) => current.min(margin),
            None => margin,
        });

        let stale = evaluation_point
            .zip(tolerance.freshness_seconds)
            .is_some_and(|(evaluation_point, freshness_seconds)| {
                evaluation_point.saturating_sub(observation_value.observed_at) > freshness_seconds
            });
        if stale {
            stale_dimensions.push(dimension.clone());
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::StaleAffect,
                format!("affect observation for '{dimension}' is stale"),
            ));
        }

        if margin < 0.0 {
            violated_dimensions.push(dimension.clone());
        }

        dimensions.insert(
            dimension.clone(),
            AffectDimensionLegitimization {
                satisfaction,
                threshold: tolerance.threshold,
                margin,
                stale,
            },
        );
    }

    let affect_feasible = !needs_clarification && violated_dimensions.is_empty();
    let result = if needs_clarification {
        LegitimizationResult::NeedsClarification
    } else if affect_feasible {
        LegitimizationResult::Passed
    } else {
        LegitimizationResult::Failed
    };
    let is_valid = match profile.mode {
        AffectLegitimizationMode::Enforce => result == LegitimizationResult::Passed,
        AffectLegitimizationMode::WarnOnly => true,
    };
    let validation = if is_valid {
        ValidationResult {
            is_valid: true,
            diagnostics,
        }
    } else {
        ValidationResult {
            is_valid: false,
            diagnostics,
        }
    };

    FullLegitimization {
        validation,
        report: LegitimizationReport {
            result,
            mode: profile.mode,
            affect_feasible,
            affect_margin,
            violated_dimensions,
            stale_dimensions,
            dimensions,
        },
    }
}

fn passed_without_affect_profile() -> FullLegitimization {
    FullLegitimization {
        validation: ValidationResult::valid(),
        report: LegitimizationReport {
            result: LegitimizationResult::Passed,
            mode: AffectLegitimizationMode::Enforce,
            affect_feasible: true,
            affect_margin: None,
            violated_dimensions: Vec::new(),
            stale_dimensions: Vec::new(),
            dimensions: BTreeMap::new(),
        },
    }
}

fn aggregate_evaluation_point(candidate: &Plan) -> Option<u64> {
    candidate.steps.iter().map(|step| step.start).min()
}

pub fn satisfaction(tolerance: &AffectTolerance, value: f64) -> Option<f64> {
    if !value.is_finite()
        || !tolerance.location.is_finite()
        || !tolerance.scale.is_finite()
        || !tolerance.threshold.is_finite()
        || tolerance.scale <= 0.0
        || !(0.0..=1.0).contains(&tolerance.threshold)
    {
        return None;
    }

    let z = match tolerance.direction {
        AffectDirection::HigherIsBetter => (value - tolerance.location) / tolerance.scale,
        AffectDirection::LowerIsBetter => (tolerance.location - value) / tolerance.scale,
    };
    Some(sigmoid(z))
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}
