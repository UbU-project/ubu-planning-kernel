//! CPU-certified response echo and an optional atomic Stage 1 strategy.
pub mod stage1;
use ubu_planning_core::{PlannerStrategy, PlanningRequest, PlanningResponse, ResponseStatus};
use ubu_planning_worker_protocol::{
    from_wire, validate_sequence, FrameType, PlanningStreamFrame, PlanningTransport,
};

pub struct InvocationResult {
    pub response: PlanningResponse,
    /// Transport outcome is independent of the retained CPU response's status.
    pub transport_status: ResponseStatus,
    pub transport_outcome: Option<PlanningStreamFrame>,
}
pub fn plan_via_transport(
    request: PlanningRequest,
    strategy: &impl PlannerStrategy,
    transport: &mut impl PlanningTransport,
) -> InvocationResult {
    let reference = ubu_planning_core::plan(request.clone(), strategy);
    let exchanged = transport.exchange(&request, &reference);
    let checked = exchanged
        .as_ref()
        .ok()
        .filter(|frame| {
            validate_sequence(std::slice::from_ref(frame)).is_ok()
                && frame.request_id == request.request_id
                && frame.frame_type == FrameType::FinalResponse
                && frame
                    .response
                    .clone()
                    .and_then(|value| from_wire::<PlanningResponse>(value).ok())
                    .is_some_and(|returned| {
                        returned.engine_provenance.validate().is_ok()
                            && returned == reference
                            && returned.plan_candidates.iter().all(|candidate| {
                                ubu_planning_core::validate_plan(&candidate.schedule).is_valid
                            })
                    })
        })
        .is_some();
    if checked {
        return InvocationResult {
            response: reference,
            transport_status: ResponseStatus::Ok,
            transport_outcome: None,
        };
    }
    let outcome = match exchanged {
        Ok(frame)
            if frame.frame_type == FrameType::Cancelled
                && frame.request_id == request.request_id
                && validate_sequence(std::slice::from_ref(&frame)).is_ok() =>
        {
            frame
        }
        _ => {
            let mut frame =
                PlanningStreamFrame::outcome(&request.request_id, FrameType::EngineError);
            frame.error = Some("worker transport or CPU certification failed".into());
            frame
        }
    };
    InvocationResult {
        response: reference,
        transport_status: ResponseStatus::EngineError,
        transport_outcome: Some(outcome),
    }
}

