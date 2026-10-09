//! Approved internal Stage 1 envelopes; not new canonical stream frame kinds.
use crate::{gpu_eligible, LocalEnvironment, ProbeFailure};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    io,
    time::Duration,
};
use ubu_planning_core::{
    CandidateSet, Plan, PlanStep, PlannerStrategy, PlanningRequest, PlanningResponse,
};
use ubu_planning_cpu::CpuStrategy;
use ubu_planning_worker_protocol::{from_wire, session::WorkerSession, to_wire};
pub const PROFILE: &str = "stage1-atomic-v1";
pub const MAX_PLANNING_TASKS: usize = 256;
pub const MAX_CANDIDATES: usize = 16;
fn invalid(message: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StageInput {
    pub profile: String,
    pub request: Value,
    pub topological_order: Vec<String>,
    pub task_validity_mask: Vec<bool>,
    pub sampling: Value,
}
impl StageInput {
    pub fn from_request(request: &PlanningRequest) -> io::Result<Self> {
        if request.tasks().len() > MAX_PLANNING_TASKS
            || request.mode != ubu_planning_core::PlanningMode::FreshGeneration
        {
            return Err(invalid("unsupported Task count or repair"));
        }
        let mut durations = vec![0u64; MAX_PLANNING_TASKS];
        let mut mask = vec![false; MAX_PLANNING_TASKS];
        for (i, task) in request.tasks().iter().enumerate() {
            let duration = task.duration.placement_seconds();
            if duration > i64::MAX as u64 {
                return Err(invalid("duration exceeds integer tensor range"));
            }
            durations[i] = duration;
            mask[i] = true;
        }
        let mut wire = to_wire(request)?;
        // No affect state or other-stage scoring inputs cross this stage seam.
        let fields = wire
            .as_object_mut()
            .ok_or_else(|| invalid("invalid request"))?;
        fields.retain(|key, _| {
            matches!(
                key.as_str(),
                "schema_version"
                    | "request_id"
                    | "mode"
                    | "rng_seed"
                    | "time_window"
                    | "tasks"
                    | "topological_order"
            )
        });
        let mut order = request.topological_order().to_vec();
        if order.is_empty() {
            let mut remaining: BTreeMap<_, BTreeSet<_>> = request
                .tasks()
                .iter()
                .map(|t| (t.id.clone(), t.depends_on.iter().cloned().collect()))
                .collect();
            while !remaining.is_empty() {
                // Same lexicographic Kahn tie-breaking as the CPU reference.
                let Some(key) = remaining
                    .iter()
                    .find(|(_, deps)| deps.is_empty())
                    .map(|(key, _)| key.clone())
                else {
                    break;
                };
                remaining.remove(&key);
                for deps in remaining.values_mut() {
                    deps.remove(&key);
                }
                order.push(key);
            }
        }
        Ok(Self {
            profile: PROFILE.into(),
            request: wire,
            topological_order: order,
            task_validity_mask: mask,
            sampling: json!({"kind":"placement_seconds","duration_samples":durations}),
        })
    }
    pub fn message(&self) -> Value {
        json!({"kind":"stage1", "payload":self})
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageFailure {
    pub task_id: Option<String>,
    pub reason: String,
    pub code: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Omission {
    pub task_id: String,
    pub reason: ubu_planning_core::UnplacedReason,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageOutput {
    pub task_index: Vec<Vec<i64>>,
    pub slot_mask: Vec<Vec<bool>>,
    pub start_time_offsets: Vec<Vec<i64>>,
    pub duration_samples: Vec<Vec<i64>>,
    pub piece_index: Vec<Vec<i64>>,
    pub piece_count: Vec<Vec<i64>>,
    pub validity_mask: Vec<bool>,
    pub dependency_slack: Vec<i64>,
    pub dependency_feasibility: Vec<bool>,
    pub hard_constraint_feasibility: Vec<bool>,
    pub rejection_codes: Vec<String>,
    pub omissions: Vec<Omission>,
    pub failure: Option<StageFailure>,
}
/// Closed public field identities, in StageOutput declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertificationField {
    TaskIndex,
    SlotMask,
    StartTimeOffsets,
    DurationSamples,
    PieceIndex,
    PieceCount,
    ValidityMask,
    DependencySlack,
    DependencyFeasibility,
    HardConstraintFeasibility,
    RejectionCodes,
    Omissions,
    Failure,
}
impl CertificationField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TaskIndex => "task_index",
            Self::SlotMask => "slot_mask",
            Self::StartTimeOffsets => "start_time_offsets",
            Self::DurationSamples => "duration_samples",
            Self::PieceIndex => "piece_index",
            Self::PieceCount => "piece_count",
            Self::ValidityMask => "validity_mask",
            Self::DependencySlack => "dependency_slack",
            Self::DependencyFeasibility => "dependency_feasibility",
            Self::HardConstraintFeasibility => "hard_constraint_feasibility",
            Self::RejectionCodes => "rejection_codes",
            Self::Omissions => "omissions",
            Self::Failure => "failure",
        }
    }
}
/// Formatting is public metadata only. Differing values are explicitly private,
/// never included by Debug, Display or the io::Error that owns this explanation.
#[derive(Clone)]
pub struct CertificationDifference {
    pub field: CertificationField,
    pub candidate_index: Option<usize>,
    pub slot_index: Option<usize>,
    pub diverging_fields: usize,
    expected: Value,
    actual: Value,
}
impl CertificationDifference {
    pub fn public_metadata(&self) -> Value {
        json!({"field":self.field.as_str(),"candidate_index":self.candidate_index,
            "slot_index":self.slot_index,"diverging_fields":self.diverging_fields})
    }
    /// Only the private diagnostic screen may consume this payload.
    pub fn private_values(&self) -> Value {
        json!({"expected":self.expected,"actual":self.actual})
    }
}
impl std::fmt::Debug for CertificationDifference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CertificationDifference")
            .field("field", &self.field)
            .field("candidate_index", &self.candidate_index)
            .field("slot_index", &self.slot_index)
            .field("diverging_fields", &self.diverging_fields)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Display for CertificationDifference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Stage 1 exact CPU certification failed: {}",
            self.public_metadata()
        )
    }
}
impl std::error::Error for CertificationDifference {}
fn first_difference<T: PartialEq>(actual: &[T], expected: &[T]) -> usize {
    actual
        .iter()
        .zip(expected)
        .position(|(a, e)| a != e)
        .unwrap_or(actual.len().min(expected.len()))
}
fn private_value<T: Serialize>(value: Option<&T>) -> Value {
    value.map_or(Value::Null, |v| {
        serde_json::to_value(v).expect("StageOutput values serialize")
    })
}
fn matrix_difference<T: PartialEq + Serialize>(
    actual: &[Vec<T>],
    expected: &[Vec<T>],
) -> (Option<usize>, Option<usize>, Value, Value) {
    let candidate = first_difference(actual, expected);
    match (actual.get(candidate), expected.get(candidate)) {
        (Some(a), Some(e)) => {
            let slot = first_difference(a, e);
            (
                Some(candidate),
                Some(slot),
                private_value(a.get(slot)),
                private_value(e.get(slot)),
            )
        }
        (a, e) => (
            Some(candidate),
            None,
            a.map_or(Value::Null, |v| json!({"row_length":v.len()})),
            e.map_or(Value::Null, |v| json!({"row_length":v.len()})),
        ),
    }
}
fn vector_difference<T: PartialEq + Serialize>(
    actual: &[T],
    expected: &[T],
    candidate_field: bool,
) -> (Option<usize>, Option<usize>, Value, Value) {
    let index = first_difference(actual, expected);
    (
        candidate_field.then_some(index),
        None,
        private_value(actual.get(index)),
        private_value(expected.get(index)),
    )
}
impl StageOutput {
    /// Exactly the same thirteen typed PartialEq checks as derived StageOutput
    /// equality, including lengths, order and every padded element. Count fields,
    /// not elements; locate the first differing candidate/slot only afterwards.
    pub fn certification_difference(&self, reference: &Self) -> Option<CertificationDifference> {
        let mut fields = Vec::new();
        macro_rules! check {
            ($name:ident,$field:ident) => {
                if self.$name != reference.$name {
                    fields.push(CertificationField::$field);
                }
            };
        }
        check!(task_index, TaskIndex);
        check!(slot_mask, SlotMask);
        check!(start_time_offsets, StartTimeOffsets);
        check!(duration_samples, DurationSamples);
        check!(piece_index, PieceIndex);
        check!(piece_count, PieceCount);
        check!(validity_mask, ValidityMask);
        check!(dependency_slack, DependencySlack);
        check!(dependency_feasibility, DependencyFeasibility);
        check!(hard_constraint_feasibility, HardConstraintFeasibility);
        check!(rejection_codes, RejectionCodes);
        check!(omissions, Omissions);
        check!(failure, Failure);
        let field = *fields.first()?;
        let (candidate_index, slot_index, actual, expected) = match field {
            CertificationField::TaskIndex => {
                matrix_difference(&self.task_index, &reference.task_index)
            }
            CertificationField::SlotMask => {
                matrix_difference(&self.slot_mask, &reference.slot_mask)
            }
            CertificationField::StartTimeOffsets => {
                matrix_difference(&self.start_time_offsets, &reference.start_time_offsets)
            }
            CertificationField::DurationSamples => {
                matrix_difference(&self.duration_samples, &reference.duration_samples)
            }
            CertificationField::PieceIndex => {
                matrix_difference(&self.piece_index, &reference.piece_index)
            }
            CertificationField::PieceCount => {
                matrix_difference(&self.piece_count, &reference.piece_count)
            }
            CertificationField::ValidityMask => {
                vector_difference(&self.validity_mask, &reference.validity_mask, true)
            }
            CertificationField::DependencySlack => {
                vector_difference(&self.dependency_slack, &reference.dependency_slack, true)
            }
            CertificationField::DependencyFeasibility => vector_difference(
                &self.dependency_feasibility,
                &reference.dependency_feasibility,
                true,
            ),
            CertificationField::HardConstraintFeasibility => vector_difference(
                &self.hard_constraint_feasibility,
                &reference.hard_constraint_feasibility,
                true,
            ),
            CertificationField::RejectionCodes => {
                vector_difference(&self.rejection_codes, &reference.rejection_codes, true)
            }
            CertificationField::Omissions => {
                vector_difference(&self.omissions, &reference.omissions, false)
            }
            CertificationField::Failure => (
                None,
                None,
                private_value(self.failure.as_ref()),
                private_value(reference.failure.as_ref()),
            ),
        };
        Some(CertificationDifference {
            field,
            candidate_index,
            slot_index,
            diverging_fields: fields.len(),
            expected,
            actual,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageReply {
    pub profile: String,
    pub request_id: String,
    pub framework_version: String,
    pub result: StageOutput,
}
impl StageReply {
    pub fn message(&self) -> Value {
        json!({"kind":"stage1_result","payload":self})
    }
}
/// Semantic CPU-only golden, independent of any process or framework.
pub fn reference_output(request: &PlanningRequest) -> StageOutput {
    let candidates = CpuStrategy.generate_candidates(request);
    let slots = |value| vec![vec![value; MAX_PLANNING_TASKS]; MAX_CANDIDATES];
    let mut output = StageOutput {
        task_index: slots(-1),
        slot_mask: vec![vec![false; MAX_PLANNING_TASKS]; MAX_CANDIDATES],
        start_time_offsets: slots(0),
        duration_samples: slots(0),
        piece_index: slots(0),
        piece_count: slots(0),
        validity_mask: vec![false; MAX_CANDIDATES],
        dependency_slack: vec![0; MAX_CANDIDATES],
        dependency_feasibility: vec![false; MAX_CANDIDATES],
        hard_constraint_feasibility: vec![false; MAX_CANDIDATES],
        rejection_codes: vec!["padding".into(); MAX_CANDIDATES],
        omissions: candidates
            .unplaced
            .iter()
            .map(|u| Omission {
                task_id: u.task_ref.clone(),
                reason: u.reason,
            })
            .collect(),
        failure: None,
    };
    if candidates.plans.is_empty() {
        if let Err(error) = ubu_planning_cpu::skeleton::build_skeleton(request) {
            let code = if error.reason
                == "dependency graph did not produce a complete deterministic order"
            {
                "dependency_cycle"
            } else {
                "skeleton_failure"
            };
            output.rejection_codes.fill(code.into());
            output.failure = Some(StageFailure {
                task_id: error.task_id,
                reason: error.reason,
                code: code.into(),
            });
        }
    }
    let start = request.time_window.as_ref().map_or(0, |w| w.start);
    for (c, plan) in candidates.plans.iter().enumerate().take(MAX_CANDIDATES) {
        output.validity_mask[c] = true;
        output.rejection_codes[c] = "ok".into();
        let mut slack = None;
        for (s, step) in plan.steps.iter().enumerate().take(MAX_PLANNING_TASKS) {
            output.task_index[c][s] = request
                .tasks()
                .iter()
                .position(|t| t.id == step.task_id)
                .unwrap() as i64;
            output.slot_mask[c][s] = true;
            output.start_time_offsets[c][s] = (step.start as i128 - start as i128) as i64;
            output.duration_samples[c][s] = (step.end - step.start) as i64;
            output.piece_index[c][s] = 1;
            output.piece_count[c][s] = 1;
            for dep in &step.depends_on {
                if let Some(before) = plan.steps.iter().find(|step| &step.task_id == dep) {
                    let margin = (step.start as i128 - before.end as i128) as i64;
                    slack = Some(slack.map_or(margin, |old: i64| old.min(margin)));
                }
            }
        }
        output.dependency_slack[c] = slack.unwrap_or(0);
        output.dependency_feasibility[c] = plan.steps.iter().all(|step| {
            step.depends_on.iter().all(|id| {
                plan.steps
                    .iter()
                    .any(|before| &before.task_id == id && before.end <= step.start)
            })
        });
        output.hard_constraint_feasibility[c] = ubu_planning_core::validate_plan(plan).is_valid
            && output.dependency_feasibility[c]
            && plan.steps.iter().all(|step| {
                let task = request
                    .tasks()
                    .iter()
                    .find(|task| task.id == step.task_id)
                    .unwrap();
                request
                    .time_window
                    .as_ref()
                    .is_none_or(|w| step.start >= w.start && step.end <= w.end)
                    && task
                        .window
                        .as_ref()
                        .is_none_or(|w| step.start >= w.start && step.end <= w.end)
                    && task
                        .static_anchor
                        .as_ref()
                        .is_none_or(|a| step.start == a.start)
            });
    }
    output
}
impl StageOutput {
    pub fn assemble(&self, request: &PlanningRequest) -> io::Result<CandidateSet> {
        // Exact comparison covers every padded value, code, mask and omission,
        // not only the final schedule. Never widen a numeric tolerance here.
        if let Some(difference) = self.certification_difference(&reference_output(request)) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, difference));
        }
        let mut plans = Vec::new();
        let window_start = request.time_window.as_ref().map_or(0, |w| w.start);
        for c in 0..MAX_CANDIDATES {
            if !self.validity_mask[c] {
                continue;
            }
            let mut steps = Vec::new();
            for s in 0..MAX_PLANNING_TASKS {
                if !self.slot_mask[c][s] {
                    continue;
                }
                let task = request
                    .tasks()
                    .get(self.task_index[c][s] as usize)
                    .ok_or_else(|| invalid("invalid Task slot"))?;
                let start = (window_start as i128 + self.start_time_offsets[c][s] as i128)
                    .try_into()
                    .map_err(invalid)?;
                let end = u64::checked_add(start, self.duration_samples[c][s] as u64)
                    .ok_or_else(|| invalid("placement overflow"))?;
                steps.push(PlanStep {
                    task_id: task.id.clone(),
                    start,
                    end,
                    depends_on: task.depends_on.clone(),
                    static_anchor: task.static_anchor.is_some(),
                });
            }
            let base = format!("plan-{}-{:016x}", request.request_id, request.rng_seed);
            plans.push(Plan {
                plan_id: if c == 0 {
                    base
                } else {
                    format!("{base}-c{c:02}")
                },
                status: ubu_planning_core::response::PlanStatus::Candidate,
                supersedes_plan_id: None,
                steps,
            });
        }
        let omitted: BTreeMap<_, _> = self
            .omissions
            .iter()
            .filter(|o| o.reason != ubu_planning_core::UnplacedReason::DeferredDependency)
            .map(|o| (o.task_id.clone(), o.reason))
            .collect();
        let excluded: BTreeSet<_> = self.omissions.iter().map(|o| o.task_id.clone()).collect();
        let dependents = ubu_planning_cpu::protection::dependent_index(request);
        let mut unplaced: Vec<_> = request
            .tasks()
            .iter()
            .filter(|task| excluded.contains(&task.id))
            .map(|task| {
                let reason = omitted
                    .get(&task.id)
                    .copied()
                    .unwrap_or(ubu_planning_core::UnplacedReason::DeferredDependency);
                let deferred = if reason == ubu_planning_core::UnplacedReason::DeferredDependency {
                    task.depends_on
                        .iter()
                        .filter(|id| excluded.contains(*id))
                        .cloned()
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect()
                } else {
                    Vec::new()
                };
                let mut entry = ubu_planning_core::UnplacedTask::new(
                    ubu_planning_cpu::protection::selection_rank(task),
                    reason,
                    Vec::new(),
                    deferred,
                );
                entry.affected_dependent_task_refs = ubu_planning_cpu::protection::dependents_of(
                    &dependents,
                    &BTreeSet::from([task.id.clone()]),
                )
                .intersection(&excluded)
                .cloned()
                .collect();
                entry
            })
            .collect();
        ubu_planning_core::unplaced::sort_report(&mut unplaced);
        let diagnostics = self
            .failure
            .as_ref()
            .map(|failure| {
                ubu_planning_core::diagnostics::SkeletonFailureDiagnostic {
                    task_id: failure.task_id.clone(),
                    reason: failure.reason.clone(),
                }
                .into()
            })
            .into_iter()
            .collect();
        Ok(CandidateSet {
            plans,
            unplaced,
            diagnostics,
        })
    }
}
pub trait StageTransport {
    fn exchange_stage1(&mut self, input: &StageInput) -> io::Result<StageReply>;
    fn owns_compute_lock(&self) -> bool {
        false
    }
    fn runs_tensor_worker(&self) -> bool {
        false
    }
}
pub struct LocalStageTransport {
    python: String,
    timeout: Duration,
    session: Option<WorkerSession>,
}
impl LocalStageTransport {
    pub fn new(python: impl Into<String>, timeout: Duration) -> Self {
        Self {
            python: python.into(),
            timeout,
            session: None,
        }
    }
}
impl StageTransport for LocalStageTransport {
    fn owns_compute_lock(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(WorkerSession::owns_compute_lock)
    }
    fn runs_tensor_worker(&self) -> bool {
        true
    }
    fn exchange_stage1(&mut self, input: &StageInput) -> io::Result<StageReply> {
        let result = (|| {
            if self.session.is_none() {
                self.session = Some(WorkerSession::spawn_compute(&self.python, self.timeout)?);
            }
            let session = self.session.as_mut().unwrap();
            session.send(&input.message())?;
            let reply = session.receive_value()?;
            if reply["kind"] != "stage1_result" {
                return Err(invalid("Stage 1 error or cancellation"));
            }
            serde_json::from_value(reply["payload"].clone()).map_err(invalid)
        })();
        if result.is_err() {
            self.session.take();
        }
        result
    }
}
/// Closed, content-free explanation of an unchanged CPU fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage1FallbackReason {
    PolicyDisabled,
    BudgetUnjustified,
    PythonUnavailable,
    StageUnimplemented,
    TorchUnavailable,
    InterpreterStartFailed,
    ModuleRootUnavailable,
    ModulePackageUnavailable,
    ProbeBudgetInvalid,
    ProbeTimedOut,
    ProbeFailed,
    TorchVersionMismatch,
    ComputeLockUnavailable,
    InputUnsupported,
    TransportFailed,
    ReplyMismatch,
    CertificationFailed,
}
impl Stage1FallbackReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PolicyDisabled => "policy_disabled",
            Self::BudgetUnjustified => "budget_unjustified",
            Self::PythonUnavailable => "python_unavailable",
            Self::StageUnimplemented => "stage_unimplemented",
            Self::TorchUnavailable => "torch_unavailable",
            Self::InterpreterStartFailed => "interpreter_start_failed",
            Self::ModuleRootUnavailable => "module_root_unavailable",
            Self::ModulePackageUnavailable => "module_package_unavailable",
            Self::ProbeBudgetInvalid => "probe_budget_invalid",
            Self::ProbeTimedOut => "probe_timed_out",
            Self::ProbeFailed => "probe_failed",
            Self::TorchVersionMismatch => "torch_version_mismatch",
            Self::ComputeLockUnavailable => "compute_lock_unavailable",
            Self::InputUnsupported => "input_unsupported",
            Self::TransportFailed => "transport_failed",
            Self::ReplyMismatch => "reply_mismatch",
            Self::CertificationFailed => "certification_failed",
        }
    }
}
/// RefCell implements the existing shared-reference strategy trait; no CPU seam
/// or planner type is refactored. This strategy is local to one planning owner.
pub struct Stage1Strategy<T> {
    transport: RefCell<T>,
    environment: LocalEnvironment,
    policy: bool,
    budget: bool,
    version: RefCell<Option<String>>,
    fallback_reason: Cell<Option<Stage1FallbackReason>>,
    certification_difference: RefCell<Option<CertificationDifference>>,
}
impl<T: StageTransport> Stage1Strategy<T> {
    pub fn new(policy: bool, environment: LocalEnvironment, budget: bool, transport: T) -> Self {
        Self {
            transport: RefCell::new(transport),
            environment,
            policy,
            budget,
            version: RefCell::new(None),
            fallback_reason: Cell::new(None),
            certification_difference: RefCell::new(None),
        }
    }
    /// The latest generation's reason alongside its candidates, reset each call.
    pub fn fallback_reason(&self) -> Option<Stage1FallbackReason> {
        self.fallback_reason.get()
    }
    pub fn certification_difference(&self) -> Option<CertificationDifference> {
        self.certification_difference.borrow().clone()
    }
    pub fn framework_version(&self) -> Option<String> {
        self.version.borrow().clone()
    }
}
impl<T: StageTransport> PlannerStrategy for Stage1Strategy<T> {
    fn generate_candidates(&self, request: &PlanningRequest) -> CandidateSet {
        self.version.borrow_mut().take();
        self.fallback_reason.set(None);
        self.certification_difference.borrow_mut().take();
        let fallback = |reason| {
            self.fallback_reason.set(Some(reason));
            CpuStrategy.generate_candidates(request)
        };
        use Stage1FallbackReason::*;
        if !self.policy {
            return fallback(PolicyDisabled);
        }
        if !self.budget {
            return fallback(BudgetUnjustified);
        }
        if let Some(failure) = self.environment.probe_failure {
            return fallback(match failure {
                ProbeFailure::PythonUnavailable => PythonUnavailable,
                ProbeFailure::InterpreterStartFailed => InterpreterStartFailed,
                ProbeFailure::ModuleRootUnavailable => ModuleRootUnavailable,
                ProbeFailure::ModulePackageUnavailable => ModulePackageUnavailable,
                ProbeFailure::ProbeBudgetInvalid => ProbeBudgetInvalid,
                ProbeFailure::ProbeTimedOut => ProbeTimedOut,
                ProbeFailure::ProbeFailed => ProbeFailed,
                ProbeFailure::TorchUnavailable => TorchUnavailable,
                ProbeFailure::TorchVersionMismatch => TorchVersionMismatch,
            });
        }
        if !self.environment.python_found {
            return fallback(PythonUnavailable);
        }
        if !self.environment.gpu_stage_implemented {
            return fallback(StageUnimplemented);
        }
        if !self.environment.torch_importable {
            return fallback(TorchUnavailable);
        }
        if !(self.transport.borrow().owns_compute_lock()
            || gpu_eligible(self.policy, &self.environment, self.budget))
        {
            return fallback(ComputeLockUnavailable);
        }
        let Ok(input) = StageInput::from_request(request) else {
            return fallback(InputUnsupported);
        };
        let Ok(reply) = self.transport.borrow_mut().exchange_stage1(&input) else {
            return fallback(TransportFailed);
        };
        if reply.profile != PROFILE
            || reply.request_id != request.request_id
            || reply.framework_version != "2.6.0+cpu"
        {
            return fallback(ReplyMismatch);
        }
        let candidates = match reply.result.assemble(request) {
            Ok(candidates) => candidates,
            Err(error) => {
                *self.certification_difference.borrow_mut() = error
                    .get_ref()
                    .and_then(|error| error.downcast_ref::<CertificationDifference>())
                    .cloned();
                return fallback(CertificationFailed);
            }
        };
        if self.transport.borrow().runs_tensor_worker() {
            *self.version.borrow_mut() = Some(reply.framework_version);
        }
        candidates
    }
}
pub fn plan_stage1<T: StageTransport>(
    request: PlanningRequest,
    strategy: &Stage1Strategy<T>,
) -> PlanningResponse {
    strategy.version.borrow_mut().take();
    strategy.fallback_reason.set(None);
    strategy.certification_difference.borrow_mut().take();
    let mut response = ubu_planning_core::plan(request, strategy);
    if let Some(version) = strategy.framework_version() {
        response.engine_provenance.backend_kind = ubu_core::worker::BackendKind::GpuWorker;
        response.engine_provenance.invocation_kind =
            ubu_core::worker::InvocationKind::PersistentPythonWorker;
        response.engine_provenance.framework = Some("pytorch".into());
        response.engine_provenance.framework_version = Some(version);
        response.engine_provenance.device_summary = Some("cpu".into());
        response.engine_provenance.tolerance_profile = Some(PROFILE.into());
    }
    response
}
/// Stub uses the same bounded codec, never a subprocess or an advisory model.
#[derive(Default)]
pub struct StageStubTransport;
impl StageTransport for StageStubTransport {
    fn exchange_stage1(&mut self, input: &StageInput) -> io::Result<StageReply> {
        let mut bytes = Vec::new();
        ubu_planning_worker_protocol::write_frame(&mut bytes, &input.message())?;
        let message = ubu_planning_worker_protocol::read_frame(&mut std::io::Cursor::new(bytes))?
            .ok_or_else(|| invalid("missing stage message"))?;
        let decoded: StageInput =
            serde_json::from_value(message["payload"].clone()).map_err(invalid)?;
        let request: PlanningRequest = from_wire(decoded.request)?;
        let reply = StageReply {
            profile: PROFILE.into(),
            request_id: request.request_id.clone(),
            framework_version: "2.6.0+cpu".into(),
            result: reference_output(&request),
        };
        let mut bytes = Vec::new();
        ubu_planning_worker_protocol::write_frame(&mut bytes, &reply.message())?;
        let message = ubu_planning_worker_protocol::read_frame(&mut std::io::Cursor::new(bytes))?
            .ok_or_else(|| invalid("missing stage reply"))?;
        serde_json::from_value(message["payload"].clone()).map_err(invalid)
    }
}

