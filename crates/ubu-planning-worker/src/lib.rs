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

/// Verified by a bounded owned interpreter, never imported into Rust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalEnvironment {
    pub python_found: bool,
    pub gpu_stage_implemented: bool,
    pub torch_importable: bool,
    pub torch_version: Option<String>,
}
impl LocalEnvironment {
    pub fn detect() -> Self {
        Self::detect_with_python("python3")
    }
    pub fn detect_with_python(python: &str) -> Self {
        let mut result = Self {
            python_found: false,
            gpu_stage_implemented: true,
            torch_importable: false,
            torch_version: None,
        };
        // A real import in our own child is necessary: metadata may be stale,
        // or a native extension may fail even when its package is present.
        if let Ok(mut session) = ubu_planning_worker_protocol::session::WorkerSession::spawn(
            python,
            std::time::Duration::from_secs(5),
        ) {
            result.python_found = true;
            if session
                .send(&serde_json::json!({"kind":"environment","payload":{}}))
                .is_ok()
            {
                if let Ok(value) = session.receive_value() {
                    if value["kind"] == "environment" {
                        result.torch_importable = value["payload"]["importable"] == true;
                        result.torch_version =
                            value["payload"]["version"].as_str().map(str::to_owned);
                    }
                }
            }
        }
        result
    }
    pub fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.python_found {
            missing.push("local Python not found");
        }
        if !self.gpu_stage_implemented {
            missing.push("CPU tensor stage not implemented");
        }
        if !self.torch_importable {
            missing.push("pinned CPU PyTorch unavailable or broken");
        }
        missing
    }
}
fn gates(policy: bool, environment: &LocalEnvironment, budget: bool, lock: bool) -> bool {
    policy
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
