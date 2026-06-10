use serde::{Deserialize, Serialize};

use crate::response::Plan;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplanationFragment {
    pub task_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplanationBundle {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fragments: Vec<ExplanationFragment>,
}

pub fn explain_plan(candidate: &Plan) -> ExplanationBundle {
    let fragments = candidate
        .tasks
        .first()
        .map(|task| ExplanationFragment {
            task_id: task.task_id.clone(),
            text: format!(
                "Task '{}' is selected next because all listed dependencies precede it in the deterministic candidate order.",
                task.task_id
            ),
        })
        .into_iter()
        .collect();

    ExplanationBundle { fragments }
}
