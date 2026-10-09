use serde_json::json;
use ubu_planning_core::{Plan, PlannerStrategy, PlanningRequest};
use ubu_planning_cpu::CpuStrategy;

fn interleaved_request(seed: u64, static_prefix: bool, early_duration: u64) -> PlanningRequest {
    serde_json::from_value(json!({
        "schema_version":"planning-kernel-contract/0.1",
        "request_id":"synthetic-interleaved-occupancy", "rng_seed":seed, "n_rollouts":0,
        "time_window":{"start":0,"end":1000},
        "topological_order":["synthetic-prefix","synthetic-early","synthetic-late"],
        "tasks":[
            {"id":"synthetic-prefix", "duration":{"type":"fixed","seconds":10},
             "window":{"start":100,"end":1000},
             "static_anchor":static_prefix.then(|| json!({"start":100}))},
            {"id":"synthetic-early", "duration":{"type":"fixed","seconds":early_duration}},
            {"id":"synthetic-late", "duration":{"type":"fixed","seconds":10},
             "window":{"start":200,"end":1000}, "depends_on":["synthetic-early"], "mandatory":true}
        ]
    }))
    .unwrap()
}

fn assert_pairwise_disjoint(plan: &Plan) {
    for (index, step) in plan.steps.iter().enumerate() {
        for other in &plan.steps[index + 1..] {
            assert!(step.end <= other.start || other.end <= step.start);
        }
    }
}

#[test]
fn every_generated_candidate_is_disjoint_across_topological_time_interleaving() {
    let mut touching = false;
    for static_prefix in [true, false] {
        for seed in 0..64 {
            let candidates =
                CpuStrategy.generate_candidates(&interleaved_request(seed, static_prefix, 10));
            assert_eq!(candidates.plans.len(), 16);
            let baseline = &candidates.plans[0];
            assert_eq!(
                baseline.steps.iter().map(|s| s.start).collect::<Vec<_>>(),
                [100, 0, 200]
            );
            // The nearest non-suffix interval is before the suffix's maximum end.
            assert!(baseline.steps[0].start < baseline.steps[2].end);
            for plan in candidates.plans {
                assert_pairwise_disjoint(&plan);
                if plan.steps[0].start == 100 {
                    assert!(plan.steps[1].end <= 100);
                    touching |= plan.steps[1].end == 100;
                    if plan.steps[1].start > 0 {
                        assert_eq!(plan.steps[2].start - 200, plan.steps[1].start);
                    }
                }
            }
        }
    }
    assert!(
        touching,
        "endpoint adjacency must remain an admissible perturbation"
    );
}

#[test]
fn zero_occupancy_gap_keeps_the_early_step_and_allows_later_suffixes() {
    let candidates = CpuStrategy.generate_candidates(&interleaved_request(0, true, 100));
    assert_eq!(candidates.plans.len(), 16);
    assert!(candidates
        .plans
        .iter()
        .skip(1)
        .any(|p| p.steps[2].start > 200));
    for plan in candidates.plans {
        assert_eq!((plan.steps[1].start, plan.steps[1].end), (0, 100));
        assert_pairwise_disjoint(&plan);
    }
}
