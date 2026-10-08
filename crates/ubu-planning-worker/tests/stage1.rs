use serde_json::Value;
use std::{io, time::Duration};
use ubu_planning_core::{PlannerStrategy, PlanningRequest};
use ubu_planning_cpu::CpuStrategy;
use ubu_planning_worker::{stage1::*, LocalEnvironment};
fn cases() -> Vec<Value> {
    serde_json::from_str(include_str!("../../../fixtures/worker/stage1-goldens.json")).unwrap()
}
fn same(actual: ubu_planning_core::CandidateSet, expected: ubu_planning_core::CandidateSet) {
    assert_eq!(actual.plans, expected.plans);
    assert_eq!(actual.unplaced, expected.unplaced);
    assert_eq!(actual.diagnostics, expected.diagnostics);
}
fn ready() -> LocalEnvironment {
    LocalEnvironment {
        python_found: true,
        gpu_stage_implemented: true,
        torch_importable: true,
        torch_version: Some("2.6.0+cpu".into()),
    }
}
#[test]
fn every_stage_fixture_roundtrips_and_assembles_exact_cpu_candidate_set() {
    for case in cases() {
        let request: PlanningRequest = serde_json::from_value(case["request"].clone()).unwrap();
        let input = StageInput::from_request(&request).unwrap();
        assert_eq!(serde_json::to_value(&input).unwrap(), case["input"]);
        let reply = StageStubTransport.exchange_stage1(&input).unwrap();
        assert_eq!(
            serde_json::to_value(&reply.result).unwrap(),
            case["expected"],
            "{}",
            case["name"]
        );
        same(
            reply.result.assemble(&request).unwrap(),
            CpuStrategy.generate_candidates(&request),
        );
    }
}
#[test]
fn every_structural_class_including_padding_is_exactly_certified() {
    let request: PlanningRequest = serde_json::from_value(cases()[0]["request"].clone()).unwrap();
    let expected = reference_output(&request);
    for field in [
        "task_index",
        "slot_mask",
        "start_time_offsets",
        "duration_samples",
        "piece_index",
        "piece_count",
        "validity_mask",
        "dependency_slack",
        "dependency_feasibility",
        "hard_constraint_feasibility",
        "rejection_codes",
        "omissions",
        "failure",
    ] {
        let mut forged = serde_json::to_value(&expected).unwrap();
        match field {
            "task_index" | "start_time_offsets" | "duration_samples" | "piece_index"
            | "piece_count" => forged[field][0][255] = serde_json::json!(123),
            "slot_mask" => forged[field][0][255] = serde_json::json!(true),
            "validity_mask" | "dependency_feasibility" | "hard_constraint_feasibility" => {
                forged[field][0] = serde_json::json!(false)
            }
            "dependency_slack" => forged[field][0] = serde_json::json!(1),
            "rejection_codes" => forged[field][0] = serde_json::json!("changed"),
            "omissions" => {
                forged[field] = serde_json::json!([{"task_id":"invented-other","reason":"outside_allowed_window"}])
            }
            _ => {
                forged[field] =
                    serde_json::json!({"task_id":null,"reason":"changed","code":"changed"})
            }
        }
        let forged: StageOutput = serde_json::from_value(forged).unwrap();
        assert!(forged.assemble(&request).is_err(), "{field}");
    }
}
struct Failed;
impl StageTransport for Failed {
    fn exchange_stage1(&mut self, _: &StageInput) -> io::Result<StageReply> {
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "synthetic crash or cancellation",
        ))
    }
}
#[test]
fn unavailable_disabled_crashed_and_repair_paths_retain_cpu_provenance() {
    let request: PlanningRequest = serde_json::from_value(cases()[0]["request"].clone()).unwrap();
    let expected = ubu_planning_core::plan(request.clone(), &CpuStrategy);
    for (policy, budget, environment) in [
        (true, true, ready()),
        (false, true, ready()),
        (true, false, ready()),
        (
            true,
            true,
            LocalEnvironment {
                torch_importable: false,
                ..ready()
            },
        ),
    ] {
        let strategy = Stage1Strategy::new(policy, environment, budget, Failed);
        assert_eq!(plan_stage1(request.clone(), &strategy), expected);
    }
    let mut repair = request;
    repair.mode = ubu_planning_core::PlanningMode::Repair;
    assert!(StageInput::from_request(&repair).is_err());
    assert_eq!(
        plan_stage1(
            repair.clone(),
            &Stage1Strategy::new(true, ready(), true, Failed)
        ),
        ubu_planning_core::plan(repair, &CpuStrategy)
    );
}
#[test]
fn canonical_frames_stay_closed_and_internal_frame_bytes_are_shared() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../../fixtures/worker/stage1-frames.json")).unwrap();
    for case in cases {
        let mut bytes = Vec::new();
        ubu_planning_worker_protocol::write_frame(&mut bytes, &case["frame"]).unwrap();
        assert_eq!(
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            case["hex"].as_str().unwrap()
        );
        assert_eq!(
            ubu_planning_worker_protocol::read_frame(&mut std::io::Cursor::new(bytes))
                .unwrap()
                .unwrap(),
            case["frame"]
        );
        assert!(
            serde_json::from_value::<ubu_planning_worker_protocol::PlanningStreamFrame>(
                case["frame"].clone()
            )
            .is_err()
        );
    }
}
#[test]
fn owned_tensor_worker_exact_parity_reuse_and_true_cpu_device_provenance() {
    let python = std::env::var("UBU_WORKER_TEST_PYTHON").unwrap_or_else(|_| "python3".into());
    let environment = LocalEnvironment::detect_with_python(&python);
    if !environment.torch_importable {
        eprintln!("SKIP: pinned CPU-only torch unavailable; real tensor-worker parity unverified");
        return;
    }
    if !ubu_planning_worker::gpu_eligible(true, &environment, true) {
        eprintln!("SKIP: Cargo holds shared lock; run owned Stage 1 suite outside Cargo");
        return;
    }
    let mut transport = LocalStageTransport::new(python, Duration::from_secs(10));
    for case in cases() {
        let request: PlanningRequest = serde_json::from_value(case["request"].clone()).unwrap();
        let reply = transport
            .exchange_stage1(&StageInput::from_request(&request).unwrap())
            .unwrap();
        assert_eq!(
            serde_json::to_value(reply.result).unwrap(),
            case["expected"],
            "{}",
            case["name"]
        );
    }
    let strategy = Stage1Strategy::new(true, environment, true, transport);
    for case in cases().into_iter().take(2) {
        let request: PlanningRequest = serde_json::from_value(case["request"].clone()).unwrap();
        let expected = ubu_planning_core::plan(request.clone(), &CpuStrategy);
        let actual = plan_stage1(request, &strategy);
        let mut normalized = actual.clone();
        normalized.engine_provenance = expected.engine_provenance.clone();
        assert_eq!(normalized, expected);
        let provenance = serde_json::to_value(actual.engine_provenance).unwrap();
        assert_eq!(provenance["backend_kind"], "gpu_worker");
        assert_eq!(provenance["invocation_kind"], "persistent_python_worker");
        assert_eq!(provenance["device_summary"], "cpu");
        assert_eq!(provenance["framework"], "pytorch");
        assert_eq!(provenance["framework_version"], "2.6.0+cpu");
    }
}