/// Public provenance is closed; the interpreter spelling remains private.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InterpreterSource {
    EnvironmentVariable,
    #[default]
    Python3Fallback,
}
impl InterpreterSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EnvironmentVariable => "UBU_PLANNING_WORKER_PYTHON",
            Self::Python3Fallback => "python3_fallback",
        }
    }
}
/// Failures before tensor computation. No value carries a path or version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeFailure {
    PythonUnavailable,
    InterpreterStartFailed,
    ModuleRootUnavailable,
    ModulePackageUnavailable,
    ProbeBudgetInvalid,
    ProbeTimedOut,
    ProbeFailed,
    TorchUnavailable,
    TorchVersionMismatch,
}
/// A cold import gets the same upper budget as the compute transport. On a
/// loaded machine five seconds can mistake a slow import for a missing install.
/// Operator overrides remain inside WorkerSession's (0, 30s] ceiling.
pub const DEFAULT_PROBE_TIMEOUT_MS: u64 = 30_000;
pub const PROBE_TIMEOUT_VARIABLE: &str = "UBU_PLANNING_WORKER_PROBE_TIMEOUT_MS";
pub fn probe_timeout(value: Option<&str>) -> Result<std::time::Duration, ProbeFailure> {
    let ms = match value {
        None => DEFAULT_PROBE_TIMEOUT_MS,
        Some(value) => value
            .parse::<u64>()
            .map_err(|_| ProbeFailure::ProbeBudgetInvalid)?,
    };
    if ms == 0 || ms > DEFAULT_PROBE_TIMEOUT_MS {
        return Err(ProbeFailure::ProbeBudgetInvalid);
    }
    Ok(std::time::Duration::from_millis(ms))
}
pub fn validate_module_root(root: &std::path::Path) -> Result<(), ProbeFailure> {
    if !root.is_dir() {
        Err(ProbeFailure::ModuleRootUnavailable)
    } else if !root.join("ubu_planning_worker").is_dir() {
        Err(ProbeFailure::ModulePackageUnavailable)
    } else {
        Ok(())
    }
}
fn probe_response_failure(error: std::io::Error) -> ProbeFailure {
    if error.kind() == std::io::ErrorKind::TimedOut {
        ProbeFailure::ProbeTimedOut
    } else {
        ProbeFailure::ProbeFailed
    }
}
/// Verified by a bounded owned interpreter, never imported into Rust.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalEnvironment {
    /// Private command spelling, not an inferred executable path.
    pub interpreter: String,
    pub interpreter_source: InterpreterSource,
    pub probe_failure: Option<ProbeFailure>,
    pub python_found: bool,
    pub gpu_stage_implemented: bool,
    pub torch_importable: bool,
    pub torch_version: Option<String>,
}
impl LocalEnvironment {
    pub fn detect() -> Self {
        let (python, source) = match std::env::var("UBU_PLANNING_WORKER_PYTHON") {
            Ok(python) => (python, InterpreterSource::EnvironmentVariable),
            Err(_) => ("python3".into(), InterpreterSource::Python3Fallback),
        };
        let timeout = std::env::var(PROBE_TIMEOUT_VARIABLE);
        let timeout = match &timeout {
            Ok(value) => Some(value.as_str()),
            Err(std::env::VarError::NotPresent) => None,
            Err(std::env::VarError::NotUnicode(_)) => Some(""),
        };
        Self::detect_with_configuration(&python, source, timeout)
    }
    /// An explicitly selected interpreter; the executable uses detect() so its
    /// source records whether the operator actually supplied the variable.
    pub fn detect_with_python(python: &str) -> Self {
        Self::detect_with_configuration(python, InterpreterSource::EnvironmentVariable, None)
    }
    pub fn detect_with_configuration(
        python: &str,
        source: InterpreterSource,
        timeout: Option<&str>,
    ) -> Self {
        let mut result = Self {
            interpreter: python.into(),
            interpreter_source: source,
            gpu_stage_implemented: true,
            ..Self::default()
        };
        result.probe_failure = result.probe(timeout).err();
        result
    }
    fn probe(&mut self, timeout: Option<&str>) -> Result<(), ProbeFailure> {
        use ubu_planning_worker_protocol::session::{module_root, WorkerSession};
        let timeout = probe_timeout(timeout)?;
        validate_module_root(&module_root())?;
        // A real import in our own child is necessary: metadata may be stale,
        // or a native extension may fail even when its package is present.
        // This is the no-compute session; it never acquires the compute lock.
        let mut session = WorkerSession::spawn(&self.interpreter, timeout).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ProbeFailure::PythonUnavailable
            } else {
                ProbeFailure::InterpreterStartFailed
            }
        })?;
        self.python_found = true;
        session
            .send(&serde_json::json!({"kind":"environment","payload":{}}))
            .map_err(|_| ProbeFailure::ProbeFailed)?;
        let reply = session.receive_value().map_err(probe_response_failure)?;
        self.apply_probe_reply(&reply)
    }
    fn apply_probe_reply(&mut self, reply: &serde_json::Value) -> Result<(), ProbeFailure> {
        if reply["kind"] != "environment" {
            return Err(ProbeFailure::ProbeFailed);
        }
        let payload = &reply["payload"];
        let importable = payload["importable"]
            .as_bool()
            .ok_or(ProbeFailure::ProbeFailed)?;
        self.torch_version = match &payload["version"] {
            serde_json::Value::String(version) => Some(version.clone()),
            serde_json::Value::Null if payload.get("version").is_some() && !importable => None,
            _ => return Err(ProbeFailure::ProbeFailed),
        };
        if self
            .torch_version
            .as_deref()
            .is_some_and(|version| version != "2.6.0+cpu")
        {
            return Err(ProbeFailure::TorchVersionMismatch);
        }
        self.torch_importable = importable;
        if importable {
            Ok(())
        } else {
            Err(ProbeFailure::TorchUnavailable)
        }
    }
    pub fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if let Some(failure) = self.probe_failure {
            missing.push(match failure {
                ProbeFailure::PythonUnavailable => "local Python not found",
                ProbeFailure::InterpreterStartFailed => "local Python could not start",
                ProbeFailure::ModuleRootUnavailable => "worker module root unavailable",
                ProbeFailure::ModulePackageUnavailable => "worker package missing from module root",
                ProbeFailure::ProbeBudgetInvalid => {
                    "probe timeout budget must be an integer in 1..=30000 milliseconds"
                }
                ProbeFailure::ProbeTimedOut => "worker environment probe timed out",
                ProbeFailure::ProbeFailed => {
                    "worker environment probe failed or returned an invalid reply"
                }
                ProbeFailure::TorchUnavailable => "pinned CPU PyTorch unavailable or broken",
                ProbeFailure::TorchVersionMismatch => {
                    "local PyTorch version differs from the pinned version"
                }
            });
        } else if !self.python_found {
            missing.push("local Python not found");
        } else if !self.torch_importable {
            missing.push("pinned CPU PyTorch unavailable or broken");
        }
        if !self.gpu_stage_implemented {
            missing.push("CPU tensor stage not implemented");
        }
        missing
    }
}
fn gates(policy: bool, environment: &LocalEnvironment, budget: bool, lock: bool) -> bool {
    policy
        && environment.probe_failure.is_none()
        && environment.python_found
        && environment.gpu_stage_implemented
        && environment.torch_importable
        && budget
        && lock
}
/// A probe does not reserve the lock: the compute session acquires it again,
/// and races still fall back to CPU without waiting.
pub fn gpu_eligible(policy: bool, environment: &LocalEnvironment, budget_justified: bool) -> bool {
    gates(
        policy,
        environment,
        budget_justified,
        policy
            && budget_justified
            && ubu_planning_worker_protocol::compute_lock::ComputeGuard::try_acquire().is_ok(),
    )
}
#[cfg(test)]
mod selection_tests {
    use super::*;
    #[test]
    fn policy_environment_budget_and_lock_are_independent_required_gates() {
        let ready = LocalEnvironment {
            python_found: true,
            gpu_stage_implemented: true,
            torch_importable: true,
            torch_version: Some("2.6.0+cpu".into()),
            ..LocalEnvironment::default()
        };
        assert!(gates(true, &ready, true, true));
        assert!(!gates(false, &ready, true, true));
        assert!(!gates(true, &ready, false, true));
        assert!(!gates(true, &ready, true, false));
        for absent in [
            LocalEnvironment {
                python_found: false,
                ..ready.clone()
            },
            LocalEnvironment {
                gpu_stage_implemented: false,
                ..ready.clone()
            },
            LocalEnvironment {
                torch_importable: false,
                ..ready.clone()
            },
        ] {
            assert!(!gates(true, &absent, true, true));
            assert!(!absent.missing().is_empty());
        }
    }
}

