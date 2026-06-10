use ubu_planning_core::request::PlanningRequest;
use ubu_planning_core::response::Plan;

pub fn rebuild(request: &PlanningRequest) -> Option<Plan> {
    crate::skeleton::build_skeleton(request).ok()
}
