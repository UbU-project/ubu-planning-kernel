use std::collections::BTreeSet;

use ubu_planning_core::request::PlanningRequest;
use ubu_planning_core::response::Plan;
use ubu_planning_core::strategy::CandidateSet;

const MAX_CANDIDATES: usize = 16;

pub fn generate(request: &PlanningRequest) -> CandidateSet {
    match crate::skeleton::build_skeleton(request) {
        Ok(baseline) => CandidateSet {
            plans: bounded_perturbations(request, baseline),
            diagnostics: Vec::new(),
        },
        Err(diagnostic) => CandidateSet {
            plans: Vec::new(),
            diagnostics: vec![diagnostic.into()],
        },
    }
}

fn bounded_perturbations(request: &PlanningRequest, baseline: Plan) -> Vec<Plan> {
    let Some(window) = &request.time_window else {
        return vec![baseline];
    };
    let mut proposals = Vec::new();
    for pivot in 0..baseline.steps.len() {
        let suffix = &baseline.steps[pivot..];
        if suffix
            .iter()
            .any(|step| step.static_anchor || step.end <= window.start || step.start < window.start)
        {
            continue;
        }
        let mut maximum_shift = window.end.saturating_sub(
            suffix
                .iter()
                .map(|step| step.end)
                .max()
                .unwrap_or(window.end),
        );
        for step in suffix {
            if let Some(task) = request.tasks().iter().find(|task| task.id == step.task_id) {
                if let Some(task_window) = &task.window {
                    maximum_shift = maximum_shift.min(task_window.end.saturating_sub(step.end));
                }
            }
        }
        if maximum_shift == 0 {
            continue;
        }

        // At most fifteen distinct placements per pivot. Integer interpolation
        // makes the bound independent of the size of the schedule window.
        for ordinal in 1..=maximum_shift.min(15) {
            let count = maximum_shift.min(15);
            let shift = ((ordinal as u128 * maximum_shift as u128) / count as u128) as u64;
            proposals.push((proposal_key(request.rng_seed, pivot, shift), pivot, shift));
        }
    }
    proposals.sort_unstable();

    let mut plans = vec![baseline.clone()];
    let mut placements = BTreeSet::new();
    placements.insert(placement_key(&baseline));
    for (_, pivot, shift) in proposals {
        if plans.len() == MAX_CANDIDATES {
            break;
        }
        let mut candidate = baseline.clone();
        for step in &mut candidate.steps[pivot..] {
            step.start += shift;
            step.end += shift;
        }
        if !placements.insert(placement_key(&candidate)) {
            continue;
        }
        candidate.plan_id = format!("{}-c{:02}", baseline.plan_id, plans.len());
        plans.push(candidate);
    }
    plans
}

fn placement_key(plan: &Plan) -> Vec<(String, u64, u64)> {
    plan.steps
        .iter()
        .map(|step| (step.task_id.clone(), step.start, step.end))
        .collect()
}

fn proposal_key(seed: u64, pivot: usize, shift: u64) -> u64 {
    // SplitMix64 gives a deterministic seed-dependent ordering without adding a
    // stochastic search or a platform-dependent RNG implementation.
    let mut value = seed ^ (pivot as u64).rotate_left(21) ^ shift.rotate_left(43);
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::MAX_CANDIDATES;

    #[test]
    fn candidate_bound_is_contractual() {
        assert_eq!(MAX_CANDIDATES, 16);
    }
}
