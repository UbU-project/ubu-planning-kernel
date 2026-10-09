"""Independent atomic Stage 1; integer CPU tensors, no device search.

The internal stage1-atomic-v1 envelope is an explicitly approved exception to
D0283's complete-response envelope. Canonical PlanningStreamFrame is unchanged.
"""
from datetime import datetime
from functools import cmp_to_key

MAX_PLANNING_TASKS = 256
MAX_CANDIDATES = 16
MASK64 = (1 << 64) - 1
PROFILE = "stage1-atomic-v1"

def seconds(value):
    if not isinstance(value, str) or not value.endswith("Z"):
        raise ValueError("coordinate must be whole-second UTC")
    dt = datetime.fromisoformat(value[:-1] + "+00:00")
    result = int(dt.timestamp())
    if result < 0 or dt.microsecond or dt.utcoffset().total_seconds() != 0:
        raise ValueError("invalid coordinate")
    return result

def proposal_key(seed, pivot, shift):
    def rotate(value, count):
        return ((value << count) | (value >> (64 - count))) & MASK64
    value = seed ^ rotate(pivot, 21) ^ rotate(shift, 43)
    value = (value + 0x9e3779b97f4a7c15) & MASK64
    value = ((value ^ (value >> 30)) * 0xbf58476d1ce4e5b9) & MASK64
    value = ((value ^ (value >> 27)) * 0x94d049bb133111eb) & MASK64
    return value ^ (value >> 31)

