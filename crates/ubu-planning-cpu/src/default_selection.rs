use ubu_planning_core::response::PlanCandidate;
use ubu_planning_core::scoring::compare_candidates;

pub fn select_default(mut candidates: Vec<PlanCandidate>) -> PlanCandidate {
    candidates.sort_by(compare_candidates);
    candidates
        .into_iter()
        .next()
        .expect("candidate generation supplies at least one plan")
}
