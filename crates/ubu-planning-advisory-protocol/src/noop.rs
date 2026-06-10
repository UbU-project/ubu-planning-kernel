use ubu_core::worker::{GpuAdvisoryRecommendation, GpuAdvisoryRequest, GpuAdvisoryResponse};

pub fn advise_noop(request: &GpuAdvisoryRequest) -> GpuAdvisoryResponse {
    GpuAdvisoryResponse {
        request_id: request.request_id.clone(),
        responded_at: request.requested_at,
        recommendation: GpuAdvisoryRecommendation::ManualReview,
        rationale: Some("no-op advisory scaffold; no GPU hardware or torch required".to_string()),
        estimated_cost: None,
    }
}
