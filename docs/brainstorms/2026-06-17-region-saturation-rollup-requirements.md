---
date: 2026-06-17
topic: region-saturation-rollup
---

# Region-Saturation Rollup

## Summary

Add a deterministic **region-saturation rollup** to matchy's analyze layer: a new top-level `regions` array in `DiffResult`. When an ARIA-landmark region's *structural* damage crosses a calibrated threshold, matchy emits one region-level finding (e.g. "`contentinfo` is largely unmatched") and demotes that region's member issues to drill-down — so a consuming coding agent receives the footer as a single work item instead of dozens of scattered line-items, while standalone real defects stay surfaced on their own.

---

## Problem Frame

matchy's `DiffResult` is excellent on synthetic single-change fixtures but floods on real migration pairs. The Tier-3 seed `testbed/pairs/p01-hiya-number-registration` (a Webflow-staging → localhost rebuild) produces **272 issues / 25 clusters**. Clustering already covers most of them, but it stops one altitude too low: it groups by `(landmark × property)`, so the footer's damage is fragmented across two landmark clusters plus ~10 cross-region property clusters. The tool already *knows* it's the footer (the ARIA `contentinfo` role is on every anchor) but never says so as one finding.

This is not hypothetical. The `number-registration` port (session `b2564cfe`) ran matchy ~10 times to drive fixes; the visual-parity phase alone was ~75 min of repeated CSS cycles ending at "we are hitting dimensioning returns," at ~$39 / 158 main-session turns — **and that was the third of 155 pages.** Reducing DiffResult triage burden is a direct lever on per-page migration cost.

Calibration on p01 (desktop) shows why the footer is the story and why the *kind* of metric matters:

| landmark | old nodes | new nodes | structural | style | **struct / oldN** | tot / oldN |
|---|---:|---:|---:|---:|---:|---:|
| **contentinfo** | 51 | 10 | 45 | 43 | **0.88** | 1.73 |
| main | 60 | 60 | 1 | 58 | **0.02** | 1.25 |
| navigation | 8 | 4 | 4 | 56 | 0.50 | 7.50 |
| banner | 4 | 3 | 2 | 28 | 0.50 | 7.75 |
| (none) | 7 | 0 | 6 | 0 | 0.86 | — |

The footer lost 41 of 51 nodes; `main` lost none (it was restyled, not gutted). A **structural-by-node** metric separates them with enormous margin (0.88 vs 0.02). The intuitive **by-issue** metric (`tot/oldN`) inverts the truth — it ranks the restyled `navigation` (7.5×) and `banner` (7.75×) *above* the actually-missing footer (1.7×).

---

## Actors

- A1. **Consuming coding agent** — reads `DiffResult` to drive migration fixes. The primary optimization target; wants region-altitude work items, not a 272-line wall, and minimal tokens.
- A2. **Human migration-QA reviewer** — triages divergences, decides accept-vs-fix per region, and signs off (the `reviewedBy` on the accepted-divergence ledger).
- A3. **matchy analyze layer** — the deterministic tool that emits `regions`. Performs no inference; the "it's the footer" knowledge is the ARIA landmark role, nothing more.

---

## Key Flows

- F1. **Region-aware migration triage**
  - **Trigger:** `matchy analyze` runs on a captured/frozen old→new pair.
  - **Actors:** A3 emits; A1/A2 consume.
  - **Steps:** analyze computes per-region structural saturation → emits `regions[]` and demotes each saturated region's members → `agentSummary` leads with region rollups + standalone real defects → consumer makes one decision per region (reconcile now / accept-defer / fix a standalone defect) → drills into member issue IDs only when a region is in scope for the current task.
  - **Outcome:** the consumer disposes of a saturated region in a single decision; real defects remain individually visible.
  - **Covered by:** R4, R6, R8, R9

---

## Requirements

