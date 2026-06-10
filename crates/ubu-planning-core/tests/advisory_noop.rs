use std::fs;

use ubu_planning_advisory_protocol::noop::advise_noop;
use ubu_planning_advisory_protocol::{GpuAdvisoryRequest, GpuAdvisoryResponse};

#[test]
fn rust_noop_advisory_uses_canonical_wire_types() {
    let fixture_root = format!("{}/../../fixtures/advisory", env!("CARGO_MANIFEST_DIR"));
    let input = fs::read_to_string(format!("{fixture_root}/noop-request.json")).unwrap();
    let expected = fs::read_to_string(format!("{fixture_root}/noop-response.json")).unwrap();
    let request: GpuAdvisoryRequest = serde_json::from_str(&input).unwrap();
    let response = advise_noop(&request);
    let expected: GpuAdvisoryResponse = serde_json::from_str(&expected).unwrap();

    assert_eq!(response, expected);
}
