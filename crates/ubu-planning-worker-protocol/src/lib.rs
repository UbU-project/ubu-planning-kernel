//! Local, bounded invocation framing. No planning authority lives here.
pub mod compute_lock;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::io::{self, Read, Write};
pub use ubu_core::worker::{validate_sequence, FrameType, PlanningStreamFrame};
use ubu_planning_core::{PlanningRequest, PlanningResponse};
pub mod session;
pub const MAX_FRAME_BYTES: usize = 1_048_576;

fn invalid(message: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}
pub fn write_frame(writer: &mut impl Write, value: &Value) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(invalid)?;
    if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
        return Err(invalid("invalid frame length"));
    }
    writer.write_all(&(bytes.len() as u32).to_be_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}
pub fn read_frame(reader: &mut impl Read) -> io::Result<Option<Value>> {
    let mut prefix = [0; 4];
    loop {
        match reader.read(&mut prefix[..1]) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    reader.read_exact(&mut prefix[1..])?;
    let size = u32::from_be_bytes(prefix) as usize;
    if size == 0 || size > MAX_FRAME_BYTES {
        return Err(invalid("invalid frame length"));
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map(Some).map_err(invalid)
}

// Convert only typed DTO paths. Arbitrary affect dimension names are untouched.
fn coordinate(item: &mut Value, encode: bool) -> io::Result<()> {
    *item = if encode {
        let seconds = item
            .as_u64()
            .ok_or_else(|| invalid("invalid internal time"))?;
        Value::String(ubu_planning_core::response::utc_timestamp(seconds).map_err(invalid)?)
    } else {
        let text = item
            .as_str()
            .ok_or_else(|| invalid("wire time must be RFC3339 UTC"))?;
        let date =
            time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339)
                .map_err(invalid)?;
        if date.offset() != time::UtcOffset::UTC || date.nanosecond() != 0 {
            return Err(invalid("wire coordinate must be whole UTC seconds"));
        }
        json!(u64::try_from(date.unix_timestamp()).map_err(invalid)?)
    };
    Ok(())
}
fn fields(value: &mut Value, names: &[&str], encode: bool) -> io::Result<()> {
    if let Some(object) = value.as_object_mut() {
        for name in names {
            if let Some(item) = object.get_mut(*name) {
                coordinate(item, encode)?;
            }
        }
    }
    Ok(())
}
fn coordinates(value: &mut Value, encode: bool) -> io::Result<()> {
    if let Some(window) = value.get_mut("time_window") {
        fields(window, &["start", "end"], encode)?;
    }
    if let Some(tasks) = value.get_mut("tasks").and_then(Value::as_array_mut) {
        for task in tasks {
            if let Some(window) = task.get_mut("window") {
                fields(window, &["start", "end"], encode)?;
            }
            if let Some(anchor) = task.get_mut("static_anchor") {
                fields(anchor, &["start"], encode)?;
            }
        }
    }
    if let Some(dimensions) = value
        .pointer_mut("/affect_observation/dimensions")
        .and_then(Value::as_object_mut)
    {
        for dimension in dimensions.values_mut() {
            fields(dimension, &["observed_at"], encode)?;
        }
    }
    if let Some(candidates) = value
        .get_mut("plan_candidates")
        .and_then(Value::as_array_mut)
    {
        for candidate in candidates {
            if let Some(steps) = candidate
                .pointer_mut("/schedule/steps")
                .and_then(Value::as_array_mut)
            {
                for step in steps {
                    fields(step, &["start", "end"], encode)?;
                }
            }
            if let Some(boundaries) = candidate
                .pointer_mut("/coverage/outcome_continuation_summary/boundaries")
                .and_then(Value::as_array_mut)
            {
                for boundary in boundaries {
                    fields(boundary, &["boundary_start"], encode)?;
                }
            }
        }
    }
    Ok(())
}
pub fn to_wire<T: Serialize>(typed: &T) -> io::Result<Value> {
    let mut value = serde_json::to_value(typed).map_err(invalid)?;
    coordinates(&mut value, true)?;
    Ok(value)
}
pub fn from_wire<T: DeserializeOwned>(mut value: Value) -> io::Result<T> {
    coordinates(&mut value, false)?;
    serde_json::from_value(value).map_err(invalid)
}
pub fn request_message(
    request: &PlanningRequest,
    reference: &PlanningResponse,
) -> io::Result<Value> {
    let mut payload = to_wire(request)?;
    // Typed request remains a legacy scoring DTO; these are replay envelope only.
    let fields = payload
        .as_object_mut()
        .ok_or_else(|| invalid("request must be object"))?;
    fields.insert(
        "schema_version".into(),
        json!(ubu_planning_core::PLANNING_SCHEMA_VERSION),
    );
    fields.insert("planner_version".into(), json!(reference.planner_version));
    fields.insert("effective_time".into(), json!(reference.effective_time));
    fields.insert("generated_at".into(), json!(reference.generated_at));
    Ok(json!({"kind":"request", "payload":payload}))
}
pub fn reference_message(reference: &PlanningResponse) -> io::Result<Value> {
    Ok(json!({"kind":"reference", "payload":to_wire(reference)?}))
}
pub fn final_frame(reference: &PlanningResponse) -> io::Result<PlanningStreamFrame> {
    let mut frame = PlanningStreamFrame::outcome(&reference.request_id, FrameType::FinalResponse);
    frame.response = Some(to_wire(reference)?);
    Ok(frame)
}
pub trait PlanningTransport {
    fn exchange(
        &mut self,
        request: &PlanningRequest,
        reference: &PlanningResponse,
    ) -> io::Result<PlanningStreamFrame>;
}
#[derive(Default)]
pub struct StubTransport;
impl PlanningTransport for StubTransport {
    fn exchange(
        &mut self,
        request: &PlanningRequest,
        reference: &PlanningResponse,
    ) -> io::Result<PlanningStreamFrame> {
        let mut input = Vec::new();
        write_frame(&mut input, &request_message(request, reference)?)?;
        write_frame(&mut input, &reference_message(reference)?)?;
        let mut reader = io::Cursor::new(input);
        let request_wire = read_frame(&mut reader)?.ok_or_else(|| invalid("missing request"))?;
        let reply = read_frame(&mut reader)?.ok_or_else(|| invalid("missing reference"))?;
        if request_wire["payload"]["request_id"] != reply["payload"]["request_id"] {
            return Err(invalid("request identity mismatch"));
        }
        let mut frame =
            PlanningStreamFrame::outcome(&reference.request_id, FrameType::FinalResponse);
        frame.response = Some(reply["payload"].clone());
        let mut output = Vec::new();
        write_frame(&mut output, &serde_json::to_value(frame).map_err(invalid)?)?;
        let decoded =
            read_frame(&mut io::Cursor::new(output))?.ok_or_else(|| invalid("missing reply"))?;
        serde_json::from_value(decoded).map_err(invalid)
    }
}
