import io
import json
from pathlib import Path
import pytest
from ubu_planning_worker import protocol
from ubu_planning_worker.main import run

GOLDENS = json.loads((Path(__file__).parents[2] / "fixtures/worker/golden-frames.json").read_text())
@pytest.mark.parametrize("case", GOLDENS, ids=lambda case: case["name"])
def test_shared_golden_bytes(case):
    writer = io.BytesIO()
    protocol.write(writer, case["frame"])
    assert writer.getvalue().hex() == case["hex"]
    assert protocol.read(io.BytesIO(bytes.fromhex(case["hex"]))) == case["frame"]
    protocol.validate_frame(case["frame"])
@pytest.mark.parametrize("payload", [b"\x00\x00", b"\x00\x00\x00\x00", b"\x00\x10\x00\x01", b"\x00\x00\x00\x02{"])
def test_invalid_prefix_or_payload(payload):
    with pytest.raises(ValueError): protocol.read(io.BytesIO(payload))
def test_split_reads_and_two_requests_in_one_session():
    class Split(io.BytesIO):
        def read(self, count=-1): return super().read(min(count, 1))
    source = io.BytesIO()
    for name in ["synthetic-a", "synthetic-b"]:
        protocol.write(source, {"kind": "request", "payload": {"schema_version":"planning-kernel-contract/0.1","request_id":name}})
        protocol.write(source, {"kind": "reference", "payload": {"request_id":name,"status":"ok"}})
    target = io.BytesIO()
    run(Split(source.getvalue()), target)
    reader = io.BytesIO(target.getvalue())
    assert [protocol.read(reader)["request_id"] for _ in range(2)] == ["synthetic-a", "synthetic-b"]
    assert protocol.read(reader) is None
def test_cancellation():
    source=io.BytesIO()
    protocol.write(source,{"kind":"request","payload":{"schema_version":"planning-kernel-contract/0.1","request_id":"synthetic"}})
    protocol.write(source,{"kind":"cancel","payload":{"request_id":"synthetic"}})
    target=io.BytesIO();run(io.BytesIO(source.getvalue()),target)
    assert protocol.read(io.BytesIO(target.getvalue()))["frame_type"]=="cancelled"
def test_unknown_type_duplicate_final_and_order():
    frame=next(case["frame"] for case in GOLDENS if case["name"]=="final_response")
    with pytest.raises(ValueError):protocol.validate_frame({**frame,"frame_type":"unknown"})
    with pytest.raises(ValueError):protocol.validate_sequence([frame,{**frame,"frame_index":1}])
    with pytest.raises(ValueError):protocol.validate_sequence([{**frame,"frame_index":1}])
