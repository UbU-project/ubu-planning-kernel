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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanningMode {
    #[default]
    FreshGeneration,
    Repair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairScope {
    Local,
    RemainingWindow,
    FullWindow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct TaskGraph {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<TaskSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topological_order: Vec<TaskId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepairContext {
    pub prior_plan_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_legitimate_plan_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed_divergence_refs: Vec<String>,
    pub repair_scope: RepairScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanningRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    pub request_id: String,
    #[serde(default)]
    pub mode: PlanningMode,
    #[serde(default)]
    pub rng_seed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_window: Option<TimeWindow>,
    #[serde(flatten)]
    pub task_graph: TaskGraph,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair_context: Option<RepairContext>,
    #[serde(skip)]
    pub prior_plan: Option<Plan>,
}

impl PlanningRequest {
    pub fn tasks(&self) -> &[TaskSpec] {
        &self.task_graph.tasks
    }

    pub fn topological_order(&self) -> &[TaskId] {
        &self.task_graph.topological_order
    }

    pub fn dependency_edges(&self) -> Vec<DependencyEdge> {
        self.tasks()
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
    #[serde(default)]
    pub rng_seed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_window: Option<TimeWindow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<TaskSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topological_order: Vec<TaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair_context: Option<RepairContext>,
}
