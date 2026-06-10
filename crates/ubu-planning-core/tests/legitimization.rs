use ubu_planning_core::{legitimization, DiagnosticCode, Plan, PlanStatus, ScheduledTask};

#[test]
fn legitimization_stubs_are_explicit() {
    let plan = Plan {
        plan_id: "plan-legitimization".to_string(),
        status: PlanStatus::Candidate,
        tasks: vec![ScheduledTask {
            task_id: "task-a".to_string(),
            start: 0,
            end: 1,
            depends_on: Vec::new(),
            static_anchor: false,
        }],
    };

    let semi = legitimization::semi_legitimize(&plan);
    let full = legitimization::full_legitimize(&plan);

    assert!(!semi.is_valid);
    assert!(!full.is_valid);
    assert_eq!(semi.diagnostics[0].code, DiagnosticCode::NotYetImplemented);
    assert_eq!(full.diagnostics[0].code, DiagnosticCode::NotYetImplemented);
}
