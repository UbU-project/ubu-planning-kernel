//! A response-level invocation wrapper, not a candidate-generating strategy.
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

/// Observations, not permission to install or import a framework. A Python file
/// can be located without running it; its suitability is deliberately unverified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalEnvironment {
    pub python_found: bool,
    pub gpu_stage_implemented: bool,
    pub pytorch_cuda_verified: bool,
}
impl LocalEnvironment {
    pub fn detect() -> Self {
        let python_found = std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|path| path.join("python3").is_file())
        });
        Self {
            python_found,
            gpu_stage_implemented: false,
            pytorch_cuda_verified: false,
        }
    }
    pub fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.python_found {
            missing.push("local Python not found");
        }
        if !self.gpu_stage_implemented {
            missing.push("GPU compute stage not implemented");
        }
        if !self.pytorch_cuda_verified {
            missing.push("PyTorch/CUDA compatibility unverified");
        }
        missing
    }
}
/// All three CPU-owned prerequisites must be true. In this boundary-only ticket
/// the device stage and its compute-budget justification are unavailable.
pub fn gpu_eligible(policy: bool, environment: &LocalEnvironment, budget_justified: bool) -> bool {
    policy
        && environment.python_found
        && environment.gpu_stage_implemented
        && environment.pytorch_cuda_verified
        && budget_justified
}
#[cfg(test)]
mod selection_tests {
    use super::*;
    #[test]
    fn policy_environment_and_budget_are_independent_required_gates() {
        let ready = LocalEnvironment {
            python_found: true,
            gpu_stage_implemented: true,
            pytorch_cuda_verified: true,
        };
        assert!(gpu_eligible(true, &ready, true));
        assert!(!gpu_eligible(false, &ready, true));
        assert!(!gpu_eligible(true, &ready, false));
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
                pytorch_cuda_verified: false,
                ..ready.clone()
            },
        ] {
            assert!(!gpu_eligible(true, &absent, true));
            assert!(!absent.missing().is_empty());
        }
        let actual = LocalEnvironment::detect();
        assert!(!gpu_eligible(true, &actual, true));
    }
}
