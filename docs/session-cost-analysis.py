#!/usr/bin/env python3
import json, glob, os, collections
from datetime import datetime

BASE = "/home/admin/.claude/projects/-home-admin-MatchyMatchy"

# Per-million-token rates (standard tier, USD). Cache multipliers applied to input rate.
RATES = {
    "claude-fable-5":            {"in": 10.0, "out": 50.0},
    "claude-opus-4-8":           {"in": 5.0,  "out": 25.0},
    "claude-sonnet-4-6":         {"in": 3.0,  "out": 15.0},
    "claude-haiku-4-5-20251001": {"in": 1.0,  "out": 5.0},
    "claude-haiku-4-5":          {"in": 1.0,  "out": 5.0},
}
CACHE_READ_MULT  = 0.10   # cached input read = 10% of base input
CACHE_W5M_MULT   = 1.25   # 5-min cache write = 125% of base input
CACHE_W1H_MULT   = 2.00   # 1-hour cache write = 200% of base input

IDLE_CAP = 300.0  # seconds; gaps longer than this are treated as idle and capped

def parse_ts(ts):
    return datetime.fromisoformat(ts.replace("Z", "+00:00"))

def price(model, fresh_in, read, w5m, w1h, out, geo_mult=1.0):
    r = RATES.get(model)
    if not r:
        return 0.0
    ci = r["in"] / 1_000_000.0
    co = r["out"] / 1_000_000.0
    cost = (fresh_in * ci
            + read * ci * CACHE_READ_MULT
            + w5m * ci * CACHE_W5M_MULT
            + w1h * ci * CACHE_W1H_MULT
            + out * co)
    return cost * geo_mult

def process_file(path, seen_ids):
    """Return per-model token+cost aggregation and list of timestamps for one jsonl."""
    agg = collections.defaultdict(lambda: dict(fresh_in=0, read=0, w5m=0, w1h=0, out=0, cost=0.0, msgs=0))
    timestamps = []
    agent_types = collections.Counter()
    user_turns = 0
    with open(path) as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                d = json.loads(line)
            except Exception:
                continue
            ts = d.get("timestamp")
            if ts:
                timestamps.append(ts)
            t = d.get("type")
            if t == "last-prompt":
                user_turns += 1
            msg = d.get("message") if isinstance(d.get("message"), dict) else None
            # capture Agent tool spawns (subagent_type) from assistant tool_use blocks
            if t == "assistant" and msg:
                content = msg.get("content")
                if isinstance(content, list):
                    for b in content:
                        if isinstance(b, dict) and b.get("type") == "tool_use" and b.get("name") == "Agent":
                            st = (b.get("input") or {}).get("subagent_type", "general-purpose")
                            agent_types[st] += 1
            if t != "assistant" or not msg:
                continue
            usage = msg.get("usage")
            model = msg.get("model")
            if not usage or not model or model == "<synthetic>":
                continue
            mid = msg.get("id")
            if mid and mid in seen_ids:
                continue  # dedupe: same API response split across multiple jsonl lines
            if mid:
                seen_ids.add(mid)
            fresh_in = usage.get("input_tokens", 0) or 0
            read = usage.get("cache_read_input_tokens", 0) or 0
            out = usage.get("output_tokens", 0) or 0
            cc = usage.get("cache_creation") or {}
            w5m = cc.get("ephemeral_5m_input_tokens", 0) or 0
            w1h = cc.get("ephemeral_1h_input_tokens", 0) or 0
            if not (w5m or w1h):
                # fall back to flat cache_creation_input_tokens, assume 5m
                w5m = usage.get("cache_creation_input_tokens", 0) or 0
            geo = usage.get("inference_geo")
            geo_mult = 1.1 if geo == "us" else 1.0
            c = price(model, fresh_in, read, w5m, w1h, out, geo_mult)
            a = agg[model]
            a["fresh_in"] += fresh_in
            a["read"] += read
            a["w5m"] += w5m
            a["w1h"] += w1h
            a["out"] += out
            a["cost"] += c
            a["msgs"] += 1
    return agg, timestamps, agent_types, user_turns

def active_seconds(timestamps):
    if len(timestamps) < 2:
        return 0.0, 0.0
    tss = sorted(parse_ts(t) for t in timestamps)
    span = (tss[-1] - tss[0]).total_seconds()
    active = 0.0
    for a, b in zip(tss, tss[1:]):
        g = (b - a).total_seconds()
        active += min(g, IDLE_CAP)
    return active, span

# Session label mapping (sessionId stem -> human label)
LABELS = {
    "dce700da": "Testbed build (golden + variants)",
    "084e6723": "M1 — implement",
    "1f2a8b9d": "M2 — implement",
    "af7bfbbb": "M2 — commit",
    "a19ef7e3": "M3 — implement",
    "d252f89a": "M4 — implement",
    "1b3f77ae": "M5 — implement",
    "dc24ae1c": "M6 — implement",
    "285dac11": "M7 — implement",
    "a9c0bcdb": "M8 — implement",
    "c6a19882": "Document-review (spec)",
    "2ef0c415": "README + LICENSE",
    "6b5dda71": "curl-install GH action / release",
    "0ad6c1f9": "New-test-case capability Q&A",
    "a5cf1e83": "This cost-analysis session",
}

