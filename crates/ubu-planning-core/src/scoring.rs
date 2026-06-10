use crate::response::Plan;

pub fn deterministic_score(candidate: &Plan) -> usize {
    candidate.tasks.len()
}