#[cfg(test)]
mod probe_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn probe_budget_is_bounded_and_invalid_input_never_reaches_an_interpreter() {
        assert_eq!(probe_timeout(None).unwrap().as_millis(), 30_000);
        for ms in ["1", "30000"] {
            assert!(probe_timeout(Some(ms)).is_ok());
        }
        for ms in [
            "",
            "0",
            "30001",
            "-1",
            "1.5",
            "unexpected",
            "18446744073709551616",
        ] {
            let environment = LocalEnvironment::detect_with_configuration(
                "synthetic-unused-interpreter",
                InterpreterSource::EnvironmentVariable,
                Some(ms),
            );
            assert_eq!(
                environment.probe_failure,
                Some(ProbeFailure::ProbeBudgetInvalid)
            );
            assert!(!environment.python_found);
            assert_eq!(environment.interpreter, "synthetic-unused-interpreter");
            assert_eq!(
                environment.interpreter_source,
                InterpreterSource::EnvironmentVariable
            );
            assert_eq!(environment.missing().len(), 1);
        }
    }
    #[test]
    fn directory_with_legacy_package_is_not_the_worker_package() {
        let root = std::env::temp_dir().join(format!(
            "ubu-synthetic-module-layout-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert_eq!(
            validate_module_root(&root),
            Err(ProbeFailure::ModuleRootUnavailable)
        );
        std::fs::create_dir_all(root.join("ubu_gpu_advisory")).unwrap();
        assert_eq!(
            validate_module_root(&root),
            Err(ProbeFailure::ModulePackageUnavailable)
        );
        std::fs::create_dir(root.join("ubu_planning_worker")).unwrap();
        assert_eq!(validate_module_root(&root), Ok(()));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn probe_reply_distinguishes_absence_mismatch_and_broken_pinned_import() {
        for (importable, version, failure) in [
            (false, None, Some(ProbeFailure::TorchUnavailable)),
            (
                false,
                Some("synthetic-wrong-version"),
                Some(ProbeFailure::TorchVersionMismatch),
            ),
            (
                true,
                Some("synthetic-wrong-version"),
                Some(ProbeFailure::TorchVersionMismatch),
            ),
            (
                false,
                Some("2.6.0+cpu"),
                Some(ProbeFailure::TorchUnavailable),
            ),
            (true, Some("2.6.0+cpu"), None),
        ] {
            let mut environment = LocalEnvironment::default();
            let result = environment.apply_probe_reply(&json!({"kind":"environment", "payload":{"importable":importable,"version":version}}));
            assert_eq!(result.err(), failure);
            assert_eq!(environment.torch_importable, failure.is_none());
            assert_eq!(environment.torch_version.as_deref(), version);
        }
    }
    #[test]
    fn malformed_probe_is_not_a_missing_torch_install() {
        for reply in [
            json!({}),
            json!({"kind":"other"}),
            json!({"kind":"environment","payload":{"importable":true}}),
            json!({"kind":"environment","payload":{"importable":"true","version":"2.6.0+cpu"}}),
            json!({"kind":"environment","payload":{"importable":true,"version":null}}),
        ] {
            assert_eq!(
                LocalEnvironment::default().apply_probe_reply(&reply),
                Err(ProbeFailure::ProbeFailed)
            );
        }
    }
    #[test]
    fn response_timeout_is_distinct_from_eof_and_invalid_frames() {
        use std::io::{Error, ErrorKind};
        assert_eq!(
            probe_response_failure(Error::from(ErrorKind::TimedOut)),
            ProbeFailure::ProbeTimedOut
        );
        for kind in [
            ErrorKind::UnexpectedEof,
            ErrorKind::InvalidData,
            ErrorKind::BrokenPipe,
        ] {
            assert_eq!(
                probe_response_failure(Error::from(kind)),
                ProbeFailure::ProbeFailed
            );
        }
    }
    #[test]
    fn timeout_missing_text_does_not_claim_python_or_torch_absence() {
        let environment = LocalEnvironment {
            probe_failure: Some(ProbeFailure::ProbeTimedOut),
            gpu_stage_implemented: true,
            ..Default::default()
        };
        assert_eq!(
            environment.missing(),
            ["worker environment probe timed out"]
        );
        assert!(!gates(true, &environment, true, true));
    }
}
