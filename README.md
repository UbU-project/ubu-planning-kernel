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
Its strategy parameters are `alternatives_per_chunk` (K, default 3, clamped to
1–3) and `beam_width` (B, default 16, minimum 1). K selects value-first,
most-constrained-first, and value-density fills in that order. Dependency bounds,
forced placements, capacity look-ahead, and merging by remaining Task set keep
the search bounded and deterministic.

The candidate set contains sweep plans, the greedy baseline when distinct, and
chunk-tail delay variants, capped at 16. The greedy baseline remains available
even when the sweep fails. Chunk-tail delays preserve Task windows and fixed
boundaries while providing alternatives within chunks.

Rollouts are unchanged: all candidates use the existing shared latent duration
draws, and overrunning a Static boundary makes a rollout infeasible. Per-chunk
rollout structure, streaming, splittable Tasks, partial placement, and mandatory
routine constraints remain later work. No request or response fields change.