results = []
seen_ids = set()  # global dedupe across all files
top_files = sorted(glob.glob(os.path.join(BASE, "*.jsonl")))
for tf in top_files:
    stem = os.path.basename(tf)[:-6]  # strip .jsonl
    short = stem[:8]
    sub_files = sorted(glob.glob(os.path.join(BASE, stem, "subagents", "agent-*.jsonl")))
    orch_agg, orch_ts, agent_types, user_turns = process_file(tf, seen_ids)
    sub_agg = collections.defaultdict(lambda: dict(fresh_in=0, read=0, w5m=0, w1h=0, out=0, cost=0.0, msgs=0))
    sub_ts = []
    for sf in sub_files:
        a, ts, _at, _ut = process_file(sf, seen_ids)
        for m, v in a.items():
            for k in v:
                sub_agg[m][k] += v[k]
        sub_ts.extend(ts)
    all_ts = orch_ts + sub_ts
    active, span = active_seconds(all_ts)
    orch_cost = sum(v["cost"] for v in orch_agg.values())
    sub_cost = sum(v["cost"] for v in sub_agg.values())
    results.append(dict(
        short=short, label=LABELS.get(short, short),
        orch_agg=orch_agg, sub_agg=sub_agg,
        orch_cost=orch_cost, sub_cost=sub_cost, total_cost=orch_cost + sub_cost,
        active=active, span=span,
        n_subagents=len(sub_files), agent_types=dict(agent_types),
        user_turns=user_turns,
        first_ts=min(all_ts) if all_ts else None, last_ts=max(all_ts) if all_ts else None,
    ))

# sort by first timestamp (chronological)
results.sort(key=lambda r: r["first_ts"] or "")

def fmt_tokens(n):
    if n >= 1_000_000: return f"{n/1_000_000:.1f}M"
    if n >= 1_000: return f"{n/1_000:.0f}k"
    return str(n)

def hms(sec):
    h = int(sec // 3600); m = int((sec % 3600) // 60)
    if h: return f"{h}h{m:02d}m"
    return f"{m}m"

# ---- print summary table ----
print(f"{'Session':<34}{'Active':>8}{'Span':>8}{'Subag':>6}{'OrchCost':>10}{'SubCost':>10}{'Total':>10}")
print("-"*86)
tot = dict(active=0,span=0,orch=0,sub=0,total=0,subag=0,turns=0)
for r in results:
    print(f"{r['label'][:33]:<34}{hms(r['active']):>8}{hms(r['span']):>8}{r['n_subagents']:>6}"
          f"{'$'+format(r['orch_cost'],'.2f'):>10}{'$'+format(r['sub_cost'],'.2f'):>10}{'$'+format(r['total_cost'],'.2f'):>10}")
    tot['active']+=r['active']; tot['span']+=r['span']; tot['orch']+=r['orch_cost']
    tot['sub']+=r['sub_cost']; tot['total']+=r['total_cost']; tot['subag']+=r['n_subagents']; tot['turns']+=r['user_turns']
print("-"*86)
print(f"{'TOTAL':<34}{hms(tot['active']):>8}{hms(tot['span']):>8}{tot['subag']:>6}"
      f"{'$'+format(tot['orch'],'.2f'):>10}{'$'+format(tot['sub'],'.2f'):>10}{'$'+format(tot['total'],'.2f'):>10}")

# ---- model-level totals ----
print("\n=== Cost & tokens by model (all sessions) ===")
model_tot = collections.defaultdict(lambda: dict(fresh_in=0, read=0, w5m=0, w1h=0, out=0, cost=0.0, msgs=0))
for r in results:
    for agg in (r['orch_agg'], r['sub_agg']):
        for m, v in agg.items():
            for k in v: model_tot[m][k]+=v[k]
print(f"{'Model':<28}{'Msgs':>7}{'FreshIn':>9}{'CacheRd':>9}{'CacheWr':>9}{'Out':>8}{'Cost':>10}")
for m, v in sorted(model_tot.items(), key=lambda x:-x[1]['cost']):
    print(f"{m:<28}{v['msgs']:>7}{fmt_tokens(v['fresh_in']):>9}{fmt_tokens(v['read']):>9}"
          f"{fmt_tokens(v['w5m']+v['w1h']):>9}{fmt_tokens(v['out']):>8}{'$'+format(v['cost'],'.2f'):>10}")

# ---- subagent type distribution ----
print("\n=== Agent (subagent) spawns by type ===")
at_tot = collections.Counter()
for r in results:
    for k,v in r['agent_types'].items(): at_tot[k]+=v
for k,v in at_tot.most_common(): print(f"  {k}: {v}")
print(f"  TOTAL Agent tool calls: {sum(at_tot.values())}; subagent transcript files: {tot['subag']}")

# ---- dump JSON ----
out = []
for r in results:
    out.append(dict(
        short=r['short'], label=r['label'],
        active_s=round(r['active']), span_s=round(r['span']),
        orch_cost=round(r['orch_cost'],4), sub_cost=round(r['sub_cost'],4), total_cost=round(r['total_cost'],4),
        n_subagents=r['n_subagents'], agent_types=r['agent_types'], user_turns=r['user_turns'],
        first_ts=r['first_ts'], last_ts=r['last_ts'],
        orch_by_model={m: {**v} for m,v in r['orch_agg'].items()},
        sub_by_model={m: {**v} for m,v in r['sub_agg'].items()},
    ))
with open("/tmp/session_analysis.json","w") as f:
    json.dump(dict(sessions=out, totals=tot,
                   model_totals={m:{**v} for m,v in model_tot.items()},
                   agent_type_totals=dict(at_tot)), f, indent=2)
print("\nWrote /tmp/session_analysis.json")
print(f"Total active wall-clock: {hms(tot['active'])}  ({tot['active']/3600:.1f}h)")
print(f"Total span (incl idle):  {hms(tot['span'])}  ({tot['span']/3600:.1f}h)")
print(f"Total user turns (last-prompt): {tot['turns']}")
