use serde::{Deserialize, Serialize};

pub type TaskId = String;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyEdge {
    pub before: TaskId,
    pub after: TaskId,
}
