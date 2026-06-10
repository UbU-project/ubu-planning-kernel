use crate::diagnostics::Diagnostic;
use crate::request::PlanningRequest;
use crate::response::Plan;

#[derive(Debug, Clone, Default)]
pub struct CandidateSet {
    pub plans: Vec<Plan>,
    pub diagnostics: Vec<Diagnostic>,
}

pub trait PlannerStrategy {
    fn generate_candidates(&self, request: &PlanningRequest) -> CandidateSet;
}
