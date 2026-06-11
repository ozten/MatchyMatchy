# 2026-06-11 field-test bugs — 5-whys root cause analysis and fix plan

Twelve reports (see INDEX.md) from matchy's first real migration gate. This document traces
them to root causes via 5-whys chains, then defines the work packages that fix them. The
packages do not map 1:1 onto the reports: several reports share one root cause, and one root
cause sometimes demands a fix wider than any single report asked for.

## 5-whys chains

### Chain 1 — "the scores are meaningless" (bugs 04, 05, 07, 12, and the noise half of 06)

1. Why are scores and error counts untrustworthy on a real page? Because they are dominated
   by issues that do not represent real regressions (style score 0.0012 with 1592 issues).
2. Why do non-regressions become error issues? The engine emits an issue whenever two
   captured byte strings differ — computed CSS literals (`start` vs `left`, invisible border
   colors), naturalWidth integers, URL schemes.
3. Why is byte-inequality equated with regression? There is no semantic-equivalence layer
   (CSS value canonicalization, environment awareness, intent modes) and no confidence
   gating between *detection* and *scoring* — uncertain pairings count the same as certain ones.
4. Why was that layer never built? The pipeline was calibrated exclusively on the testbed,
   where old and new are the same engine, same server, same environment — there, literal
   equality *is* a valid proxy for visual equality.
