use std::collections::BTreeMap;

use serde::{de, Deserialize, Deserializer, Serialize};

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

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskSpec {
    pub id: TaskId,
    pub duration: DurationModel,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub correlation_groups: Vec<CorrelationGroup>,
    #[serde(default = "default_task_value")]
    pub value: f64,
    #[serde(default = "default_task_priority")]
    pub priority: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<TaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<TimeWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub static_anchor: Option<StaticAnchor>,
}

fn default_task_value() -> f64 {
    1.0
}

fn default_task_priority() -> f64 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DurationModel {
    Fixed {
        seconds: u64,
    },
    ShiftedLognormalP95 {
        min_seconds: u64,
        mode_seconds: u64,
        p95_seconds: u64,
    },
}

impl DurationModel {
    /// Deterministic placement duration for C-1. C-2 will sample the model.
    pub fn placement_seconds(&self) -> u64 {
        match self {
            Self::Fixed { seconds } => *seconds,
            Self::ShiftedLognormalP95 { mode_seconds, .. } => *mode_seconds,
        }
    }

    pub fn relative_spread(&self) -> f64 {
        match self {
            Self::Fixed { .. } => 0.0,
            Self::ShiftedLognormalP95 {
                min_seconds,
                p95_seconds,
                ..
            } => (*p95_seconds - *min_seconds) as f64 / *p95_seconds as f64,
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Fixed { seconds: 0 } => {
                Err("fixed duration seconds must be a positive integer".to_string())
            }
            Self::Fixed { .. } => Ok(()),
            Self::ShiftedLognormalP95 {
                min_seconds,
                mode_seconds,
                p95_seconds,
            } if !(*min_seconds < *mode_seconds && *mode_seconds < *p95_seconds) => Err(format!(
                "shifted_lognormal_p95 duration must satisfy 0 <= min_seconds < mode_seconds < p95_seconds; got {min_seconds}, {mode_seconds}, {p95_seconds} seconds"
            )),
            Self::ShiftedLognormalP95 { .. } => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CorrelationGroup {
    pub group: String,
    pub strength: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
struct TaskSpecWire {
    id: TaskId,
    duration: DurationModel,
    #[serde(default)]
    correlation_groups: Vec<CorrelationGroup>,
    #[serde(default = "default_task_value")]
    value: f64,
    #[serde(default = "default_task_priority")]
    priority: f64,
    #[serde(default)]
    depends_on: Vec<TaskId>,
    #[serde(default)]
    window: Option<TimeWindow>,
    #[serde(default)]
    static_anchor: Option<StaticAnchor>,
}

impl<'de> Deserialize<'de> for TaskSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = TaskSpecWire::deserialize(deserializer)?;
        let task = Self {
            id: wire.id,
            duration: wire.duration,
            correlation_groups: wire.correlation_groups,
            value: wire.value,
            priority: wire.priority,
            depends_on: wire.depends_on,
            window: wire.window,
            static_anchor: wire.static_anchor,
        };
        task.validate_contract().map_err(de::Error::custom)?;
        Ok(task)
    }
}

impl TaskSpec {
    pub fn new(id: TaskId, duration: DurationModel) -> Result<Self, String> {
        let task = Self {
            id,
            duration,
            correlation_groups: Vec::new(),
            value: default_task_value(),
            priority: default_task_priority(),
            depends_on: Vec::new(),
            window: None,
            static_anchor: None,
        };
        task.validate_contract()?;
        Ok(task)
    }

    pub fn validate_contract(&self) -> Result<(), String> {
        self.duration
            .validate()
            .map_err(|reason| format!("task '{}': {reason}", self.id))?;

        let mut groups = std::collections::BTreeSet::new();
        for correlation in &self.correlation_groups {
            if correlation.group.is_empty() {
                return Err(format!(
                    "task '{}': correlation group name must not be empty",
                    self.id
                ));
            }
            if !correlation.strength.is_finite() || !(0.0..=1.0).contains(&correlation.strength) {
                return Err(format!(
                    "task '{}': correlation group '{}' strength must be finite and in [0,1]",
                    self.id, correlation.group
                ));
            }
            if !groups.insert(correlation.group.as_str()) {
                return Err(format!(
                    "task '{}': duplicate correlation group '{}'",
                    self.id, correlation.group
                ));
            }
        }
        if !self.value.is_finite() || self.value < 0.0 {
            return Err(format!(
                "task '{}': value must be finite and non-negative",
                self.id
            ));
        }
        if !self.priority.is_finite() || self.priority < 0.0 {
            return Err(format!(
                "task '{}': priority must be finite and non-negative",
                self.id
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AffectLegitimizationMode {
    #[default]
    Enforce,
    WarnOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AffectDirection {
    HigherIsBetter,
    LowerIsBetter,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AffectTolerance {
    pub direction: AffectDirection,
    pub location: f64,
    pub scale: f64,
    pub threshold: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct AffectProfile {
    #[serde(default)]
    pub mode: AffectLegitimizationMode,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dimensions: BTreeMap<String, AffectTolerance>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AffectObservationValue {
    pub value: f64,
    pub observed_at: u64,
    pub source_kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct AffectObservation {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dimensions: BTreeMap<String, AffectObservationValue>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affect_profile: Option<AffectProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affect_observation: Option<AffectObservation>,
    #[serde(default)]
    pub scoring_policy: ScoringPolicy,
    #[serde(skip)]
    pub prior_plan: Option<Plan>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScoringPolicy {
    pub utility_weight: f64,
    pub robustness_weight: f64,
    pub affect_margin_weight: f64,
    pub schedule_diversity_weight: f64,
}

impl Default for ScoringPolicy {
    fn default() -> Self {
        Self {
            utility_weight: 1.0,
            robustness_weight: 1.0,
            affect_margin_weight: 1.0,
            schedule_diversity_weight: 1.0,
        }
    }
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affect_profile: Option<AffectProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affect_observation: Option<AffectObservation>,
}
