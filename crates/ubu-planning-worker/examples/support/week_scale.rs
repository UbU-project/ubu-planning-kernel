//! Shape-only generation. No external input, titles, facts or operator durations.
use serde_json::{json, Value};
use ubu_planning_core::PlanningRequest;
use ubu_planning_worker::stage1::reference_output;
pub const SEED: u64 = 0x78a1_2026;
pub const ANCHORS: usize = 93;
pub const ROUTINES: usize = 7;
pub const DYNAMIC: usize = 27;
pub const EARLY_DYNAMIC: usize = 10;
pub const DAY_SECONDS: u64 = 86_400;

pub fn cases() -> Vec<Value> {
    let mut tasks = Vec::new();
    let mut commitment_indices = Vec::new();
    for day in 0..ROUTINES {
        for block in 0..(12 + usize::from(day < 2)) {
            let index = tasks.len();
            if block == 0 {
                commitment_indices.push(index);
            }
            tasks.push(json!({"id":format!("synthetic-anchor-{index:03}"),
                "duration":{"type":"fixed","seconds":600 + ((SEED ^ index as u64) % 17) * 11},
                "static_anchor":{"start":day as u64 * DAY_SECONDS + 28_800 + block as u64 * 3600}}));
        }
    }
    let last_anchor = tasks.len() - 1;
    let last_anchor_id = tasks[last_anchor]["id"].as_str().unwrap().to_owned();
    for day in 0..ROUTINES {
        tasks.push(
            json!({"id":format!("synthetic-routine-{day:02}"),"mandatory":true,
            "duration":{"type":"fixed","seconds":900},
            "static_anchor":{"start":day as u64 * DAY_SECONDS + 21_600}}),
        );
    }
    assert_eq!(tasks.len(), ANCHORS);
    for index in 0..DYNAMIC {
        let mut deps = Vec::new();
        if index > 0 {
            deps.push(format!("synthetic-dynamic-{:02}", index - 1));
        }
        if index == EARLY_DYNAMIC {
            deps.push(last_anchor_id.clone());
        }
        tasks.push(json!({"id":format!("synthetic-dynamic-{index:02}"),
            "duration":{"type":"fixed","seconds":420 + ((SEED + index as u64 * 37) % 13) * 23},
            "mandatory":index == DYNAMIC - 1,"depends_on":deps,
            "window":{"start":28_800,"end":ROUTINES as u64 * DAY_SECONDS}}));
    }
    // An anchor depends on a real ten-deep predecessor chain; a mandatory tail
    // protects the whole 27-deep chain. Late suffixes can still be perturbed.
    tasks[last_anchor]["depends_on"] =
        json!([format!("synthetic-dynamic-{:02}", EARLY_DYNAMIC - 1)]);
    let mut order: Vec<String> = tasks[..ANCHORS]
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != last_anchor)
        .map(|(_, t)| t["id"].as_str().unwrap().into())
        .collect();
    order.extend((0..EARLY_DYNAMIC).map(|i| format!("synthetic-dynamic-{i:02}")));
    order.push(last_anchor_id);
    order.extend((EARLY_DYNAMIC..DYNAMIC).map(|i| format!("synthetic-dynamic-{i:02}")));
    let request = json!({"schema_version":"planning-kernel-contract/0.1","request_id":"synthetic-week-bound",
        "rng_seed":SEED,"n_rollouts":0,"time_window":{"start":0,"end":ROUTINES as u64 * DAY_SECONDS},
        "tasks":tasks,"topological_order":order});
    let mut padding = request.clone();
    padding["request_id"] = json!("synthetic-week-candidate-padding");
    let parsed: PlanningRequest = serde_json::from_value(request.clone()).unwrap();
    let baseline = reference_output(&parsed);
    assert!(
        baseline.failure.is_none(),
        "synthetic base shape must be feasible"
    );
    let last_end = baseline.start_time_offsets[0]
        .iter()
        .zip(&baseline.duration_samples[0])
        .map(|(start, duration)| start + duration)
        .max()
        .unwrap();
    padding["time_window"]["end"] = json!(last_end);
    // Without the bridge through the last anchor, the dynamic chain occupies
    // early gaps while its anchor-free topological suffix has occupancy ahead.
    let mut ahead = request.clone();
    ahead["request_id"] = json!("synthetic-week-occupancy-ahead");
    ahead["tasks"][last_anchor]["depends_on"] = json!([]);
    ahead["tasks"][ANCHORS + EARLY_DYNAMIC]["depends_on"] =
        json!([format!("synthetic-dynamic-{:02}", EARLY_DYNAMIC - 1)]);
    ahead["topological_order"] = json!(ahead["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["id"].as_str().unwrap())
        .collect::<Vec<_>>());
    let mut overlaps = request.clone();
    overlaps["request_id"] = json!("synthetic-week-three-overlaps");
    for (day, index) in commitment_indices.into_iter().take(3).enumerate() {
        overlaps["tasks"][index]["static_anchor"]["start"] =
            json!(day as u64 * DAY_SECONDS + 21_660);
    }
    [("synthetic-week-bound",request,0,EARLY_DYNAMIC),("synthetic-week-candidate-padding",padding,0,EARLY_DYNAMIC),
        ("synthetic-week-three-overlaps",overlaps,3,EARLY_DYNAMIC),("synthetic-week-occupancy-ahead",ahead,0,0)].into_iter().map(|(name,request,overlaps,early_dynamic)|
        json!({"name":name,"request":request,"shape":{"seed":SEED,"anchors":ANCHORS,"routine_anchors":ROUTINES,
            "dynamic":DYNAMIC,"tasks":ANCHORS+DYNAMIC,"overlap_pairs":overlaps,"early_dynamic":early_dynamic}})).collect()
}
