//! Optional work omitted from an otherwise usable Plan (UBU-D0289).
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnplacedReason {
    InsufficientTotalCapacity,
    NoEligibleChunkLargeEnough,
    OutsideAllowedWindow,
    OmittedLowerValue,
    DeferredDependency,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HorizonExtension {
    NotApplicable,
    SkippedByPolicy,
    AttemptedExhausted,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeAlternativeAction {
    DecomposeTask,
    ReprioritizeTask,
    ExtendPlanningHorizon,
    RelaxTaskWindow,
    RemoveOrMootTask,
    ManualDecision,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeAlternative {
    pub action: SafeAlternativeAction,
    pub label: String,
    pub requires_user_input: bool,
    pub resulting_change_summary: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkRef {
    pub index: usize,
    pub start: u64,
    pub end: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectionRank {
    pub value: f64,
    pub deadline_or_latest_finish: Option<u64>,
    pub task_id: String,
}
impl SelectionRank {
    /// Most protected first; absent deadlines are least protected.
    pub fn compare_protection(&self, other: &Self) -> Ordering {
        other
            .value
            .total_cmp(&self.value)
            .then_with(|| {
                match (
                    self.deadline_or_latest_finish,
                    other.deadline_or_latest_finish,
                ) {
                    (Some(a), Some(b)) => a.cmp(&b),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => Ordering::Equal,
                }
            })
            .then(self.task_id.cmp(&other.task_id))
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnplacedTask {
    pub diagnostic_id: String,
    pub task_ref: String,
    pub reason: UnplacedReason,
    pub deferred_by_task_refs: Vec<String>,
    pub affected_dependent_task_refs: Vec<String>,
    pub eligible_chunk_refs: Vec<ChunkRef>,
    pub selection_rank: SelectionRank,
    pub horizon_extension: HorizonExtension,
    pub safe_alternatives: Vec<SafeAlternative>,
    pub user_facing_summary: String,
}
impl UnplacedTask {
    pub fn new(
        selection_rank: SelectionRank,
        reason: UnplacedReason,
        eligible_chunk_refs: Vec<ChunkRef>,
        deferred_by_task_refs: Vec<String>,
    ) -> Self {
        let task_ref = selection_rank.task_id.clone();
        let cause = match reason {
            UnplacedReason::InsufficientTotalCapacity => "required work uses the available time",
            UnplacedReason::NoEligibleChunkLargeEnough => "no free interval is long enough",
            UnplacedReason::OutsideAllowedWindow => {
                "its allowed time range cannot hold it after its prerequisites"
            }
            UnplacedReason::OmittedLowerValue => "other optional work uses the available time",
            UnplacedReason::DeferredDependency => "a prerequisite was left out",
        };
        Self {
            diagnostic_id: format!("unplaced-{task_ref}"),
            user_facing_summary: format!("Task `{task_ref}` was left out because {cause}."),
            task_ref,
            reason,
            deferred_by_task_refs,
            affected_dependent_task_refs: Vec::new(),
            eligible_chunk_refs,
            selection_rank,
            horizon_extension: horizon_extension(reason),
            safe_alternatives: safe_alternatives(reason),
        }
    }
}
fn horizon_extension(reason: UnplacedReason) -> HorizonExtension {
    match reason {
        UnplacedReason::InsufficientTotalCapacity
        | UnplacedReason::NoEligibleChunkLargeEnough
        | UnplacedReason::OutsideAllowedWindow => HorizonExtension::SkippedByPolicy,
        _ => HorizonExtension::NotApplicable,
    }
}
pub fn safe_alternatives(reason: UnplacedReason) -> Vec<SafeAlternative> {
    use SafeAlternativeAction::*;
    let choices = match reason {
        UnplacedReason::NoEligibleChunkLargeEnough => vec![DecomposeTask, ExtendPlanningHorizon],
        UnplacedReason::OutsideAllowedWindow => vec![RelaxTaskWindow, ExtendPlanningHorizon],
        UnplacedReason::InsufficientTotalCapacity => vec![ExtendPlanningHorizon, DecomposeTask],
        UnplacedReason::OmittedLowerValue => vec![ReprioritizeTask, RemoveOrMootTask],
        UnplacedReason::DeferredDependency => vec![ManualDecision, ReprioritizeTask],
    };
    choices
        .into_iter()
        .map(|action| {
            let (label, change) = match action {
                DecomposeTask => (
                    "Break up the task",
                    "Model smaller tasks before planning again.",
                ),
                ReprioritizeTask => (
                    "Review task priority",
                    "Review the task's value and plan again.",
                ),
                ExtendPlanningHorizon => {
                    ("Plan a longer period", "Request a longer planning horizon.")
                }
                RelaxTaskWindow => (
                    "Review the allowed time range",
                    "Widen the task's allowed time range.",
                ),
                RemoveOrMootTask => (
                    "Review whether this task is needed",
                    "Remove or moot the task if it is no longer needed.",
                ),
                ManualDecision => (
                    "Review the prerequisite",
                    "Decide how to handle the missing prerequisite.",
                ),
            };
            SafeAlternative {
                action,
                label: label.into(),
                requires_user_input: true,
                resulting_change_summary: change.into(),
            }
        })
        .collect()
}
pub fn sort_report(report: &mut [UnplacedTask]) {
    report.sort_by(|a, b| b.selection_rank.compare_protection(&a.selection_rank));
}
/// Less means the left set gives up less protected work.
pub fn compare_omissions(left: &[SelectionRank], right: &[SelectionRank]) -> Ordering {
    let mut left: Vec<_> = left.iter().collect();
    let mut right: Vec<_> = right.iter().collect();
    left.sort_by(|a, b| a.compare_protection(b));
    right.sort_by(|a, b| a.compare_protection(b));
    for (a, b) in left.iter().zip(&right) {
        let order = b.compare_protection(a);
        if order != Ordering::Equal {
            return order;
        }
    }
    left.len().cmp(&right.len())
}
