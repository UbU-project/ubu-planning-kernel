# Planning Kernel Contract

## Public API Ownership

`ubu_planning_core` owns the public planning API:

- `plan(request: PlanningRequest, strategy: &impl PlannerStrategy) -> PlanningResponse`
- `repair(request: RepairRequest, strategy: &impl PlannerStrategy) -> RepairResponse`
- `validate_plan(candidate: &Plan) -> ValidationResult`
- `explain_plan(candidate: &Plan) -> ExplanationBundle`

`plan` and `repair` require an explicit strategy argument. Core has no built-in default strategy.

## Authority Layering

`ubu_planning_core` owns authoritative deterministic validation and legitimization. `PlannerStrategy` implementations generate candidates only.

`ubu_planning_cpu` provides the default deterministic generator for Phase 1 fixture mode.

Future GPU strategy implementations must use the same `PlannerStrategy` candidate-proposer shape. GPU output is always certified by `ubu_planning_core::validate_plan` and never receives a separate certify entrypoint.

## Vocabulary

Use:

- semi-legitimization
- full-legitimization
- Legitimizer engine
- enforcement gate

Do not introduce public names using "decision" for planning authority concepts.
