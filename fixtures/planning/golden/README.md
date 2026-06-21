# Planning Goldens

`skeleton-phase-a.json` is a human-reviewed-then-frozen regression anchor for Phase A skeleton planning and repair. Each case pairs a fixed-`rng_seed` request with the exact expected `PlanningResponse` produced by the deterministic CPU planner.

`affect-legitimization.json` is a human-reviewed-then-frozen regression anchor for the Phase B affect legitimacy filter. Each case pairs a fixed-`rng_seed` request carrying an `AffectProfile` and affect observation with the exact expected `LegitimizationReport`.

`scoring-selection-c1.json` is a human-reviewed-then-frozen regression anchor for the C-1 bounded candidate generation, Stage 3 value scoring, semi-legitimization pruning, candidate roles, deterministic tiebreaking, and rank-1 selection pipeline. It records the full post-P7 kernel revision used to freeze the corpus. Its probability summaries are deliberately empty.

`rollout-c2.json` is the human-reviewed-then-frozen, fixed-seed Stage 4 corpus. It freezes feasibility frequency, p10 robustness, display probability, Wilson bounds, probability quality, the `rng_seed + 3` stage seed, retained candidate order, and rollout-driven default changes. `rollout-degraded-c2.json` freezes the numeric-jitter, independence-fallback, and strict-rejection policy against synthetic matrices that cannot arise from the valid PSD-by-construction request path.

These fixtures are not auto-trusted as proof of correctness. They are review artifacts: changes to expected output should be deliberate, reviewed, and tied to an intentional contract or planner behavior change.

The corpus is offline-only. Do not add live, networked, external-event, support-task, or GPU fixtures here.
