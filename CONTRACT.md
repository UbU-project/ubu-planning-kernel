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

## Response Status

Planning and repair responses carry `status`: `ok` means every eligible Task is
placed; `partial` means a Plan survives with a nonempty `unplaced_tasks` report;
`rejected` means no Plan survives and the report is empty. `engine_error` is
reserved for a backend failing outside planning and is never produced by the
CPU path. Plan's own candidate/validated status is a separate lifecycle field.

## Partial Placement

UBU-D0289 lets a `CandidateSet` report optional work its plans leave out. Every
plan in one set omits the same Tasks, keeping scores comparable and one report
accurate. A candidate omitting a Task must not schedule that Task's dependents;
they are reported as `deferred_dependency`. Mandatory Tasks (UBU-D0288), Static
Tasks, fixed placements, and their transitive prerequisites are protected.
Their placement failure remains a blocking `SkeletonFailureDiagnostic`.

The report uses `diagnostic_id` and `task_ref` as specified in the design
contract. The older skeleton diagnostic's `task_id` spelling is unchanged;
that divergence awaits GPU worker parity work. Five reasons are implemented:
`insufficient_total_capacity`, `no_eligible_chunk_large_enough`,
`outside_allowed_window`, `omitted_lower_value`, and `deferred_dependency`.
Split-policy and extension-limit reasons await their corresponding features.
No horizon extension is attempted: the three capacity/window reasons record
`skipped_by_policy`; ranking omissions and dependency deferrals record
`not_applicable`. Suggested safe alternatives require user input and do not
mutate the request.

The schema version remains `planning-kernel-contract/0.1`. UBU-D0289's shared
`0.2` bump lands with the split-policy change UBU-D0284, so that version names
both halves together. These fields are additive and consumers pin revisions.

## Numeric Round-Trip

The existing `serde_json` dependency enables `float_roundtrip` without changing
versions or Cargo.lock. Serialized scores, probabilities and selection ranks
parse back to the same floating-point values, supporting exact CPU/GPU parity.

## Coverage

A boundary outcome records the candidate's Static placement index, start and
Task ref, the set of work completed by that boundary, and remaining lateness
rounded up to whole 60-second buckets. Zero lateness remains distinct. The
response records `lateness_seconds_ceil_60` as its quantization rule. Digests use
FNV-1a over fixed-order little-endian u64 fields and length-prefixed UTF-8 strings,
with completed ids in sorted order; no machine memory layout is hashed.

Phase 1b certifies two continuations using the same duration draws: the Plan as
written, and the Plan with optional work that no longer fits omitted. A separate
continuation clock does not advance when it drops work. A missing prerequisite
also prevents its dependent from continuing; a mandatory or Static Task can
never be dropped. Failing protected work makes the continuation fail.

`coverage_estimate >= display_probability` always holds for generated, validated
candidates. The original feasibility clock, outcome and scores are unchanged.
Coverage estimates successful continuation rollouts; uncovered mass is its
complement and confidence is 95% Wilson over those same samples. A merged
boundary state counts as covered only when none of its visits failed; its
uncovered mass counts failing visits divided by all rollouts. The summary is
conservative about state certification even when some visits to a mixed state
continue. A boundary's heaviest failing state is chosen by failure count then
smaller digest.

`horizon_policy` defaults to 3,600 reactive seconds and target 0.99. Only Static
boundaries up to window start plus that duration (clamped to window end) are
listed. This first slice assesses the full candidate continuation against those
states; it does not truncate the feasibility or continuation walks. No sampled
boundaries means an empty summary, not an invented boundary. Coverage is absent
when rollout does not run, and `budget_limited` is false. Alternative continuation
search, compute-budget allocation and mobile continuation refs remain later work.
