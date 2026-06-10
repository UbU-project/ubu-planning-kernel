from __future__ import annotations

from .protocol import GpuAdvisoryRequest, GpuAdvisoryResponse


def advise(request: GpuAdvisoryRequest) -> GpuAdvisoryResponse:
    return {
        "requestId": request["requestId"],
        "respondedAt": request["requestedAt"],
        "recommendation": "manual_review",
        "rationale": "no-op advisory scaffold; no GPU hardware or torch required",
    }