// These transports only exchange in-memory frames; no subprocess or device.
struct SyntheticReply {
    mutation: Option<&'static str>,
}
impl StageTransport for SyntheticReply {
    fn owns_compute_lock(&self) -> bool {
        true
    }
    fn exchange_stage1(&mut self, input: &StageInput) -> io::Result<StageReply> {
        if self.mutation == Some("transport") {
            return Err(io::Error::other("invented transport failure"));
        }
        let mut reply = StageStubTransport.exchange_stage1(input)?;
        match self.mutation.take() {
            Some("profile") => reply.profile = "invented-profile".into(),
            Some("identity") => reply.request_id = "invented-request".into(),
            Some("version") => reply.framework_version = "invented-version".into(),
            Some("certification") => reply.result.task_index[0][255] = 0,
            _ => {}
        }
        Ok(reply)
    }
}
#[test]
fn each_policy_environment_gate_names_its_reason_and_preserves_cpu_answer() {
    use Stage1FallbackReason::*;
    let request: PlanningRequest = serde_json::from_value(cases()[0]["request"].clone()).unwrap();
    let expected = ubu_planning_core::plan(request.clone(), &CpuStrategy);
    for (policy, budget, environment, reason) in [
        (false, true, ready(), PolicyDisabled),
        (true, false, ready(), BudgetUnjustified),
        (
            true,
            true,
            LocalEnvironment {
                python_found: false,
                ..ready()
            },
            PythonUnavailable,
        ),
        (
            true,
            true,
            LocalEnvironment {
                gpu_stage_implemented: false,
                ..ready()
            },
            StageUnimplemented,
        ),
        (
            true,
            true,
            LocalEnvironment {
                torch_importable: false,
                ..ready()
            },
            TorchUnavailable,
        ),
    ] {
        let strategy = Stage1Strategy::new(
            policy,
            environment,
            budget,
            SyntheticReply { mutation: None },
        );
        assert_eq!(plan_stage1(request.clone(), &strategy), expected);
        assert_eq!(strategy.fallback_reason(), Some(reason));
    }
}
#[test]
fn occupied_compute_lock_is_named_without_transport_or_wait() {
    // If Cargo or another owned test already holds it, that is the same gate.
    let _held = ubu_planning_worker_protocol::compute_lock::ComputeGuard::try_acquire().ok();
    let request: PlanningRequest = serde_json::from_value(cases()[0]["request"].clone()).unwrap();
    let strategy = Stage1Strategy::new(true, ready(), true, Failed);
    assert_eq!(
        plan_stage1(request.clone(), &strategy),
        ubu_planning_core::plan(request, &CpuStrategy)
    );
    assert_eq!(
        strategy.fallback_reason(),
        Some(Stage1FallbackReason::ComputeLockUnavailable)
    );
}
#[test]
fn unsupported_input_transport_mismatches_and_certification_name_cpu_fallbacks() {
    use Stage1FallbackReason::*;
    let request: PlanningRequest = serde_json::from_value(cases()[0]["request"].clone()).unwrap();
    for (mutation, reason) in [
        ("transport", TransportFailed),
        ("profile", ReplyMismatch),
        ("identity", ReplyMismatch),
        ("version", ReplyMismatch),
        ("certification", CertificationFailed),
    ] {
        let strategy = Stage1Strategy::new(
            true,
            ready(),
            true,
            SyntheticReply {
                mutation: Some(mutation),
            },
        );
        assert_eq!(
            plan_stage1(request.clone(), &strategy),
            ubu_planning_core::plan(request.clone(), &CpuStrategy)
        );
        assert_eq!(strategy.fallback_reason(), Some(reason));
    }
    let mut repair = request;
    repair.mode = ubu_planning_core::PlanningMode::Repair;
    let strategy = Stage1Strategy::new(true, ready(), true, SyntheticReply { mutation: None });
    same(
        strategy.generate_candidates(&repair),
        CpuStrategy.generate_candidates(&repair),
    );
    assert_eq!(strategy.fallback_reason(), Some(InputUnsupported));
}
#[test]
fn reason_resets_after_success_and_stub_never_claims_tensor_provenance() {
    let request: PlanningRequest = serde_json::from_value(cases()[0]["request"].clone()).unwrap();
    let strategy = Stage1Strategy::new(
        true,
        ready(),
        true,
        SyntheticReply {
            mutation: Some("certification"),
        },
    );
    let expected = ubu_planning_core::plan(request.clone(), &CpuStrategy);
    assert_eq!(plan_stage1(request.clone(), &strategy), expected);
    assert_eq!(
        strategy.fallback_reason(),
        Some(Stage1FallbackReason::CertificationFailed)
    );
    assert_eq!(plan_stage1(request, &strategy), expected);
    assert_eq!(strategy.fallback_reason(), None);
    assert_eq!(strategy.framework_version(), None);
}
