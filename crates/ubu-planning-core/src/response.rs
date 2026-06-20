use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::diagnostics::Diagnostic;
use crate::explanations::ExplanationFragment;
use crate::graph::TaskId;
use crate::request::AffectLegitimizationMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Candidate,
    Validated,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanStep {
    pub task_id: TaskId,
    pub start: u64,
    pub end: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<TaskId>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub static_anchor: bool,
}

pub type ScheduledTask = PlanStep;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Plan {
    pub plan_id: String,
    pub status: PlanStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes_plan_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[serde(rename = "steps", alias = "tasks")]
    pub steps: Vec<PlanStep>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ValidationResult {
    pub is_valid: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

impl ValidationResult {
    pub fn valid() -> Self {
        Self {
            is_valid: true,
            diagnostics: Vec::new(),
        }
    }

    pub fn invalid(diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            is_valid: false,
            diagnostics,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanningResponse {
    pub schema_version: String,
    pub request_id: String,
    #[serde(default)]
    pub plan_candidates: Vec<PlanCandidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

impl PlanningResponse {
    pub fn success(
        schema_version: String,
        request_id: String,
        plan_candidates: Vec<PlanCandidate>,
        diagnostics: Vec<Diagnostic>,
    ) -> Self {
        Self {
            schema_version,
            request_id,
            plan_candidates,
            diagnostics,
        }
    }

    pub fn failure(
        schema_version: String,
        request_id: String,
        diagnostics: Vec<Diagnostic>,
    ) -> Self {
        Self {
            schema_version,
            request_id,
            plan_candidates: Vec::new(),
            diagnostics,
        }
    }

    pub fn default_plan(&self) -> Option<&Plan> {
        self.plan_candidates
            .first()
            .map(|candidate| &candidate.schedule)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateRole {
    HighestUtility,
    MostRobust,
    MostScheduleDiverse,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScoreSummary {
    pub utility_score: f64,
    pub robustness_score: f64,
    pub affect_margin_score: f64,
    pub schedule_diversity_score: f64,
    pub total_score: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FeasibilitySummary {
    /// Advisory only: the engine is not a hard-constraint proof system.
    pub hard_constraints_assumed_satisfied_by_engine: bool,
    pub affect_feasible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_affect_score: Option<f64>,
    #[serde(default)]
    pub violated_affect_dimensions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemiLegitimizationResult {
    PassesCheapChecks,
    RejectObvious,
    NeedsFullLegitimization,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SemiLegitimizationSummary {
    pub result: SemiLegitimizationResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affect_budget_ok: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slack_preserved: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_fragility_ok: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_mode_compatible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_repair_viable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legitimacy_delta_estimate: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProbabilityInterval {
    pub lower: f64,
    pub upper: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct ProbabilitySummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_probability: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_probability: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probability_interval: Option<ProbabilityInterval>,
    #[serde(default)]
    pub provenance_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCandidate {
    pub candidate_id: String,
    pub rank: usize,
    pub candidate_role: CandidateRole,
    pub schedule: Plan,
    pub score_summary: ScoreSummary,
    pub feasibility_summary: FeasibilitySummary,
    pub semi_legitimization_summary: SemiLegitimizationSummary,
    pub probability_summary: ProbabilitySummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanation_fragments: Vec<ExplanationFragment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation_hints: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegitimizationResult {
    Passed,
    Failed,
    NeedsClarification,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AffectDimensionLegitimization {
    pub satisfaction: f64,
    pub threshold: f64,
    pub margin: f64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LegitimizationReport {
    pub result: LegitimizationResult,
    pub mode: AffectLegitimizationMode,
    pub affect_feasible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affect_margin: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub violated_dimensions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stale_dimensions: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dimensions: BTreeMap<String, AffectDimensionLegitimization>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepairResponse {
    pub schema_version: String,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repaired_plan: Option<Plan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

impl RepairResponse {
    pub fn unchanged(schema_version: String, request_id: String, candidate: Plan) -> Self {
        Self {
            schema_version,
            request_id,
            repaired_plan: Some(candidate),
            diagnostics: Vec::new(),
        }
    }

    pub fn failure(
        schema_version: String,
        request_id: String,
        diagnostics: Vec<Diagnostic>,
    ) -> Self {
        Self {
            schema_version,
            request_id,
            repaired_plan: None,
            diagnostics,
        }
    }
}
