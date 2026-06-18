---
date: 2026-06-18
topic: progressive-disclosure-report
---

# Agent-First Progressive Disclosure for matchy Output

## Summary

Give matchy a progressive-disclosure view so a consuming agent's *first* read is a compact, recursive table-of-contents — region rollups, per-section counts, standalone real defects, and scores — with the gutted footer appearing as one line instead of dozens of rows, and cheap CLI drill-down to the full detail, which is never lost, only deferred to a finer level. Disclosure depth adapts to remaining work via a token budget rather than a hardcoded issue count, so the report "breathes": collapsed when a page is a mess, progressively fuller as the queue shrinks toward a clean match.

---

## Problem Frame

matchy is agent-first by intent — the README names the JSON `DiffResult` (entered through `agentSummary`) as the primary product, consumed as a shrinking queue (run → fix/accept → re-run → zero). But on a real migration pair the first read floods the consumer. The seed pair `testbed/pairs/p01-hiya-number-registration` produces ~272 issues; its `report.md` is ~27 KB (~7K tokens) and its `diff-result.json` is ~539 KB (~135K tokens). An agent paying that token tax on every re-run of every page is a direct cost: the number-registration migration ran matchy ~10 times across a ~75-minute visual-parity grind, at ~$39 / 158 turns — and that was the 3rd of 155 pages.

The region-saturation rollup shipped the *data* to fix this — `contentinfo` already collapses to a single region finding with its 88 members demoted out of `clusters`/`topFixes`. But the rendering still floods: `report.md` for p01 then walks "Issues by section" and prints all five `contentinfo` sub-tables anyway (PRODUCTS 26, "Start for free" 26, RESOURCES 14, CONSIDERING HIYA? 9, SOLUTIONS 8 = 83 rows). The tool summarized the footer *and* re-dumped it. More broadly, the output is grouped and rolled up but still delivered as one flat linear payload, with no way for the consumer to read the high-level shape first and expand only the branch its current task touches.

---

## Actors

- A1. **Consuming coding agent** — the primary optimization target. Reads the compact first view, expands only the branches relevant to its current fix, and wants minimal tokens with nothing unreachable.
- A2. **Human migration-QA reviewer** — triages divergences in the HTML report; expands branches visually to decide accept-vs-fix per section.
- A3. **matchy analyze / CLI layer** — emits the complete deterministic `DiffResult` archive and serves read-time projections of it. Performs no inference; "it's the footer" is the ARIA landmark, nothing more.

---

## Key Flows

- F1. **First-read triage with drill-down (agent)**
  - **Trigger:** `matchy analyze`/`run` completes on an old→new pair.
  - **Actors:** A3 emits; A1 consumes.
  - **Steps:** A1 reads the compact ToC first view → identifies the branch in scope for its current task (a region, a section, a cluster) → expands exactly that branch via the CLI → fixes or accepts → re-runs → as the queue shrinks the same budget inlines progressively more, until the first view shows everything.
  - **Outcome:** the consumer disposes of a saturated region in one decision and drills only where needed; real defects stay individually visible; token cost tracks remaining work.
  - **Escape path:** if a fine-grained issue handle has churned across runs (known ID-stability bug), the agent falls back to the stable region/section handle to reach the same content.
  - **Covered by:** R1, R2, R4, R5, R6, R7

- F2. **Visual triage (human, HTML)**
  - **Trigger:** A2 opens `report.html`.
  - **Actors:** A2.
  - **Steps:** sees collapsed rollups + section counts → expands branches of interest → drills to per-issue detail in place.
  - **Outcome:** the reviewer sees the page's shape at a glance and opens only what matters, mirroring the agent's structure.
  - **Covered by:** R10, R11, R12

---

## Requirements