5. Why didn't calibration surface the gap? The testbed has no cross-engine
   (Webflow vs Next.js), cross-environment (https vs http://localhost) pair.

**Root cause 1: detection is conflated with judgment.** The pipeline lacks a
normalization/equivalence/confidence layer between "values differ" and "issue counts
against the score", and nothing in the test fleet ever exercised the conditions that make
the difference visible. (The Tier-3 real-pair fixtures PRD, already drafted, is the
long-term guard for the last why.)

### Chain 2 — "silent integrity failure" (bugs 01, and the outlier half of 03)

1. Why did developers spend 8 fix rounds chasing phantom diffs? The baseline capture was
   corrupted and nothing said so.
2. Why was the corruption silent? Stabilizer step failures are recorded as a buried bundle
   field (`determinism.lazyLoadPass: "failed"`) and never promoted to run-level output.
3. Why is there no promotion? The DiffResult contract has no warnings/degraded channel;
   capture was designed best-effort: log, continue, diff whatever came out.
4. Why best-effort? Capture assumed a failed *enhancement* step still leaves a usable page.
5. Why is that assumption wrong and unchecked? Page JS interacts with the injected clock in
   DOM-mutating ways (Swiper's `slideTo` fired by a fake timer mid-init), and no integrity
   self-check compares the page inventory before/after stabilization.

**Root cause 2: analyze trusts capture unconditionally.** There is no capture-integrity
contract — no retry policy on stabilizer-induced page breakage, no warnings channel, no
inventory check. (A retry exists today, but only for the navigation-timeout case.)

### Chain 3 — "identity built on volatile bytes" (bug 02)

1. Why don't `--baseline` accept-lists hold? Issue ids drift between runs.
2. Why do ids drift? The id hash includes the raw captured `href`, which includes
   nondeterministically-injected tracking params (`__hstc` …).
3. Why is the raw href in the hash? Id derivation reuses the anchors verbatim; anchors were
   designed for grep-ability (exact bytes), not stability.
4. Why no normalization at the identity boundary? URL normalizers exist (`norm_href`) but
   were built for *matching*; nobody specified which anchor fields are identity-grade.
5. Same bottom as chain 1: captured bytes were trusted as stable because on the testbed
   they are.

**Root cause 3: no normalization boundary at identity derivation** — a sub-case of root
cause 1 applied to ids instead of scores.

### Chain 4 — "results aren't organized the way pages are" (bugs 06, 10, 11)

1. Why does triage require external python over diff-result.json? Reports and scores are
   page-global flat lists (1791-row table; one style score for the whole page).
2. Why flat? Renderers iterate `issues[]` in fix-value order; aggregation never uses
   `locator.anchors.landmark` / `nearestHeading`, although every issue carries them.
3. Why unused? Anchors were designed for issue *location*; no milestone required
   structural *aggregation*; and `1/(1+n)` scoring was tuned on testbed pages with <30
   issues.
4. Why does that break on real pages? At 1592 issues the score saturates
   (1/(1+1592)≈0.0006): the formula has no resolution left, and shared chrome contributes
   68–83% of all errors on every page of a page-by-page migration.

**Root cause 4: the aggregation model ignores the structural dimension (landmark/section)
that the data already carries, and count-based global scores saturate at real-page issue
volumes.**

### Chain 5 — "works only from the repo root with make-exported env" (bugs 08, 09)

1. Why does the installed binary fail outside the repo? `capture.cjs` resolution is
   CWD-relative and browser resolution depends on a Makefile-exported env var.
2. Why? The resolution candidates encode the dev layout, and `doctor` answers a different
   question ("can Playwright find *some* chromium") than capture asks ("is the pinned
   headless-shell build present in the configured cache").
3. Why the mismatch? Environment assumptions live in the Makefile, not in the binary's own
   resolution and diagnostics.

**Root cause 5: implicit environment assumptions are not encoded in the binary** (chain
bottoms out at three whys; the cause is already actionable).

### Synthesis

The deepest shared root (chains 1–3, feeding 4): **matchy 0.1 was built and calibrated
against its own deterministic, same-engine testbed, so the design silently equates
"captured bytes" with "ground truth" and "byte difference" with "regression".** First
contact with a real cross-engine, live-CDN page pair broke every layer that relied on that
equivalence — capture integrity (01), identity (02), determinism (03), pairing confidence
(04), value equivalence (05), scope relevance (06/12), and aggregation scale (06/10/11).

## Work packages

Fixes are grouped by root cause, not by report. Contract changes are concentrated in one
package (WP-E) and ship as schemaVersion 1.1 (additive). All goldens re-record once at the
end, with a golden-changelog entry and golden-auditor verdict.

| WP | Root cause | Fixes bugs | Scope |
|----|-----------|-----------|-------|
| A | 5 | 08, 09 | `resolve_capture_script` ancestor walk + actionable error; stderr banner condensing + single browser-not-found remedy; doctor verifies pinned executable path |
| B | 1 | 12 | Loopback-host protocol-downgrade issues → info severity, honest remediation note |
| C | 1 | 05 | Computed-value canonicalization before compare: `border`/`outline` with style `none` ≡ `none`; `text-align` start→left / end→right (LTR); `line-height: normal` ≡ font-size×1.2 (±0.5px) |
| D | 3 | 02 | Issue-id hash normalizes URLs to scheme+host+path (query+fragment dropped); output anchors unchanged |
| E | 1+2+4 | 02, 04, 06, 12 (scoring), 03 (visibility) | Contract 1.1: `warnings[]`, `scopedTo`, `outOfScope`, `scores.byLandmark`; `--scope` flag; uncertain-pairing gate (band ≠ matched or score < 0.75 → info + excluded from style score); category scores exclude info-severity issues; baseline-staleness warning |
| F | 4 | 10 | report.md grouped by landmark › nearestHeading with viewport folding; per-section counts in Summary; warnings/uncertain/out-of-scope sections; html warnings banner |
| G | 4 | 11 | Capture `landmarkRects` (additive bundle field); `page_height_changed` gains `evidence.sectionDeltas` |
| H | 2 | 01, 03 | Stabilizer: retry-without-freeze on ANY step failure under frozen clock; pre/post-stabilization inventory in determinism; integrity warning; `--self-check` double-capture volatility mode |
| I | 1 | 07 | `--image-dims-mode responsive`: aspect-preserving downscales ≥ rendered width → info; upscales/aspect-changes still error |

Execution order (file-conflict-driven): Wave 1 = A∥B∥C∥D → Wave 2 = E → Wave 3 = F∥G∥I →
Wave 4 = H → full `make verify`, golden re-record + audit + changelog.

Deliberately not done (and why):
- Bug 03's median-of-N capture and auto-`--mask` suggestion: deferred — `--self-check` +
  the warnings channel give visibility into residual live-page volatility; smoothing it is
  a feature with real design surface, not a bug fix.
- Bug 04's selector-depth plausibility heuristic: the band/score gate covers the reported
  population (106 of 208 issues band-null); depth heuristics risk false suppression.
- Profile-TOML rule configurability (bugs 04/05/07 asides): no TOML profile system exists;
  constants in `config.rs` until profiles grow a config file.