**Saturation metric & trigger**
- R1. matchy computes, per ARIA landmark region, a **structural-saturation ratio** = (old-side nodes in the region that are missing, broken, or structurally changed) / (old-side semantic nodes anchored to that region). Style-only churn is excluded from the numerator.
- R2. A region rollup is emitted when structural saturation ≥ a **saturation threshold** (calibrated default `0.6`) **and** the region has ≥ a **minimum old-side node count** (calibrated default `10`). Both are tunable constants, calibrated offline against Tier-3 fixtures and then frozen.
- R3. The metric, thresholds, and emission are fully deterministic and live in the analyze layer: byte-identical bundles → byte-identical `regions`; no map-iteration-order dependence, total-ordered tie-breaks, fixed-order float reductions (spec §15). No LLM or inference anywhere in the tool.

**Region object & output shape**
- R4. `DiffResult` gains a new top-level `regions` array, distinct from `clusters`. Each region rollup carries: the landmark, the saturation ratio with its numerator/denominator evidence, the member issue IDs, a severity/status, and a human-readable summary.
- R5. Each region rollup's `id` is derived from the **landmark** (ordinal-independent), **not** from a hash of its member issue IDs — so the rollup id survives re-captures even while per-issue IDs churn.
- R6. A saturated region **claims all issues anchored to it** (structural *and* style). Claimed issues are demoted: they remain in the `issues` array for drill-down but are referenced by the region and no longer counted as independent line-items in `clusters` / `topFixes`.
- R7. The `regions` array is **purely additive**: the schema is versioned additively, and when no region saturates (e.g. single-change variant fixtures), `regions` is empty and all other output is unchanged.

**Consumer-facing summary**
- R8. `agentSummary` leads the consumer with the highest-altitude work first — region rollups and standalone real defects ahead of the long tail. `topFixes` may reference region ids (as it already may reference cluster ids), and a region count is exposed.
- R9. A standalone real defect is **never silently swallowed** by a rollup: a defect outside any saturated region stays a top-level item, and an error/critical-severity member inside a saturated region remains individually reachable so the agent does not lose it under the rollup.

