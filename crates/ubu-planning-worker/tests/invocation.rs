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

#[test]
fn owned_probe_reports_absence_or_pinned_cpu_framework_and_version() {
    use ubu_planning_worker::LocalEnvironment;
    let absent = LocalEnvironment::detect_with_python("/nonexistent-synthetic-worker-python");
    assert!(!absent.python_found && !absent.torch_importable && absent.torch_version.is_none());
    let python = std::env::var("UBU_WORKER_TEST_PYTHON").unwrap_or_else(|_| "python3".into());
    let environment = LocalEnvironment::detect_with_python(&python);
    if !environment.python_found {
        eprintln!("SKIP: suitable local Python unavailable for environment probe");
    }
    if environment.torch_importable {
        assert_eq!(environment.torch_version.as_deref(), Some("2.6.0+cpu"));
    } else {
        eprintln!("CPU: pinned torch absent, incompatible or broken");
    }
}
#[test]
fn compute_session_try_lock_is_released_on_end_and_panic() {
    use ubu_planning_worker_protocol::compute_lock::ComputeGuard;
    let Ok(guard) = ComputeGuard::try_acquire() else {
        eprintln!("SKIP: Cargo holds shared compute lock; run owned suite outside Cargo to verify release");
        return;
    };
    let ready = ubu_planning_worker::LocalEnvironment {
        python_found: true,
        gpu_stage_implemented: true,
        torch_importable: true,
        torch_version: Some("2.6.0+cpu".into()),
    };
    assert!(!ubu_planning_worker::gpu_eligible(true, &ready, true));
    assert_eq!(
        WorkerSession::spawn_compute("python3", Duration::from_secs(3))
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    let answer = ubu_planning_core::plan(request(), &CpuStrategy);
    assert_eq!(
        serde_json::to_value(answer.engine_provenance).unwrap()["backend_kind"],
        "cpu_reference"
    );
    drop(guard);
    let python = std::env::var("UBU_WORKER_TEST_PYTHON").unwrap_or_else(|_| "python3".into());
    let worker = match WorkerSession::spawn_compute(&python, Duration::from_secs(3)) {
        Ok(worker) => worker,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            eprintln!("SKIP: suitable local Python unavailable");
            return;
        }
        Err(error) => panic!("compute session: {error}"),
    };
    let pid = worker.id();
    assert!(ComputeGuard::try_acquire().is_err());
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _worker = worker;
            panic!("synthetic compute owner panic");
        }))
        .is_err()
    );
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
    drop(ComputeGuard::try_acquire().unwrap());
    drop(WorkerSession::spawn_compute(&python, Duration::from_secs(3)).unwrap());
    drop(ComputeGuard::try_acquire().unwrap());
}

fn quiet_probe(reply: &serde_json::Value) -> Result<(), &'static str> {
    if reply["kind"] != "environment" {
        return Err("wrong owned probe response");
    }
    if reply["payload"]["import_warning_count"].as_u64() != Some(0) {
        return Err("worker import emitted a warning; verify the documented torch/numpy installation with a quiet owned-worker run");
    }
    Ok(())
}

#[test]
fn warning_probe_fails_even_when_framework_import_falls_back() {
    for importable in [true, false] {
        assert!(quiet_probe(&json!({"kind":"environment", "payload":{"importable":importable, "import_warning_count":1}})).is_err());
    }
    assert!(
        quiet_probe(&json!({"kind":"environment", "payload":{"import_warning_count":0}})).is_ok()
    );
}

#[test]
fn owned_worker_framework_import_must_be_quiet() {
    let Some(mut worker) = real(Duration::from_secs(30)) else {
        return;
    };
    worker
        .send(&json!({"kind":"environment", "payload":{}}))
        .unwrap();
    let reply = worker.receive_value().unwrap();
    quiet_probe(&reply).expect("documented worker import must be quiet");
    if reply["payload"]["importable"] != true {
        eprintln!(
            "SKIP: pinned CPU framework unavailable; its installation has not been verified quiet"
        );
    } else {
        assert_eq!(reply["payload"]["version"], "2.6.0+cpu");
    }
}
