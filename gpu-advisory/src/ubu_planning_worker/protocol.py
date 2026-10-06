"""Bounded length-prefixed JSON, shared with the Rust pure codec."""
import json
import struct

MAX_FRAME_BYTES = 1048576
FRAME_TYPES = {"chunk_result", "final_response", "engine_error", "cancelled"}

def exact(reader, size):
    result = bytearray()
    while len(result) < size:
        part = reader.read(size - len(result))
        if not part:
            raise ValueError("truncated frame")
        result.extend(part)
    return bytes(result)

def read(reader):
    first = reader.read(1)
    if not first:
        return None
    size = struct.unpack(">I", first + exact(reader, 3))[0]
    if not 0 < size <= MAX_FRAME_BYTES:
        raise ValueError("invalid frame length")
    return json.loads(exact(reader, size))

def write(writer, value):
    payload = json.dumps(value, separators=(",", ":"), sort_keys=True, ensure_ascii=False, allow_nan=False).encode("utf-8")
    if not 0 < len(payload) <= MAX_FRAME_BYTES:
        raise ValueError("invalid frame length")
    writer.write(struct.pack(">I", len(payload)) + payload)
    writer.flush()

def validate_frame(frame):
    common = {"schema_version", "request_id", "frame_index", "frame_type"}
    kind = frame.get("frame_type")
    fields = {"chunk_result": {"chunk_depth", "chunk_id", "partial_response"}, "final_response": {"response"}, "engine_error": {"error"}, "cancelled": set()}
    if kind not in FRAME_TYPES or set(frame) != common | fields[kind]:
        raise ValueError("unknown frame_type or invalid payload")
    if frame["schema_version"] != "planning-kernel-contract/0.1" or not isinstance(frame["request_id"], str) or not frame["request_id"]:
        raise ValueError("invalid version or identity")
    if type(frame["frame_index"]) is not int or frame["frame_index"] < 0:
        raise ValueError("invalid frame_index")
    if kind == "chunk_result" and (type(frame["chunk_depth"]) is not int or frame["chunk_depth"] <= 0 or not isinstance(frame["chunk_id"], str) or not frame["chunk_id"] or not isinstance(frame["partial_response"], dict)):
        raise ValueError("invalid chunk")
    if kind == "final_response" and not isinstance(frame["response"], dict):
        raise ValueError("invalid response")
    if kind == "engine_error" and (not isinstance(frame["error"], str) or not frame["error"]):
        raise ValueError("invalid error")

def validate_sequence(frames):
    if not frames or frames[0]["frame_index"] != 0:
        raise ValueError("first frame_index must be zero")
    for index, frame in enumerate(frames):
        validate_frame(frame)
        if frame["request_id"] != frames[0]["request_id"] or (index and frame["frame_index"] <= frames[index-1]["frame_index"]):
            raise ValueError("frame identity or ordering violation")
        if (frame["frame_type"] != "chunk_result") != (index == len(frames)-1):
            raise ValueError("exactly one terminal frame must end the sequence")
