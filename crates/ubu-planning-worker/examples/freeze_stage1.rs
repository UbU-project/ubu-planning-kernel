//! Explicit developer regeneration; no live data, process or framework.
use serde_json::{json, Value};
#[path = "support/week_scale.rs"]
mod week_scale;
use ubu_planning_worker::stage1::*;
fn main() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/worker");
    let mut fixtures: Vec<Value> = serde_json::from_str(include_str!(
        "../../../fixtures/worker/stage1-requests.json"
    ))
    .unwrap();
    fixtures.extend(week_scale::cases());
    for fixture in &mut fixtures {
        let request = serde_json::from_value(fixture["request"].clone()).unwrap();
        fixture["input"] =
            serde_json::to_value(StageInput::from_request(&request).unwrap()).unwrap();
        fixture["expected"] = serde_json::to_value(reference_output(&request)).unwrap();
    }
    std::fs::write(
        root.join("stage1-goldens.json"),
        format!("{}\n", serde_json::to_string(&fixtures).unwrap()),
    )
    .unwrap();
    let input: StageInput = serde_json::from_value(fixtures[0]["input"].clone()).unwrap();
    let reply = StageReply {
        profile: PROFILE.into(),
        request_id: input.request["request_id"].as_str().unwrap().into(),
        framework_version: "2.6.0+cpu".into(),
        result: serde_json::from_value(fixtures[0]["expected"].clone()).unwrap(),
    };
    let cases=[("stage1",input.message()),("stage1_result",reply.message())].into_iter().map(|(name,frame)|{
        let mut bytes=Vec::new();ubu_planning_worker_protocol::write_frame(&mut bytes,&frame).unwrap();
        json!({"name":name,"frame":frame,"hex":bytes.iter().map(|b|format!("{b:02x}")).collect::<String>()})
    }).collect::<Vec<_>>();
    std::fs::write(
        root.join("stage1-frames.json"),
        format!("{}\n", serde_json::to_string(&cases).unwrap()),
    )
    .unwrap();
}
