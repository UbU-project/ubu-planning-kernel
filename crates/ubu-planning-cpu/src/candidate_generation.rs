use ubu_planning_core::request::PlanningRequest;
use ubu_planning_core::strategy::CandidateSet;

pub fn generate(request: &PlanningRequest) -> CandidateSet {
    match crate::skeleton::build_skeleton(request) {
        Ok(plan) => CandidateSet {
            plans: vec![crate::default_selection::select_default(vec![plan])],
            diagnostics: Vec::new(),
        },
        Err(diagnostic) => CandidateSet {
            plans: Vec::new(),
            diagnostics: vec![diagnostic.into()],
        },
    }
}
