from ubu_gpu_advisory.noop import advise


def test_noop_protocol_round_trip_shape() -> None:
    request = {
        "request_id": "advisory-fixture",
        "requested_at": "2026-01-01T00:00:00Z",
        "workload": "fixture",
    }

    response = advise(request)

    assert response == {
        "request_id": "advisory-fixture",
        "responded_at": "2026-01-01T00:00:00Z",
        "recommendation": "manual_review",
        "rationale": "no-op advisory scaffold; no GPU hardware or torch required",
    }
