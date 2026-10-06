use serde_json::{json, Value};
use std::io::{self, Cursor, Read};
use ubu_planning_worker_protocol::*;
#[test]
fn shared_python_goldens_are_exact_bytes_in_both_directions() {
    let golden: Value =
        serde_json::from_str(include_str!("../../../fixtures/worker/golden-frames.json")).unwrap();
    for case in golden.as_array().unwrap() {
        let bytes: Vec<u8> = case["hex"]
            .as_str()
            .unwrap()
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect();
        let mut encoded = Vec::new();
        write_frame(&mut encoded, &case["frame"]).unwrap();
        assert_eq!(encoded, bytes, "{}", case["name"]);
        assert_eq!(
            read_frame(&mut Cursor::new(bytes)).unwrap().unwrap(),
            case["frame"]
        );
        let frame: PlanningStreamFrame = serde_json::from_value(case["frame"].clone()).unwrap();
        frame.validate().unwrap();
    }
}
struct Split(Cursor<Vec<u8>>);
impl Read for Split {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let n = buffer.len().min(1);
        self.0.read(&mut buffer[..n])
    }
}
#[test]
fn partial_reads_and_split_frames_preserve_boundaries_and_eof() {
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &json!({"a":1})).unwrap();
    write_frame(&mut bytes, &json!({"b":2})).unwrap();
    let mut split = Split(Cursor::new(bytes));
    assert_eq!(read_frame(&mut split).unwrap(), Some(json!({"a":1})));
    assert_eq!(read_frame(&mut split).unwrap(), Some(json!({"b":2})));
    assert_eq!(read_frame(&mut split).unwrap(), None);
}
#[test]
fn zero_oversize_and_truncated_prefix_or_payload_are_refused() {
    for bytes in [
        vec![0, 0, 0, 0],
        ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes().to_vec(),
        vec![0],
        vec![0, 0, 0],
        vec![0, 0, 0, 2, b'{'],
        vec![0, 0, 0, 1, b'x'],
    ] {
        assert!(read_frame(&mut Cursor::new(bytes)).is_err());
    }
    assert!(write_frame(&mut Vec::new(), &json!("x".repeat(MAX_FRAME_BYTES))).is_err());
}
#[test]
fn sequence_refuses_nonmonotonic_duplicate_terminal_unknown_kind_and_identity() {
    let mut final_frame = PlanningStreamFrame::outcome("fixture", FrameType::FinalResponse);
    final_frame.response = Some(json!({"request_id":"fixture"}));
    validate_sequence(&[final_frame.clone()]).unwrap();
    assert!(validate_sequence(&[final_frame.clone(), final_frame.clone()]).is_err());
    let mut later = final_frame.clone();
    later.frame_index = 1;
    assert!(validate_sequence(&[later.clone()]).is_err());
    later.request_id = "another".into();
    assert!(validate_sequence(&[final_frame.clone(), later]).is_err());
    let mut chunk = PlanningStreamFrame::outcome("fixture", FrameType::ChunkResult);
    chunk.chunk_depth = Some(1);
    chunk.chunk_id = Some("chunk".into());
    chunk.partial_response = Some(json!({}));
    assert!(validate_sequence(&[chunk.clone(), final_frame.clone()]).is_err());
    final_frame.frame_index = 1;
    validate_sequence(&[chunk, final_frame]).unwrap();
    assert!(serde_json::from_value::<PlanningStreamFrame>(json!({"schema_version":"planning-kernel-contract/0.1","request_id":"fixture","frame_index":0,"frame_type":"unknown"})).is_err());
}
#[test]
fn every_typed_coordinate_roundtrips_without_touching_durations_or_dimension_names() {
    let input = json!({"time_window":{"start":1,"end":100},"tasks":[{"window":{"start":2,"end":99},"static_anchor":{"start":3},"duration":{"seconds":20}}],"affect_observation":{"dimensions":{"start":{"value":0.5,"observed_at":4}}},"plan_candidates":[{"schedule":{"steps":[{"start":5,"end":6}]},"coverage":{"outcome_continuation_summary":{"boundaries":[{"boundary_start":7,"boundary_index":1}]}}}]});
    let encoded = to_wire(&input).unwrap();
    assert_eq!(encoded["tasks"][0]["duration"]["seconds"], 20);
    assert_eq!(
        encoded["affect_observation"]["dimensions"]["start"]["observed_at"],
        "1970-01-01T00:00:04Z"
    );
    assert_eq!(
        encoded["plan_candidates"][0]["coverage"]["outcome_continuation_summary"]["boundaries"][0]
            ["boundary_start"],
        "1970-01-01T00:00:07Z"
    );
    assert_eq!(from_wire::<Value>(encoded).unwrap(), input);
    assert!(from_wire::<Value>(json!({"time_window":{"start":1,"end":2}})).is_err());
    assert!(to_wire(&json!({"time_window":{"start":u64::MAX,"end":u64::MAX}})).is_err());
    assert!(from_wire::<Value>(
        json!({"time_window":{"start":"1970-01-01T00:00:01.1Z","end":"1970-01-01T00:00:02Z"}})
    )
    .is_err());
}
