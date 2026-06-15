from __future__ import annotations

from .protocol import GpuAdvisoryRequest, GpuAdvisoryResponse


def advise(request: GpuAdvisoryRequest) -> GpuAdvisoryResponse:
    return {
        "request_id": request["request_id"],
        "responded_at": request["requested_at"],
        "recommendation": "manual_review",
        "rationale": "no-op advisory scaffold; no GPU hardware or torch required",
    }
