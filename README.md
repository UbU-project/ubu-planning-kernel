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