**Calibration fixture**
- R10. p01's `expected-issues.json` evolves from a raw `maxIssues` cap to a **top-level-work-item assertion**: ≤ N top-level items (standalone issues + clusters + region rollups), exactly one of which is a `contentinfo` region rollup at saturation ≥ threshold, with the `broken_link` true-positive still asserted and unswallowed.
- R11. Region-rollup assertions reuse the existing `expected-issues` matcher vocabulary (which already has `sharedLandmark`); no second matcher implementation. (`check-pair.py` continues to import `check-fixture.py`'s engine.)

---

## Acceptance Examples

- AE1. **Covers R1, R2.** Given the p01 pair where `contentinfo` dropped 51→10 nodes with 45 structural issues (0.88) and `main` is 60→60 with 1 structural (0.02), when analyze runs, then exactly one region rollup is emitted — for `contentinfo` — and none for `main`.
- AE2. **Covers R2.** Given a region with only 4 old nodes even at high structural saturation (e.g. `banner`), when analyze runs, then no rollup is emitted because the region is below the minimum node count.
- AE3. **Covers R6.** Given `contentinfo` saturates, when the rollup is emitted, then the footer's `style_changed` issues are claimed by the region and are no longer independent members of the global `display`/`color` property clusters (those clusters shrink accordingly).
- AE4. **Covers R7.** Given a single-change variant fixture (one deliberate change, no saturated region), when analyze runs, then `regions` is empty and `issues`/`clusters`/`agentSummary` are byte-identical to today apart from the new empty field.
- AE5. **Covers R9.** Given the `broken_link` in `main` (an unsaturated region), when analyze runs, then it remains an independently surfaced top-level work item regardless of any region rollups elsewhere.

---

## Success Criteria

- A consuming coding agent receives the footer as **one** work item: top-of-output item count on p01 drops from ~272 issues to a small number of top-level items (region rollups + remaining clusters + standalone defects), one of which is the `contentinfo` rollup.
- The real `broken_link` defect is still visible at the top, not buried.
- The structural-vs-by-issue separation holds: `main` (restyled) does not roll up; `contentinfo` (gutted) does.
- Output stays byte-deterministic and golden-stable; single-change variants are unaffected beyond the additive field.
- `ce-plan` can implement from this doc without inventing the metric, the trigger, the output shape, or the p01 assertion.

---

## Scope Boundaries

- **Not** fixing the issue-ID-stability bug (only 2/129 ids survived a re-run on p01) — separate parallel track. The region key (R5) is deliberately designed not to depend on it.
- **No** consumer-side LLM reranking skill ("is the footer in scope for *my* task?") — separate effort. The tool emits neutral deterministic rollups only.
- **No** new detection capability — this aggregates existing detections; it is not a G1–G8 detection goal.
- **No** rollup for the no-landmark `(none)` chrome tail (notification bar, Weglot) — those ~6 issues stay individual; landmark-keyed rollups cannot capture them.
- **Not** re-litigating the M9 Tier-3 milestone.
- **Not** changing or depending on the separate `extract-webflow-styles.sh` styleCompare tool or its per-selector ledger.

---

## Key Decisions

- **Structural-by-node saturation, not by-issue.** Calibration showed by-issue ranks restyled small regions (nav 7.5×, banner 7.75×) above the gutted footer (1.7×); structural-by-node cleanly separates `contentinfo` (0.88) from `main` (0.02).
- **Neutral region object — not a fix-item, not an accept-handle.** A deterministic tool cannot infer intent; the consumer decides fix-vs-accept. The rollup carries evidence + members so it is usable both by `topFixes` (fix framing) and by suppression (accept framing).
- **Region keyed on landmark, ordinal-independent.** A durable handle that survives re-capture even while per-issue ids churn — explicitly decoupled from the ID-stability bug.
- **New top-level `regions` array, not a `clusters` extension.** Clusters mean "same root-cause fix"; regions mean "semantic territory." Overloading muddies both; a separate additive array preserves backward-compat.
- **Membership demotes the whole region (structural + style).** Once the footer is a single reconcile/rebuild unit, its style churn is subsumed under that work, not separately actionable.
- **Trace: §7.4 migration-loop support**, alongside clusters and the baseline accept-list — not a G1–G8 detection goal. (The plan's "scoped by a G-goal" rule applies to downstream *detection* fixes; this is output aggregation.)
- **Constants frozen on p01, validated in planning** (decision 2026-06-17). `0.6` / `10` are locked now given p01's wide 0.88-vs-0.02 margin; a second-page calibration is a planning-time validation step, not a pre-freeze blocker.

---

## Dependencies / Assumptions

- Each semantic node carries its landmark via the anchor set — **verified**: nodes expose `anchors.landmark`; the p01 old bundle has 130 nodes with a clean per-landmark distribution.
- Calibration constants are **frozen at the p01-derived values** (`0.6` / `10`); assumes the 0.88-vs-0.02 separation generalizes. A second-page calibration (home / branded-call, which share the cross-page chrome divergence) is a planning-time validation step rather than a pre-freeze blocker.
- Changing p01's assertion (R10) and re-recording affected variant goldens are **golden-discipline changes**: each requires a `docs/golden-changelog.md` entry plus a `golden-auditor` APPROVE verdict (per CLAUDE.md). Adding a brand-new fixture would not, but these touch existing expectations/goldens.
- The contract change must be validated in **both** TS zod and Rust serde against `/contract/*.schema.json` in CI (spec §15).

---

## Outstanding Questions

### Deferred to Planning

- [Affects R2][Needs research] Second-page calibration: capture/run home or branded-call and confirm the structural-vs-restyle separation holds; adjust the frozen `0.6` / `10` only if it doesn't.
- [Affects R6][Technical] Exact precedence between region-claim and the existing property→landmark cluster precedence: does a saturated region claim its members before or after clustering, and how do the affected property clusters recompute?
- [Affects R4][Technical] Region rollup severity/status derivation (worst-member vs saturation-derived), and how the new `regions` relate to the existing `scores.byLandmark`.
- [Affects R9][Technical] The exact severity floor (error? critical?) above which a member defect inside a saturated region stays individually surfaced rather than fully demoted.
- [Affects R1][Needs research] Whether "structurally changed" should include `changed_link_target` / `changed_link_text` or be missing/broken-only. p01's `contentinfo` is missing-dominated so both agree there; a second page may discriminate (it is what pushes nav/banner to 0.50).
