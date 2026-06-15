use serde::{Deserialize, Serialize};

use crate::graph::{DependencyEdge, TaskId};
use crate::response::Plan;

pub const PLANNING_SCHEMA_VERSION: &str = "planning-kernel-contract/0.1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TimeWindow {
    pub start: u64,
    pub end: u64,
}

impl TimeWindow {
    pub fn is_possible(&self) -> bool {
        self.start < self.end
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StaticAnchor {
    pub start: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskSpec {
    pub id: TaskId,
    pub duration: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<TaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<TimeWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub static_anchor: Option<StaticAnchor>,
    #[serde(default)]
    pub affect_required: bool,
    #[serde(default)]
    pub affect_current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanningRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<TaskSpec>,
}

impl PlanningRequest {
    pub fn dependency_edges(&self) -> Vec<DependencyEdge> {
        self.tasks
            .iter()
            .flat_map(|task| {
                task.depends_on.iter().map(|dependency| DependencyEdge {
                    before: dependency.clone(),
                    after: task.id.clone(),
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepairRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    pub request_id: String,
    pub candidate: Plan,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<TaskSpec>,
}
