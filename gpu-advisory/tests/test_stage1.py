import copy
import pytest
from ubu_planning_worker import stage1

def payload():
    return {"profile":stage1.PROFILE,"request":{"request_id":"invented-stage","rng_seed":17,"tasks":[{"id":"invented-a","duration":{"type":"fixed","seconds":2}},{"id":"invented-b","depends_on":["invented-a"],"duration":{"type":"fixed","seconds":3}}],"time_window":{"start":"1970-01-01T00:00:00Z","end":"1970-01-01T00:00:20Z"}},"topological_order":["invented-a","invented-b"],"task_validity_mask":[True,True]+[False]*254,"sampling":{"kind":"placement_seconds","duration_samples":[2,3]+[0]*254}}

@pytest.fixture
def cpu_torch():
    try:
        import torch
    except Exception:
        pytest.skip("torch absent or broken; CPU-only checks remain mandatory")
    if torch.__version__ != "2.6.0+cpu":
        pytest.skip("pinned CPU-only torch unavailable")
    return torch

def check_shape(result):
    assert len(result["validity_mask"]) == 16
    for name in ("task_index","slot_mask","start_time_offsets","duration_samples","piece_index","piece_count"):
        assert len(result[name]) == 16 and all(len(row)==256 for row in result[name])
    assert result["piece_index"][0][:2] == result["piece_count"][0][:2] == [1,1]
    assert all(value == 0 for value in result["piece_count"][0][2:])

def test_python_profile_shapes_and_exact_repeat():
    result=stage1.reference_without_framework(payload());check_shape(result)
    assert result == stage1.reference_without_framework(payload())
    assert result["start_time_offsets"][0][:2] == [0,2]

@pytest.mark.parametrize("use_torch",[False,True])
def test_padding_never_influences_result(use_torch,request):
    if use_torch: request.getfixturevalue("cpu_torch")
    solve=stage1.compute if use_torch else stage1.reference_without_framework
    first=payload();second=copy.deepcopy(first)
    second["sampling"]["duration_samples"][2:] = [(1<<100)]*254
    assert solve(first) == solve(second)

def test_tensor_shapes_dtypes_and_exact_python_implementation(cpu_torch):
    actual=stage1.compute(payload());check_shape(actual)
    assert actual == stage1.reference_without_framework(payload())

@pytest.mark.parametrize("use_torch",[False,True])
def test_cycle_rejects_without_candidates(use_torch,request):
    if use_torch: request.getfixturevalue("cpu_torch")
    p=payload();p["request"]["tasks"][0]["depends_on"]=["invented-b"]
    result=(stage1.compute if use_torch else stage1.reference_without_framework)(p)
    assert not any(result["validity_mask"])
    assert result["rejection_codes"] == ["dependency_cycle"]*16

@pytest.mark.parametrize("use_torch",[False,True])
def test_split_policy_is_explicitly_rejected(use_torch,request):
    if use_torch: request.getfixturevalue("cpu_torch")
    p=payload();p["request"]["tasks"][0]["split_policy"]={"type":"splittable","min_piece_seconds":1,"resume_overhead_seconds":0,"max_pieces":2}
    result=(stage1.compute if use_torch else stage1.reference_without_framework)(p)
    assert not any(result["validity_mask"])
    assert result["rejection_codes"] == ["unsupported_split_policy"]*16

import json
from pathlib import Path
from ubu_planning_worker import protocol
GOLDENS=json.loads((Path(__file__).parents[2]/"fixtures/worker/stage1-goldens.json").read_text())
FRAMES=json.loads((Path(__file__).parents[2]/"fixtures/worker/stage1-frames.json").read_text())
@pytest.mark.parametrize("case",GOLDENS,ids=lambda c:c["name"])
def test_python_algorithm_matches_rust_golden_exactly(case):
    assert stage1.reference_without_framework(case["input"]) == case["expected"]

@pytest.mark.parametrize("case",GOLDENS,ids=lambda c:c["name"])
def test_tensor_algorithm_matches_rust_golden_exactly(case,cpu_torch):
    assert stage1.compute(case["input"]) == case["expected"]

@pytest.mark.parametrize("case",FRAMES,ids=lambda c:c["name"])
def test_shared_stage1_frame_bytes(case):
    import io
    writer=io.BytesIO();protocol.write(writer,case["frame"])
    assert writer.getvalue().hex() == case["hex"]
    assert protocol.read(io.BytesIO(writer.getvalue())) == case["frame"]
