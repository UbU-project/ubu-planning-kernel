use serde::{Deserialize, Serialize};

use crate::diagnostics::Diagnostic;
use crate::graph::TaskId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Candidate,
    Validated,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScheduledTask {
    pub task_id: TaskId,
    pub start: u64,
    pub end: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<TaskId>,
    #[serde(default)]
    pub static_anchor: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Plan {
    pub plan_id: String,
    pub status: PlanStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<ScheduledTask>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanningResponse {
    pub schema_version: String,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<Plan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

impl PlanningResponse {
    pub fn success(
        schema_version: String,
        request_id: String,
        plan: Plan,
        diagnostics: Vec<Diagnostic>,
    ) -> Self {
        Self {
            schema_version,
            request_id,
            plan: Some(plan),
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
            plan: None,
            diagnostics,
        }
    }
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
