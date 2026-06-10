use ubu_planning_core::response::Plan;

pub fn select_default(mut candidates: Vec<Plan>) -> Plan {
    candidates.sort_by(|left, right| left.plan_id.cmp(&right.plan_id));
    candidates
        .into_iter()
        .next()
        .expect("candidate generation supplies at least one plan")
}