#[cfg(test)]
mod owned_failure_tests {
    use super::*;
    #[test]
    fn killed_mid_stage1_retains_cpu_answer_and_releases_owned_lock() {
        let python = std::env::var("UBU_WORKER_TEST_PYTHON").unwrap_or_else(|_| "python3".into());
        let mut session = match WorkerSession::spawn_compute(&python, Duration::from_secs(3)) {
            Ok(session) => session,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::WouldBlock
                ) =>
            {
                eprintln!("SKIP: local Python unavailable or Cargo holds compute lock; run owned suite outside Cargo");
                return;
            }
            Err(error) => panic!("owned session: {error}"),
        };
        let request: PlanningRequest = serde_json::from_str(include_str!(
            "../../../fixtures/planning/valid/simple-success.json"
        ))
        .unwrap();
        let expected = ubu_planning_core::plan(request.clone(), &CpuStrategy);
        let pid = session.id();
        session
            .send(&StageInput::from_request(&request).unwrap().message())
            .unwrap();
        session.stop();
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
        let transport = LocalStageTransport {
            python,
            timeout: Duration::from_secs(3),
            session: Some(session),
        };
        let environment = LocalEnvironment {
            python_found: true,
            gpu_stage_implemented: true,
            torch_importable: true,
            torch_version: Some("2.6.0+cpu".into()),
            ..LocalEnvironment::default()
        };
        let strategy = Stage1Strategy::new(true, environment, true, transport);
        assert_eq!(plan_stage1(request, &strategy), expected);
        assert!(strategy.framework_version().is_none());
        drop(strategy);
        drop(ubu_planning_worker_protocol::compute_lock::ComputeGuard::try_acquire().unwrap());
    }
}
