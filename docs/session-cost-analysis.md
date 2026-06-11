# MatchyMatchy — Claude Code session cost & risk analysis

_Generated 2026-06-11 from the raw `.jsonl` session transcripts under
`~/.claude/projects/-home-admin-MatchyMatchy/`. Covers the entire build of
page-pair-diff (testbed → M1–M8 → release), 15 top-level sessions plus their
80 subagent transcripts._

---

## 1. Headline numbers

| Metric | Value |
|---|---|
| **Total estimated API cost** | **≈ $318.62** |
| — orchestrator (main session) | $246.33 (77%) |
| — subagents (delegated work) | $72.29 (23%) |
| **Active wall-clock** (idle excluded) | **≈ 12.7 h** (12 h 41 m) |
| Total span incl. idle | 35.0 h |
| Sessions analyzed | 15 top-level + 80 subagent transcripts |
| Subagents spawned | 80 |
| User turns (prompts submitted) | 312 |
| Total billed input-equivalent tokens | 271.0 M |
| Total output tokens | 1.38 M |
| **Cache-read share of all input** | **95.5%** |

**The single most important cost driver is context size, not output.** Of 271 M
input-equivalent tokens, 258.8 M (95.5%) were *cache reads* — the large working
context (spec + contract + testbed + crate) re-read on essentially every turn.
Fresh input was only 0.24 M and output 1.38 M. This is why a long milestone costs
$30–50 even though the model "wrote" relatively little.

---

## 2. Methodology

### Pricing (per million tokens, standard tier, June 2026)

| Model | Input | Output | Cache read (0.1×) | Cache write 5m (1.25×) / 1h (2×) |
|---|---|---|---|---|
| Claude **Fable 5** | $10 | $50 | $1.00 | $12.50 / $20.00 |
| Claude **Opus 4.8** | $5 | $25 | $0.50 | $6.25 / $10.00 |
| Claude **Sonnet 4.6** | $3 | $15 | $0.30 | $3.75 / $6.00 |
| Claude **Haiku 4.5** | $1 | $5 | $0.10 | $1.25 / $2.00 |

Rates from Anthropic's published 2026 pricing (sources in §7). Cache multipliers
are the standard Anthropic ratios (read = 10% of base input, 5-minute write =
125%, 1-hour write = 200%); the transcripts record the 5m/1h split per message
(`usage.cache_creation.ephemeral_{5m,1h}_input_tokens`) so writes are priced at
the correct tier. `inference_geo` was `not_available` on every message, so no
US-region 1.1× multiplier was applied.

### Token accounting

- **Dedup by `message.id`.** Each API response is split across ~3 JSONL lines
  (one per content block) and the full `usage` object is repeated on each. Naïve
  summing triples the cost. All figures dedupe by `message.id` so each response
  is counted once.
- **Two transcript tiers combined.** Each session `<id>.jsonl` is the
  orchestrator. Its subagent runs live in `<id>/subagents/agent-*.jsonl` and are
  attributed to that session. Both tiers are included in every total.
- `<synthetic>` messages (local, no API call) are excluded.

### Wall-clock (idle-excluded)