**Compact first view**
- R1. matchy produces a compact, recursive table-of-contents as the first surface a consumer reads, leading with the highest-altitude work — region rollups, per-section counts, standalone real defects, and scores — rather than the full per-issue detail.
- R2. The first view is bounded by a calibrated token/size budget, not a hardcoded issue count. Inlined detail expands until the budget would be exceeded; below that, nodes collapse to a count plus a drill-down handle.
- R3. The collapse/expand boundary uses confidence bands, not a single knife-edge cutoff, so the rendered first view is deterministic on byte-identical bundles and a ±1-issue wobble near the boundary cannot flip a branch between collapsed and expanded across runs.
- R4. As the queue shrinks across re-runs, the same budget naturally inlines progressively more; below a small enough remaining count the first view inlines everything and no drill-down is needed.

**Lossless drill-down**
- R5. Every collapsed node carries a stable drill-down handle — landmark for regions, (landmark, nearest-heading) for sections, existing IDs for clusters/issues — that names exactly the branch to expand.
- R6. Drill-down is served by the CLI: a read-only command/flag expands a single branch from the run's existing output, reusing the hermetic read path `matchy explain` already uses — no browser, no network, no re-capture — and without loading the full result tree into the consumer's context.
- R7. No information is lost. Every issue present today stays present and reachable at a fine-grained level; disclosure changes only what is in the first linear payload, never what exists.

**Determinism & contract**
- R8. Progressive disclosure is a read-time projection over the complete `DiffResult`, not a new required contract field. The full JSON remains the complete, byte-deterministic archive; existing `diff-result.json` goldens do not change shape because of this feature.
- R9. The projection is computed from data already in the output (region rollups, clusters, by-section grouping, `agentSummary`); it adds no detection and no inference.

**Renderer parity (phased)**
- R10. Slice 1 (ships first): the markdown and HTML renderers honor the region-rollup demotion that already exists — a saturated region (e.g. `contentinfo`) collapses to a single line plus a drill-down pointer instead of re-printing its demoted member rows.
- R11. The HTML report provides human-facing collapse/expand, collapsed-by-default for large or saturated branches, mirroring the structure the agent sees.
- R12. Agent-native parity: any branch a human can expand in the HTML report, an agent can expand via the CLI, and vice versa.

---

## Acceptance Examples

- AE1. **Covers R1, R2, R10.** Given the p01 pair (`contentinfo` gutted, 177 fixable, ~272 issues), when matchy renders the first view, then `contentinfo` appears as a single rollup line with a drill handle, the standalone `broken_link` stays surfaced at the top, and the view fits the budget rather than printing the 83 footer rows.
- AE2. **Covers R3.** Given a section sitting one issue either side of the fold boundary, when matchy runs twice on byte-identical bundles, then the same branches are collapsed/expanded both times, and a ±1-issue wobble near the boundary does not flip a branch.
- AE3. **Covers R4.** Given a near-finished page with few remaining issues (below the budget), when matchy renders the first view, then it inlines all issues with no collapsed branches and no drill-down required.
- AE4. **Covers R5, R6, R7.** Given a collapsed `contentinfo` rollup, when the agent expands it by its handle via the CLI, then it receives exactly that region's member issues at full detail, hermetically, without loading the full result tree — and those issues were present in the archive all along.
- AE5. **Covers R8.** Given any existing variant/pair fixture, when this feature ships, then its `diff-result.json` golden is byte-identical to before (no contract shape change).

---

## Success Criteria

- On a real flooded pair (p01), the agent's first read drops from ~27 KB / thousands of tokens to a bounded compact view, lowering per-page migration triage cost — with no defect becoming unreachable.
- The footer renders as **one** work item in markdown and HTML (not 83 rows), and the real `broken_link` stays visible at the top.
- As a page nears a pixel-perfect match, the first view becomes fully inlined on its own — no mode switch, no flag to tune, no magic threshold.
- `ce-plan` can implement without inventing the disclosure trigger (budget + bands), the drill-down interaction (CLI branch expand), or the determinism guarantee (read-time projection over an unchanged archive).

