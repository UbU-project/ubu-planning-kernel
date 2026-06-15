use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    EmptyRequest,
    DuplicateTaskId,
    MissingDependency,
    CyclicDependency,
    ImpossibleWindow,
    StaleAffect,
    EmptyPlan,
    DuplicatePlanTask,
    PlanMissingTask,
    DependencyOrderViolation,
    StaticAnchorViolation,
    SkeletonFailure,
    MissingSchemaVersion,
    UnknownSchemaVersion,
    NotYetImplemented,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub message: String,
}

impl Diagnostic {
    pub fn new(code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkeletonFailureDiagnostic {
    pub task_id: Option<String>,
    pub reason: String,
}

impl From<SkeletonFailureDiagnostic> for Diagnostic {
    fn from(value: SkeletonFailureDiagnostic) -> Self {
        let subject = value.task_id.map_or_else(
            || "request".to_string(),
            |task_id| format!("task {task_id}"),
        );
        Diagnostic::new(
            DiagnosticCode::SkeletonFailure,
            format!(
                "Could not build deterministic skeleton for {subject}: {}",
                value.reason
            ),
        )
    }
}
