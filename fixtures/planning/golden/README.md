# Planning Goldens

`skeleton-phase-a.json` is a human-reviewed-then-frozen regression anchor for Phase A skeleton planning and repair. Each case pairs a fixed-`rng_seed` request with the exact expected `PlanningResponse` produced by the deterministic CPU planner.

`affect-legitimization.json` is a human-reviewed-then-frozen regression anchor for the Phase B affect legitimacy filter. Each case pairs a fixed-`rng_seed` request carrying an `AffectProfile` and affect observation with the exact expected `LegitimizationReport`.

`scoring-selection-c1.json` is a human-reviewed-then-frozen regression anchor for the C-1 bounded candidate generation, Stage 3 value scoring, semi-legitimization pruning, candidate roles, deterministic tiebreaking, and rank-1 selection pipeline. It records the full post-P7 kernel revision used to freeze the corpus. Its probability summaries are deliberately empty.

These fixtures are not auto-trusted as proof of correctness. They are review artifacts: changes to expected output should be deliberate, reviewed, and tied to an intentional contract or planner behavior change.

The corpus is offline-only. Do not add live, networked, probability, duration-sampling, correlation-matrix, rollout, support-task, or GPU fixtures here.
