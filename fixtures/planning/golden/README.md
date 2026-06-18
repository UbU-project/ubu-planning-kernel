# Planning Goldens

`skeleton-phase-a.json` is a human-reviewed-then-frozen regression anchor for Phase A skeleton planning and repair. Each case pairs a fixed-`rng_seed` request with the exact expected `PlanningResponse` produced by the deterministic CPU planner.

`affect-legitimization.json` is a human-reviewed-then-frozen regression anchor for the Phase B affect legitimacy filter. Each case pairs a fixed-`rng_seed` request carrying an `AffectProfile` and affect observation with the exact expected `LegitimizationReport`.

These fixtures are not auto-trusted as proof of correctness. They are review artifacts: changes to expected output should be deliberate, reviewed, and tied to an intentional contract or planner behavior change.

The corpus is offline-only. Do not add live, networked, scoring, probability, robustness, rollout, support-task, or GPU fixtures here.
