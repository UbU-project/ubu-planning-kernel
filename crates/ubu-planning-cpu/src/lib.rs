pub mod candidate_generation;
pub mod default_selection;
pub mod repair;
pub mod skeleton;

use ubu_planning_core::request::PlanningRequest;
use ubu_planning_core::strategy::{CandidateSet, PlannerStrategy};

#[derive(Debug, Clone, Copy, Default)]
pub struct CpuStrategy;

impl PlannerStrategy for CpuStrategy {
    fn generate_candidates(&self, request: &PlanningRequest) -> CandidateSet {
        candidate_generation::generate(request)
    }
}
