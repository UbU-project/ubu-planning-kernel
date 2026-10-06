use serde_json::json;
use std::{io, path::Path, time::Duration};
use ubu_planning_core::{PlanningRequest, PlanningResponse, ResponseStatus};
use ubu_planning_cpu::CpuStrategy;
use ubu_planning_worker::plan_via_transport;
use ubu_planning_worker_protocol::{session::WorkerSession, *};
fn request() -> PlanningRequest {
    let mut request: PlanningRequest = serde_json::from_str(include_str!(
        "../../../fixtures/planning/valid/simple-success.json"
    ))
    .unwrap();
    request.n_rollouts = 0;
    request
}
fn real(timeout: Duration) -> Option<WorkerSession> {
    let python = std::env::var("UBU_WORKER_TEST_PYTHON").unwrap_or_else(|_| "python3".into());
    match WorkerSession::spawn(&python, timeout) {
        Ok(worker) => Some(worker),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            eprintln!("SKIP: suitable local Python unavailable");
            None
        }
        Err(error) => panic!("owned repository worker failed to start: {error}"),
    }
}
#[test]
fn fixtures_cross_stub_with_complete_cpu_answer_unchanged() {
    for fixture in [
        include_str!("../../../fixtures/planning/valid/simple-success.json"),
        include_str!("../../../fixtures/planning/valid/static-anchor.json"),
        include_str!("../../../fixtures/planning/valid/dependency-chain.json"),
    ] {
        let request: PlanningRequest = serde_json::from_str(fixture).unwrap();
        let expected = ubu_planning_core::plan(request.clone(), &CpuStrategy);
        let actual = plan_via_transport(request, &CpuStrategy, &mut StubTransport);
        assert_eq!(actual.response, expected);
        assert!(actual.transport_outcome.is_none());
        assert_eq!(
            actual
                .response
                .engine_provenance
                .tolerance_profile
                .as_deref(),
            Some("boundary-v1")
        );
    }
}
struct Forged(FrameType);
impl PlanningTransport for Forged {
    fn exchange(
        &mut self,
        _: &PlanningRequest,
        reference: &PlanningResponse,
    ) -> io::Result<PlanningStreamFrame> {
        let mut frame = final_frame(reference)?;
        if self.0 == FrameType::Cancelled {
            return Ok(PlanningStreamFrame::outcome(
                &reference.request_id,
                FrameType::Cancelled,
            ));
        }
        if self.0 == FrameType::ChunkResult {
            frame.frame_type = FrameType::ChunkResult;
            frame.partial_response = frame.response.take();
            frame.chunk_depth = Some(1);
            frame.chunk_id = Some("fixture".into());
        } else {
            frame.response.as_mut().unwrap()["plan_candidates"][0]["schedule"]["steps"][0]["end"] =
                json!("1970-01-01T00:00:00Z");
        }
        Ok(frame)
    }
}
#[test]
fn forged_candidate_and_uncertified_chunk_never_surface_and_cancel_falls_back() {
    let request = request();
    let expected = ubu_planning_core::plan(request.clone(), &CpuStrategy);
    for kind in [
        FrameType::FinalResponse,
        FrameType::ChunkResult,
        FrameType::Cancelled,
    ] {
        let actual = plan_via_transport(request.clone(), &CpuStrategy, &mut Forged(kind));
        assert_eq!(actual.response, expected);
        assert_eq!(actual.transport_status, ResponseStatus::EngineError);
        assert_eq!(
            actual.transport_outcome.unwrap().frame_type,
            if kind == FrameType::Cancelled {
                FrameType::Cancelled
            } else {
                FrameType::EngineError
            }
        );
    }
}
#[test]
fn two_requests_reuse_one_real_process_and_drop_reaps_it() {
    let Some(mut worker) = real(Duration::from_secs(3)) else {
        return;
    };
    let pid = worker.id();
    for id in ["fixture-first", "fixture-second"] {
        let mut request = request();
        request.request_id = id.into();
        let expected = ubu_planning_core::plan(request.clone(), &CpuStrategy);
        let actual = plan_via_transport(request, &CpuStrategy, &mut worker);
        assert_eq!(actual.response, expected);
        assert!(actual.transport_outcome.is_none());
        assert_eq!(worker.id(), pid);
    }
    drop(worker);
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
}
#[test]
fn killed_mid_request_yields_error_frame_and_retained_cpu_answer() {
    let Some(mut worker) = real(Duration::from_secs(3)) else {
        return;
    };
    let request = request();
    let reference = ubu_planning_core::plan(request.clone(), &CpuStrategy);
    let pid = worker.id();
    worker
        .send(&request_message(&request, &reference).unwrap())
        .unwrap();
    worker
        .send(&json!({"kind":"test_wait","payload":{"milliseconds":200}}))
        .unwrap();
    worker.stop();
    let actual = plan_via_transport(request, &CpuStrategy, &mut worker);
    assert_eq!(actual.transport_status, ResponseStatus::EngineError);
    assert_eq!(actual.response, reference);
    assert_eq!(
        actual.transport_outcome.unwrap().frame_type,
        FrameType::EngineError
    );
    drop(worker);
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
}
#[test]
fn real_cancellation_is_transport_only_and_session_can_be_reused() {
    let Some(mut worker) = real(Duration::from_secs(3)) else {
        return;
    };
    let request = request();
    let reference = ubu_planning_core::plan(request.clone(), &CpuStrategy);
    let frame = worker.cancel(&request, &reference).unwrap();
    validate_sequence(std::slice::from_ref(&frame)).unwrap();
    assert_eq!(frame.frame_type, FrameType::Cancelled);
    assert!(frame.response.is_none());
    assert!(plan_via_transport(request, &CpuStrategy, &mut worker)
        .transport_outcome
        .is_none());
}
#[test]
fn owner_reaps_child_on_panic_without_signal_handler() {
    let Some(worker) = real(Duration::from_secs(3)) else {
        return;
    };
    let pid = worker.id();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _owned = worker;
        panic!("synthetic owner failure");
    }));
    assert!(result.is_err());
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
}
#[test]
fn timeout_reaps_owned_child() {
    let Some(mut worker) = real(Duration::from_millis(10)) else {
        return;
    };
    let pid = worker.id();
    worker
        .send(&json!({"kind":"test_wait","payload":{"milliseconds":200}}))
        .unwrap();
    assert_eq!(
        worker.receive().unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    drop(worker);
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
}
#[test]
fn absent_python_is_cleanly_unavailable_without_spawn_fallback() {
    assert_eq!(
        WorkerSession::spawn(
            "/nonexistent-synthetic-worker-python",
            Duration::from_secs(1)
        )
        .err()
        .unwrap()
        .kind(),
        io::ErrorKind::NotFound
    );
}