---

## Scope Boundaries

- **Sharded per-section/per-region output files** — rejected. It fights the hermetic frozen-fixture / golden model (deterministic filenames, cleanup, integrity hashing); drill-by-handle covers the same need.
- **Baking the budgeted/collapsed view into `DiffResult` as a required field** — rejected; it would couple goldens to budget tuning. An *optional additive* machine-readable outline may be revisited (see Outstanding Questions) but is not required.
- **No new detection capability and no consumer-side "is this branch in scope for *my* task" reranking** — the tool stays neutral and deterministic; relevance judgment is the consumer's.
- **Not fixing the issue-ID-instability bug** — separate track. Drill handles rely on within-run stability plus stable region/cluster/section keys, so this feature does not depend on cross-run ID stability.
- **Not re-tuning the saturation metric, thresholds, or what rolls up** — this consumes the existing region rollup; it does not change it.

---

## Key Decisions

- **Read-time projection over a complete archive, not a new contract field** — preserves byte-determinism and golden stability; the adaptive view can evolve without contract churn. (Resolves the determinism concern: a shifting fold boundary is cosmetic because the complete artifact never changes and everything stays reachable.)
- **Token budget with confidence bands, not a hardcoded issue count** — adaptivity with no magic number and a deterministic, testable fold; it rides the shrinking-queue loop the product already centers on.
- **Drill-down via the CLI (branch expand), reusing the `explain`-style hermetic read** — the agent never reloads the full tree, and it extends an existing tool surface rather than inventing one.
- **Phase the work** — renderers honor the existing demotion first (cheap, immediate footer win); the budgeted recursive outline + CLI drill-down lands behind it.
- **Disclosure is structural plus budget-bounded** — saturated regions and oversized sections always collapse; the budget governs how much of everything else inlines.

---

## Dependencies / Assumptions

- Builds on the shipped region-saturation rollup — `regions` array, landmark-keyed IDs, member demotion out of `clusters`/`topFixes`, schemaVersion 1.2 (verified shipped).
- Stable drill handles exist: region landmark keys and cluster IDs are stable; (landmark, nearest-heading) section keys derive from anchors already on nodes; issue IDs are stable within a single run (sufficient for same-run drill-down). Cross-run issue-ID instability is a known separate bug and is deliberately not relied upon.
- The markdown renderer currently does **not** honor the rollup demotion — verified: p01 `report.md` prints all five `contentinfo` sub-tables (83 rows) despite the rollup line. Slice 1 (R10) closes this.
- If slice 1 changes the rendered-report goldens (`report.md` / `report.html`), that is a golden-discipline change (changelog entry + `golden-auditor` APPROVE per CLAUDE.md). The JSON `DiffResult` golden does not change (R8).
- The token-budget default is calibrated offline (like the saturation constants) — a planning-time calibration step, likely against p01 plus a second flooded page.

---

## Outstanding Questions

### Deferred to Planning

- [Affects R2][Needs research] Budget default and unit (token estimate vs char count vs item count) and how it maps to disclosure depth; calibrate against p01 plus a second flooded page.
- [Affects R6][Technical] Exact drill-down CLI surface: a new subcommand vs flags on `analyze`; how a branch handle is expressed on the command line across node kinds (region landmark / section key / cluster id / issue id), and whether one command covers all of them.
- [Affects R3][Technical] Precise band formulation for the fold boundary (what quantity is banded — cumulative budget consumption? per-node size?) so the collapse decision is provably stable across runs.
- [Affects R8][Technical] Whether to also emit an optional additive machine-readable outline in the JSON for non-CLI consumers, or keep the projection purely render/CLI-side. (Leaned render/CLI-side; revisit only if a JSON consumer needs it.)
- [Affects R1][Technical] What leads the compact first view and in what order (rollups, standalone errors, section counts, scores), and how it relates to or extends the existing `agentSummary` block.