def _solve(payload, torch=None):
    """Lists are the no-framework test oracle for the Python implementation.

    With torch, duration and slot-mask tensors drive placement; schedule and
    dependency tensor reductions drive the reported structural checks.
    """
    if payload.get("profile") != PROFILE:
        raise ValueError("unknown Stage 1 profile")
    request = payload["request"]
    tasks = request["tasks"]
    count = len(tasks)
    if count > MAX_PLANNING_TASKS:
        raise ValueError("Task bound exceeded")
    mask = payload["task_validity_mask"]
    draws = payload["sampling"]
    if draws.get("kind") != "placement_seconds":
        raise ValueError("unsupported sampling source")
    durations = draws["duration_samples"]
    if len(mask) != MAX_PLANNING_TASKS or len(durations) != MAX_PLANNING_TASKS or mask != [True] * count + [False] * (MAX_PLANNING_TASKS-count):
        raise ValueError("invalid padded inputs")
    if torch is not None:
        active = torch.tensor(mask, dtype=torch.bool, device="cpu")
        # Values in padded slots are discarded before tensor construction.
        duration_tensor = torch.tensor(durations[:count] + [0]*(MAX_PLANNING_TASKS-count), dtype=torch.int64, device="cpu")
        duration_tensor = duration_tensor.masked_fill(~active, 0)
        durations = duration_tensor.tolist()
    failures = None
    omitted = {}
    excluded = set()
    occupied = []
    steps = {}
    by_id = {task["id"]: (i, task) for i, task in enumerate(tasks)}

    def fail(task_id, reason, code="skeleton_failure"):
        nonlocal failures
        failures = {"task_id": task_id, "reason": reason, "code": code}
        raise _Failure

    def overlap(start, end):
        return next((entry for entry in sorted(occupied, key=lambda x:(x[1],x[2],x[0])) if start < entry[2] and end > entry[1]), None)

    try:
        if any(t.get("split_policy", {"type":"atomic"}) != {"type":"atomic"} for t in tasks):
            fail(None, "split policy is unsupported by legacy 0.1 Stage 1", "unsupported_split_policy")
        if request.get("mode", "fresh_generation") != "fresh_generation":
            fail(None, "repair remains on CPU", "unsupported_repair")
        window = request.get("time_window")
        if window is None:
            fail(None, "missing time_window starting state")
        window_start, window_end = seconds(window["start"]), seconds(window["end"])
        if window_start >= window_end:
            fail(None, "time_window has no available duration")
        # Validate acyclicity independently; never use a GPU-discovered order.
        remaining = {t["id"]: set(t.get("depends_on", [])) for t in tasks}
        resolved = set()
        while remaining:
            ready = sorted(key for key, deps in remaining.items() if deps <= resolved)
            if not ready:
                fail(None, "dependency graph did not produce a complete deterministic order", "dependency_cycle")
            for key in ready:
                resolved.add(key)
                del remaining[key]
        order = payload["topological_order"]
        if len(order) != count:
            fail(None, "provided topological_order length does not match task graph")
        seen = set()
        for key in order:
            if key not in by_id:
                fail(key, "provided topological_order contains an unknown task")
            if key in seen:
                fail(key, "provided topological_order contains a duplicate task")
            seen.add(key)
        positions = {key:i for i,key in enumerate(order)}
        for task in tasks:
            for dep in task.get("depends_on", []):
                if positions.get(dep, count) >= positions[task["id"]]:
                    fail(task["id"], f"provided topological_order places dependency '{dep}' after task")
        protected = {t["id"] for t in tasks if t.get("mandatory", False) or t.get("static_anchor") is not None}
        pending = list(protected)
        while pending:
            key = pending.pop()
            for dep in by_id[key][1].get("depends_on", []):
                if dep not in protected:
                    protected.add(dep); pending.append(dep)
        def bounds(task):
            local = task.get("window")
            lo, hi = window_start, window_end
            if local:
                lo,hi = max(lo,seconds(local["start"])), min(hi,seconds(local["end"]))
            if lo >= hi or hi-lo < durations[by_id[task["id"]][0]]:
                return None
            return lo,hi
        affixed = {}
        for key in order:
            i,task = by_id[key]
            expected = task["duration"].get("seconds",task["duration"].get("mode_seconds"))
            if type(durations[i]) is not int or durations[i] != expected or durations[i] <= 0:
                raise ValueError("placement duration differs from CPU profile")
            anchor = task.get("static_anchor")
            if anchor:
                bound = bounds(task)
                if bound is None: fail(key,"task has insufficient available window")
                start=seconds(anchor["start"]);end=start+durations[i]
                if start < bound[0]: fail(key,"static anchor collides with dependencies or window start")
                if end > bound[1]: fail(key,"static anchor exceeds available window")
                hit=overlap(start,end)
                if hit: fail(key,f"static anchor collides with scheduled task '{hit[0]}'")
                occupied.append((key,start,end)); affixed[key]=(i,start,end)
        for key in order:
            if key in excluded: continue
            i,task=by_id[key]
            deps=task.get("depends_on", [])
            earliest=max([window_start]+[steps[dep][2] for dep in deps])
            if key in affixed:
                step=affixed[key]
                if step[1] < earliest: fail(key,"static anchor collides with dependencies or window start")
                steps[key]=step;continue
            bound=bounds(task)
            reason="task has insufficient available window" if bound is None else None
            start=max(earliest,bound[0]) if bound else earliest
            if bound:
                while True:
                    end=start+durations[i]
                    if end > bound[1]:
                        reason="insufficient available window for deterministic skeleton placement";break
                    hit=overlap(start,end)
                    if hit: start=max(hit[2],start+1);continue
                    steps[key]=(i,start,end);occupied.append((key,start,end));break
            if reason:
                if key in protected: fail(key,reason)
                local=task.get("window")
                lo=max(earliest,seconds(local["start"]) if local else window_start)
                hi=min(window_end,seconds(local["end"]) if local else window_end)
                if lo+durations[i] > hi: omitted[key]="outside_allowed_window"
                elif any(k not in protected and s < hi and e > lo for k,s,e in occupied): omitted[key]="omitted_lower_value"
                else: omitted[key]="insufficient_total_capacity"
                excluded.add(key)
                changed=True
                while changed:
                    before=len(excluded)
                    excluded.update(t["id"] for t in tasks if set(t.get("depends_on",[])) & excluded)
                    changed=len(excluded)!=before
        base=[steps[key] for key in order if key not in excluded]
        if not base: fail(None,"partial placement left no Task in the Plan")
        proposals=[]
        for pivot in range(len(base)):
            suffix=base[pivot:]
            if any(tasks[i].get("static_anchor") or end <= window_start or start < window_start for i,start,end in suffix): continue
            maximum=max(0,window_end-max(end for _,_,end in suffix))
            suffix_tasks={tasks[i]["id"] for i,_,_ in suffix}
            for i,_,end in suffix:
                if tasks[i].get("window"): maximum=min(maximum,max(0,seconds(tasks[i]["window"]["end"])-end))
                ahead=[start-end for key,start,_ in occupied if key not in suffix_tasks and start >= end]
                if ahead: maximum=min(maximum,min(ahead))
            n=min(maximum,15)
            for ordinal in range(1,n+1):
                shift=ordinal*maximum//n
                proposals.append((proposal_key(request["rng_seed"],pivot,shift),pivot,shift))
        batches=[base]; placements={tuple(base)}
        for _,pivot,shift in sorted(proposals):
            if torch is None:
                candidate=[(i,s+(shift if p>=pivot else 0),e+(shift if p>=pivot else 0)) for p,(i,s,e) in enumerate(base)]
            else:
                schedule=torch.tensor(base,dtype=torch.int64,device="cpu")
                suffix=torch.arange(len(base),device="cpu") >= pivot
                schedule[:,1:] += suffix.to(torch.int64).unsqueeze(1)*shift
                candidate=[tuple(row) for row in schedule.tolist()]
            if tuple(candidate) in placements: continue
            placements.add(tuple(candidate));batches.append(candidate)
            if len(batches)==MAX_CANDIDATES: break
        def least_protected_first(a,b):
            av,bv=a.get("value",1.0),b.get("value",1.0)
            if av!=bv: return -1 if av<bv else 1
            ad=seconds(a["window"]["end"]) if a.get("window") else None
            bd=seconds(b["window"]["end"]) if b.get("window") else None
            if ad!=bd:
                if ad is None: return -1
                if bd is None: return 1
                return -1 if ad>bd else 1
            return (b["id"]>a["id"])-(b["id"]<a["id"])
        omissions=[{"task_id":t["id"],"reason":omitted.get(t["id"],"deferred_dependency")} for t in sorted((t for t in tasks if t["id"] in excluded),key=cmp_to_key(least_protected_first))]
    except _Failure:
        batches=[];omissions=[];window_start=0
    output={name:[] for name in ("task_index","slot_mask","start_time_offsets","duration_samples","piece_index","piece_count")}
    validity=[];slack=[];codes=[];dependency_feasibility=[];hard_constraint_feasibility=[]
    for c in range(MAX_CANDIDATES):
        batch=batches[c] if c<len(batches) else []
        n=len(batch);padding=MAX_PLANNING_TASKS-n
        output["task_index"].append([i for i,_,_ in batch]+[-1]*padding)
        output["slot_mask"].append([True]*n+[False]*padding)
        output["start_time_offsets"].append([s-window_start for _,s,_ in batch]+[0]*padding)
        output["duration_samples"].append([e-s for _,s,e in batch]+[0]*padding)
        output["piece_index"].append([1]*n+[0]*padding)
        output["piece_count"].append([1]*n+[0]*padding)
        candidate_steps={tasks[i]["id"]:(s,e) for i,s,e in batch}
        margins=[s-candidate_steps[dep][1] for i,s,_ in batch for dep in tasks[i].get("depends_on",[])]
        slack.append(min(margins) if margins else 0)
        dependency_feasibility.append(bool(batch) and all(m>=0 for m in margins))
        within=all(s>=window_start and e<=window_end and (not tasks[i].get("window") or s>=seconds(tasks[i]["window"]["start"]) and e<=seconds(tasks[i]["window"]["end"])) and (not tasks[i].get("static_anchor") or s==seconds(tasks[i]["static_anchor"]["start"])) for i,s,e in batch)
        if torch is None:
            disjoint=all(not(s<oe and e>os) for n,(_,s,e) in enumerate(batch) for _,os,oe in batch[n+1:])
        elif batch:
            times=torch.tensor([[s,e] for _,s,e in batch],dtype=torch.int64,device="cpu")
            overlaps=(times[:,0,None] < times[None,:,1]) & (times[:,1,None] > times[None,:,0])
            disjoint=not torch.triu(overlaps,diagonal=1).any().item()
        else:
            disjoint=True
        hard_constraint_feasibility.append(bool(batch) and within and disjoint and dependency_feasibility[-1])
        validity.append(c<len(batches));codes.append("ok" if c<len(batches) else failures["code"] if failures else "padding")
    output.update(validity_mask=validity,dependency_slack=slack,rejection_codes=codes,omissions=omissions,failure=failures,dependency_feasibility=dependency_feasibility,hard_constraint_feasibility=hard_constraint_feasibility)
    if torch is not None:
        for name in ("task_index","start_time_offsets","duration_samples","piece_index","piece_count","dependency_slack"):
            output[name]=torch.tensor(output[name],dtype=torch.int64,device="cpu").tolist()
        for name in ("slot_mask","validity_mask","dependency_feasibility","hard_constraint_feasibility"):
            output[name]=torch.tensor(output[name],dtype=torch.bool,device="cpu").tolist()
    return output

class _Failure(Exception):
    pass

def compute(payload):
    import torch
    if torch.__version__ != "2.6.0+cpu":
        raise ValueError("pinned CPU framework unavailable")
    return _solve(payload, torch)

def reference_without_framework(payload):
    """Test-only entry point; production compute never uses this fallback."""
    return _solve(payload)
