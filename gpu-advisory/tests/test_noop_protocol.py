from ubu_gpu_advisory.noop import advise


def test_noop_protocol_round_trip_shape() -> None:
    request = {
        "requestId": "advisory-fixture",
        "requestedAt": "2026-01-01T00:00:00Z",
        "workload": "fixture",
    }

    response = advise(request)

    assert response == {
        "requestId": "advisory-fixture",
        "respondedAt": "2026-01-01T00:00:00Z",
        "recommendation": "manual_review",
        "rationale": "no-op advisory scaffold; no GPU hardware or torch required",
    }