Active time = Σ min(gap, 300 s) over consecutive timestamped events in the
session (orchestrator + subagent events merged). Any gap longer than 5 minutes
is capped at 5 minutes, which discards human-idle stretches ("Claude finished but
it took me 30 minutes to notice") while still crediting genuine long tool runs up
to 5 min. The raw span (first→last timestamp) is reported alongside for contrast;
the gap between the two columns is almost entirely idle.

### Limitations of this analysis

- Active-time capping is a heuristic; a legitimate build/test run longer than 5 min
  is under-counted. In practice such runs were delegated to the test-runner
  subagent and rare.
- Cost is an **estimate** at list price. It ignores any subscription/plan
  bundling, batch discounts (not used here), and assumes the published per-token
  rates above.
- Subagent role is inferred from the orchestrator's `Agent` tool-call
  `subagent_type`, not re-derived from each subagent transcript.

---

## 3. Per-session breakdown (chronological)

| Session | Orchestrator model | Active | Span | Subagents | Orch $ | Subagent $ | **Total $** |
|---|---|--:|--:|--:|--:|--:|--:|
| Document-review (spec) | Fable 5 | 51 m | 1 h 04 m | 6 | 13.41 | 3.99 | **17.40** |
| Testbed build (golden + variants) | Fable 5 | 49 m | 49 m | 9 | 13.61 | 9.62 | **23.23** |
| README + LICENSE | Fable 5 | 11 m | 31 m | 0 | 1.41 | 0.00 | **1.41** |
| M1 — implement | Fable 5 | 52 m | 55 m | 5 | 24.28 | 5.03 | **29.31** |
| M2 — implement | Fable 5 | 54 m | 54 m | 8 | 23.09 | 6.81 | **29.90** |
| M2 — commit | Fable 5 | 2 m | 2 m | 1 | 1.06 | 0.02 | **1.08** |
| M3 — implement | Fable 5 | 56 m | 58 m | 10 | 27.72 | 5.96 | **33.68** |
| M4 — implement | Fable 5 | 1 h 20 m | 1 h 20 m | 10 | 37.15 | 14.36 | **51.51** |
| M5 — implement | Fable 5 | 1 h 13 m | 1 h 29 m | 6 | 17.34 | 6.66 | **23.99** |
| M6 — implement | Fable 5 | 1 h 55 m | 2 h 02 m | 11 | 39.60 | 11.80 | **51.40** |
| M7 — implement | **Opus 4.8** | 1 h 05 m | 3 h 43 m | 7 | 19.63 | 4.85 | **24.49** |
| M8 — implement | **Opus 4.8** | 1 h 10 m | 7 h 27 m | 5 | 18.48 | 2.88 | **21.37** |
| curl-install / release | Opus 4.8 | 1 h 00 m | 13 h 21 m | 1 | 7.61 | 0.03 | **7.64** |
| New-test-case Q&A | Opus 4.8 | 7 m | 7 m | 1 | 0.93 | 0.27 | **1.19** |
| This cost-analysis session | Opus 4.8 | 7 m | 7 m | 0 | 1.03 | 0.00 | **1.03** |
| **TOTAL** | | **12 h 41 m** | 35 h 00 m | **80** | **246.33** | **72.29** | **318.62** |

The two most expensive sessions are **M4 ($51.51)** and **M6 ($51.40)** — M4
because of heavy subagent use ($14.36, the highest) chasing the cross-origin
`url()` false-positive bug, and M6 because it was the longest active session
(1 h 55 m) and ran the real-pair calibration gate plus a spec-numbering correction.

---

## 4. Cost & tokens by model

| Model | Role | Msgs | Fresh in | Cache read | Cache write | Output | **Cost** |
|---|---|--:|--:|--:|--:|--:|--:|
| **Fable 5** | orchestrator (testbed, M1–M6) | 722 | 160 k | 96.7 M | 3.9 M | 807 k | **$212.96** |
| **Sonnet 4.6** | code-implementer, fixture-builder | 1 669 | 22 k | 107.7 M | 4.7 M | 179 k | **$52.69** |
| **Opus 4.8** | orchestrator (M7, M8, release) + auditor | 307 | 52 k | 41.2 M | 2.1 M | 341 k | **$49.99** |
| **Haiku 4.5** | test-runner | 507 | 2 k | 13.2 M | 1.1 M | 56 k | **$2.98** |

**Fable 5 is 67% of total cost** despite being one model on the main thread,
because it carried testbed + M1–M6 at the premium $10/$50 rate, and Fable output
is billed at $50/M. Sonnet processed the *most* tokens (107.7 M cache reads — the
implementers re-read large briefs) but at 1/3 the rate costs a quarter as much.

### The Fable → Opus switch paid off

M1–M6 ran the orchestrator on **Fable 5**; M7–M8 switched to **Opus 4.8**
(half the per-token price). For comparable milestone work the orchestrator cost
roughly halved:

- Fable milestones, orch-only: M3 $27.72, M4 $37.15, M6 $39.60
- Opus milestones, orch-only: M7 $19.63, M8 $18.48

Same scope of work, ~50% cheaper, with no quality regression visible in the
transcripts (M7/M8 both shipped green with auditor-approved goldens). **If cost
matters, Opus 4.8 is the better default for this orchestration workload**; reserve
Fable for the design/architecture sessions where its edge is worth $10/M.

---

## 5. Subagent delegation — the model-routing policy in practice

The CLAUDE.md cost-control policy (frontier orchestrator, mechanical work to
cheaper subagents) **was followed heavily**: 80 subagents spawned across the build,
$72 of the $319 (23%) pushed down to cheaper tiers.

| Subagent type | Spawns | Typical model |
|---|--:|---|
| test-runner | 30 | Haiku 4.5 |
| code-implementer | 23 | Sonnet 4.6 |
| golden-auditor | 10 | Fable 5 / Opus 4.8 |
| fixture-builder | 9 | Sonnet 4.6 |
| Explore | 2 | (inherited) |
| document-review personas (6 types) | 6 | (inherited) |

This is the policy working as designed: 30 cheap Haiku test-runs ($2.98 total
across *all* Haiku work), 32 Sonnet implementer/fixture builds doing the bulk
token-heavy file work at $3/M, and the expensive auditor kept on the frontier
model where its judgment matters. Without delegation, the test-running and
file-building alone (≈121 M tokens on Sonnet+Haiku) would have cost far more on
Fable.

---

## 6. Open risks, big questions & unfinished items

Collected from the wrap-up reports and mid-session diagnoses of each milestone.
Most are deliberate, documented carry-forwards rather than defects. Grouped by
theme; the originating milestone is noted.

### 6.1 Deliberately deferred to post-v1 (per spec §7.3 / §9 / §14)

- **Capability detectors** — `missing_capability`, `nonfunctional_capability`,
  `changed_capability`, `capability_added` are spec-reserved for post-v1 and
  **must not be emitted in v1**. This tripped up M6: `prompts/step2-implement-goal.md`
  predated spec v3 and demanded capability probes; the spec won and they were not
  built. A header warning was added to the milestone playbook so future sessions
  never paste a goal demanding them. (M6)
- **Capability-only parity profile** — v1 ships exactly two profiles; the
  capability-only third profile is deferred with the capability differ. (M6)
- **Other post-v1 parked items** named at M8 close: auth/authed capture,
  `locale_parity_missing`, interactive HTML report (filters, region-jump), and
  multi-page crawl. (M8)
- **`missing_form` / `missing_form_field`** — the golden page has no static
  `<form>` (HubSpot injects it at runtime), so these paths are unit-tested on
  synthetic bundles only and need a future **synthetic fixture pair**.
  `duplicate_text`, `changed_cta`, `missing_submit`, `changed_form`,
  `changed_required_field` likewise deferred with rationale (design doc D10).
  v08 covers G2 via missing-CTA instead. (Testbed, M3)
- **"Redirect where none should occur"** hygiene clause — deferred until the §14
  config file exists. (M2)

### 6.2 Calibration & threshold risk

- Many constants were **chosen defaults, "advisory until the M6 real-pair gate"**:
  `matchFloor`/`noMatchCeil` and the image-src pre-pass (D5) (M3);
  `SEQ_MIN_DISPLACEMENT=2` and structure-issue severity (M5). **M6 froze these**
  against three real pairs (Wayback-2025 vs live, live vs live → zero false
  positives, golden vs live) with evidence in `docs/calibration-note.md` — so this
  risk is largely **discharged**, with two residuals:
  - **Mobile viewport remains uncalibrated** until first production use. (M6)
  - **M7's three new confidence constants are NOT under the M6 freeze** —
    annotated as M7-introduced, calibratable at real use. (M7)

### 6.3 Known modeling limitations (spec-accepted)

- **Cross-block moves** (e.g. a heading demoted to a styled `<div>`) surface as
  `missing` + `added` rather than a move, and heading-demotion specifically reads
  as `missing_text` — kind-blocking and spec-mandated (design doc D1). (M3, M6)
- **`added_*` types** will need added-side duplicate-label suppression to ship
  with them — recorded as a carry-forward so a future `added_*` feature doesn't
  regress orphan-label false positives. (M6)
- **a11y diff is rule-level** for v1 (per §11 "violation sets"); per-node a11y
  delta across a DOM rewrite is a noted post-v1 refinement. (M7)
- **`console_error` filtering** excludes "Failed to load resource" lines via an
  English-text prefix match (deterministic but locale-scoped); correlating by
  message-location URL is future hardening. (M7)

### 6.4 The one live reliability risk

- **v20 capture-layer asset-404 flake** (M8) — a non-deterministic asset 404 in
  the *capture* layer, i.e. a §3.3 capture-nondeterminism leak, **not** an analyze
  bug. Flagged at M8 close as "the one open risk to harden." The analyze layer is
  pure/byte-deterministic; this is the capture side occasionally not fetching an
  asset deterministically. **This is the highest-value thing to fix** because the
  whole golden-discipline regime assumes deterministic capture.

### 6.5 Bugs found & fixed during the build (for the record — not open)

- **Cross-origin/port `url()` absolutization → 16 false positives on the control**
  (M4): `norm_href` returned absolute paths despite its doc comment; fixed by
  page-directory-relative canonicalization over two iterations. The `norm_href`
  doc-comment divergence was left untouched to protect recorded goldens (noted as
  an M3 divergence). (M4)
- **Network differ keyed correlation on the origin root** → ~6 symmetric
  dangling-asset 404 false positives on the URL-hygiene variants (v14/v15/v16,
  served under a path prefix). A URL migration *is* a base-path change, so the
  correlation was re-keyed relative to each page's own URL directory. (M7)
- **Fixture-server bug** (M2): v15/v16 emitted spurious `visual_region_changed`
  because 77/78 asset requests failed — the serve scripts didn't map the
  parent-directory prefix that relative assets resolve under. Fixed in server code;
  no expectation touched.
- **UTF-8 truncation panic + threshold-boundary bug** surfaced during the M5 v08
  regression — both fixed and unit-tested. (M5)

### 6.6 Housekeeping / loose ends

- **`norm_href` doc comment is wrong** (says it returns page-relative, returns
  absolute) — knowingly left as-is to avoid churning goldens. Worth a real fix +
  audited golden change later. (M4)
- **Testbed ships a percent-encoded broken footer image on both sides** —
  renaming it later shifts node ordinals and requires an audited golden change. (M3)
- **`locale` stamping is inconsistent across emitters** (`component_swapped` emits
  `null`, visual issues emit `"en-US"`); auditor asked to unify. Partially handled
  in M6 ("sequence-diff locale stamping"). (M5)
- **Untracked artifacts flagged as probably-don't-commit**: `ppd-claude-kit.zip`
  (packaging artifact) and `prompts/tmp.md` (scratch). (README session)
- **Build plan is complete**: spec §12 runs M1→M8 with no M9; G1–G8 each have a
  passing fixture and the M6 calibration gate passed — the two §15 conditions for
  v1 "done". (M8)

---

## 7. Sources

- [Claude API pricing — official docs](https://platform.claude.com/docs/en/about-claude/pricing)
- [Claude Fable 5 — OpenRouter pricing](https://openrouter.ai/anthropic/claude-fable-5)
- [Claude Fable 5 pricing explained (2026)](https://www.ayautomate.com/blog/claude-fable-5-pricing-explained)
- [Claude API Pricing 2026: Opus 4.8, Sonnet 4.6, Haiku 4.5](https://www.metacto.com/blogs/anthropic-api-pricing-a-full-breakdown-of-costs-and-integration)
- [CloudZero — Claude Mythos / Fable 5 pricing 2026](https://www.cloudzero.com/blog/claude-mythos-pricing/)
- [TechCrunch — Anthropic releases Claude Fable 5 (2026-06-09)](https://techcrunch.com/2026/06/09/anthropic-released-claude-fable-5-its-most-powerful-model-publicly-days-after-warning-ai-is-getting-too-dangerous/)

_Raw per-session data: `docs/session-cost-data.json`. Reproducible analysis
script: `docs/session-cost-analysis.py` (run against
`~/.claude/projects/-home-admin-MatchyMatchy/`)._
