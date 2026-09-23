# ubu-planning-kernel

Deterministic Rust planning kernel for UbU Phase 1.

This repository is an internal Cargo workspace. The Rust CPU planning core is authoritative, synchronous, and deterministic. GPU work is advisory only: it may propose rankings or diagnostics, but every final plan is certified by `ubu_planning_core`.

## Workspace

- `ubu_planning_core`: public planning API, authoritative validation, and semi/full-legitimization entrypoints.
- `ubu_planning_cpu`: deterministic CPU `PlannerStrategy` implementation.
- `ubu_planning_advisory_protocol`: JSON stdio plumbing for canonical `ubu_core` GPU advisory wire types.
- `ubu_planning_cli`: thin local CLI for fixture-oriented planning, validation, repair, and advisory checks.
- `gpu-advisory`: stdlib-only Python no-op advisory process.

## Authority Rules

Allowed GPU advisory behavior:

- Propose candidate ranking.
- Batch-score candidate schedules.
- Simulate uncertainty.
- Estimate robustness.
- Return advisory diagnostics.

Forbidden GPU advisory behavior:

- Certify final `Plan` validity.
- Bypass dependency validation.
- Bypass static task constraints.
- Bypass Compartment/export rules.
- Mutate canonical store state.
- Be required for default Phase 1 fixture mode.

## Development

```sh
cargo fmt --check
cargo clippy --workspace --all-targets
cargo test --workspace
```

Python advisory tests are optional for Rust CI and require only stdlib Python plus pytest:

```sh
cd gpu-advisory
python -m pytest tests
```

No default build path requires GPU hardware, CUDA, torch, or Python.

## Planner strategies

`ubu-planning plan <path> [--strategy greedy|chunked]` and
`ubu-planning repair <path> [--strategy greedy|chunked]` default to `greedy`.
Unknown strategy values are errors. The default preserves fixture-oriented CLI output.

The greedy `CpuStrategy` builds a first-fit skeleton in topological order and
adds bounded suffix delays. `ChunkedSweepStrategy` partitions free time around
Static and preserved placements, then sweeps the chunks with a bounded beam.
Its strategy parameters are `alternatives_per_chunk` (K, default 4, clamped to
1–4) and `beam_width` (B, default 16, minimum 1). K selects value-first,
most-constrained-first, value-density, and protected-first fills in that order. Dependency bounds,
forced placements, capacity look-ahead, and merging by omitted and remaining Task sets keep
the search bounded and deterministic.

The candidate set contains sweep plans, the greedy baseline when distinct, and
chunk-tail delay variants, capped at 16. The greedy baseline replaces the sweep when it gives up less protected work,
joins only when it omits the same Tasks, and is discarded when it gives up more.
It remains available when the sweep fails. Chunk-tail delays preserve Task windows and fixed
boundaries while providing alternatives within chunks.

Rollouts are unchanged: all candidates use the existing shared latent duration
draws, and overrunning a Static boundary makes a rollout infeasible. Per-chunk
rollout structure, streaming, and splittable Tasks remain later work.

## Partial placement

`TaskSpec.mandatory` defaults to false. Routine occurrences mark it true, so
value zero does not make them expendable. Mandatory Tasks, Static Tasks, fixed
placements, and their transitive prerequisites are protected from omission.
An impossible protected Task still rejects the Plan.

Optional Dynamic Tasks that do not fit are reported in `unplaced_tasks` while
the remaining Plan proceeds with response status `partial`. Dependents of an
omitted Task are deferred too. Empty Plans are rejected. The omission order is
lowest value first, then latest own deadline (missing deadlines first), then
largest id. Protection compares the reverse order in one shared implementation.
The beam compares omission sets before utility, and every candidate in a
response omits the same set. Greedy uses the same report, with no chunk refs.

Horizon extension is never attempted. The orchestrator owns the horizon and no
extension policy bound exists in the kernel. Capacity/window reports record
`skipped_by_policy`; alternatives describe changes the user can request.

## Coverage

`horizon_policy` defaults to `reactive_horizon_seconds: 3600` and
`branch_coverage_target: 0.99`; supplied requests may override them. Rollout
finalists carry `coverage`, with an estimate, uncovered mass, 95% Wilson interval
and boundary continuation summary. Boundaries are the candidate's own Static
placements inside the reactive horizon; the recorded merge rule rounds lateness
up to whole minutes.

`display_probability` still measures running the Plan exactly as written.
Coverage additionally credits continuing after optional work is dropped, so it
can be higher. Protected work is never dropped. This slice uses existing draws
and reports `budget_limited: false`; it does not search alternative continuations.
