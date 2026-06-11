# Step 2 — Implementation: prompts to paste

Run one goal per milestone rather than one mega-goal for the whole spec. Each milestone has a
crisp DoD in spec §12, which makes the evaluator's job (and your auditing job) tractable, and a
fresh session per milestone keeps Fable's context clean. (`/goal` allows one active goal per
session anyway.)

## Per-milestone pattern (repeat for M1 → M8)

> **Numbering note (2026-06-11):** this file originally predated spec v3, which deferred
> capability probes to post-v1 and renumbered §12 (M6 = real-pair calibration, M7 = a11y +
> network, M8 = reporters/profiles/migration loop). The M6–M8 entries below now match spec v3;
> never paste a goal that demands capability detectors/probes or the capability issue types —
> spec §7.3 reserves those for post-v1 and CLAUDE.md makes the spec authoritative.

```
/implement-milestone M1
```
then immediately:
```
/goal Milestone M1 of docs/prds/page-pair-diff-spec.md is done per its DoD: docs/design/M1.md exists covering modules, types, contract deltas, and acceptance criteria; contract/capture-bundle.schema.json and contract/diff-result.schema.json exist with TS zod schemas and Rust serde structs validated against them in `make verify`; the capture package drives Playwright Chromium with the section 4.2 stabilization steps (animation kill, time/RNG freeze, fonts ready, lazy-load pass, hide/mask selectors) recorded in CaptureBundle.determinism, and produces a schema-valid CaptureBundle plus full-page and viewport screenshots for any two URLs; the analyze binary consumes two bundles and emits old.png, new.png, diff.png, the page-height delta, and a DiffResult that validates against contract/diff-result.schema.json with visual_region_changed and page_height_changed issues populated including region bounding boxes; running ppd against golden localhost:3000 vs variant v01-identical on :3001 yields status pass with zero issues, and golden vs v02-banner-added yields a visual_region_changed whose region overlaps the banner plus a nonzero page-height delta, with both runs' output shown; `make verify` output has been shown exiting 0; a determinism spot-check has been shown passing (same two bundles analyzed twice produce a byte-identical DiffResult excluding runId and timestamp fields); and any changes to expected-issues.json files or goldens have an APPROVE verdict from golden-auditor recorded in docs/golden-changelog.md. Surface all verification output in conversation as proof. Stop and report instead of continuing if blocked on the same failure for 3 consecutive turns, or after 80 turns.
```

---

## M2 — URL & locale hygiene (G5, G6)

```
/implement-milestone M2
```
then immediately:
```
/goal Milestone M2 of docs/prds/page-pair-diff-spec.md is done per its DoD: docs/design/M2.md exists; spec section 10 is implemented in full in the analyze layer (trailing-slash policy never/always/preserve, redirect-chain detection with the full chain in evidence, protocol-downgrade detection, canonical mismatch, and BCP-47 locale-path validation covering case, separator, and unknown codes) with the capture layer recording finalUrl and redirectChain; the testbed contains all four hygiene variants — if a redirect-chain variant or an es-mx lowercase-region variant is missing, they have been added via fixture-builder with manifest.json and hand-authored expected-issues.json like the existing v14 trailing-slash and v15 es_MX variants; fixture runs shown in conversation demonstrate that the trailing-slash variant yields url_trailing_slash, the redirect-chain variant yields url_redirect_chain with the chain listed, the es_MX variant yields locale_separator_invalid with remediation from /es_MX/... to /es-MX/..., and the es-mx variant yields locale_case_invalid with remediation from /es-mx/... to /es-MX/..., each with correct structured remediation from and to fields; Rust unit tests cover the locale parser and URL normalization including at least es-MX valid, es_MX, es-mx, ES-mx, and an unknown code; `make verify` output has been shown exiting 0; the determinism spot-check has been shown passing; and any expectation changes have an APPROVE verdict from golden-auditor recorded in docs/golden-changelog.md. Surface all verification output in conversation. Stop and report instead of continuing if blocked on the same failure for 3 consecutive turns, or after 80 turns.
```

---

## M3 — Semantic extraction + content diff (G2)

```
/implement-milestone M3
```
then immediately:
```
/goal Milestone M3 of docs/prds/page-pair-diff-spec.md is done per its DoD: docs/design/M3.md exists and documents the chosen per-kind weight tables, matchFloor and noMatchCeil defaults, and the assignment strategy with its deterministic tie-break; the capture layer extracts the ordered SemanticNode stream per sections 4.3 and 6.1 (visible nodes only, anchors populated including nearestHeading, landmark, and ordinalInLandmark) plus title, meta description, canonical, lang, headings, text blocks with section 11 normalization, links, images with alt and load status, and forms with fields and labels; the analyze layer implements section 6 matching with kind-blocked weighted scoring, constrained assignment, and confidence bands writing per-signal sub-scores into evidence.match for every emitted issue; content, link, image, and form issues from the section 7.3 taxonomy are emitted with anchor-set locators and remediation; fixture runs shown in conversation demonstrate that v08-form-removed yields missing_form, v09-h1-changed yields changed_h1 with from and to text in evidence, v10-paragraph-removed yields missing_text, v11-broken-link yields broken_link, and v13-render-equivalent yields zero missing or added issues for the rewrapped element with its evidence.match sub-scores shown proving the matcher paired it; v01-identical still yields status pass with zero issues; Rust unit tests cover text normalization, similarity scoring, and the matcher including a render-equivalent pairing case; `make verify` output has been shown exiting 0; the determinism spot-check has been shown passing; and any expectation changes have an APPROVE verdict from golden-auditor recorded in docs/golden-changelog.md. Surface all verification output in conversation. Stop and report instead of continuing if blocked on the same failure for 3 consecutive turns, or after 80 turns.
```

---

## M4 — Computed-style diff + anchor locator (G1, G4)

```
/implement-milestone M4
```
then immediately:
```
/goal Milestone M4 of docs/prds/page-pair-diff-spec.md is done per its DoD: docs/design/M4.md exists; the capture layer performs scoped computed-style capture per section 4.4 (candidate elements only, the curated property set, colors normalized to a canonical rgb form so hex and named colors do not false-diff, and background-image gradients parsed into kind, angle, and stops); the analyze layer emits style_changed, background_gradient_lost, and background_gradient_changed on matched pairs with property-level from and to in evidence and remediation carrying findBy grep targets built from the anchor set; fixture runs shown in conversation demonstrate that v06-gradient-removed yields background_gradient_lost with the original linear-gradient string as from and the flat replacement as to, v05-spacing-color-change yields style_changed with the exact properties and from/to values, and v03-font-size and v04-font-family each yield style_changed on the affected elements; the string of every emitted DiffResult has been grepped to show no source component or framework component name is claimed anywhere, only anchor sets; cssSelectorOld/New remain internal relative selectors and anchors are the agent-facing locator; Rust unit tests cover color normalization and the gradient parser including linear, radial, angle, and stop deltas; `make verify` output has been shown exiting 0; the determinism spot-check has been shown passing; and any expectation changes have an APPROVE verdict from golden-auditor recorded in docs/golden-changelog.md. Surface all verification output in conversation. Stop and report instead of continuing if blocked on the same failure for 3 consecutive turns, or after 80 turns.
```

---

## M5 — Sequence diff (G3)

```
/implement-milestone M5
```
then immediately:
```
/goal Milestone M5 of docs/prds/page-pair-diff-spec.md is done per its DoD: docs/design/M5.md exists; the analyze layer implements section 8 sequence diff over matched pairs using their seqIndex values with an LCS or edit-script computation, emitting component_reordered for relative-order changes and collapsing a mutual A-B position exchange into a single component_swapped carrying both locators, with remediation.action reorder_components including target, before and after anchors, and the expected order list; the fixture run shown in conversation demonstrates that v07-sections-swapped yields exactly one component_swapped and zero missing, added, or duplicate component_reordered issues for those two sections, and the rest of the page produces no order issues; v01-identical and v13-render-equivalent still pass unchanged, with output shown, proving sequence diff introduced no regressions; Rust unit tests cover the LCS or edit-script logic including a swap, a single move, an identity sequence, and a deterministic tie-break case; `make verify` output has been shown exiting 0; the determinism spot-check has been shown passing; and any expectation changes have an APPROVE verdict from golden-auditor recorded in docs/golden-changelog.md. Surface all verification output in conversation. Stop and report instead of continuing if blocked on the same failure for 3 consecutive turns, or after 80 turns.
```

---

## M6 — Real-pair calibration gate ✅ done 2026-06-11

Completed (commit `bd795bd`): three real pairs run and triaged, all matcher/visual constants
frozen unchanged, four FP-class fixes (C1–C4) landed under golden discipline. Record:
`docs/calibration-note.md`, `docs/design/M6.md`, `calibration/`. If thresholds ever change,
recalibrate per spec §12 M6 against the archived-pair procedure in the note; the goal below is
kept only for such a re-run.

```
/goal Milestone M6 of docs/prds/page-pair-diff-spec.md is done per its DoD: docs/design/M6.md exists; the tool has been run end-to-end against at least one real old/new pair of the actual target page (not a testbed fixture), with the capture bundles archived and their SHA-256s recorded; every issue in the resulting DiffResult has been triaged into true-positive (defect or drift), false-positive (tool or noise), or artifact buckets with the triage tables shown in conversation, and there are zero unexplained missing or added issues; matcher weights, identityFloor, matchFloor/noMatchCeil, and the visual thresholds have each been confirmed or tuned against the observed false positives and negatives, with a written calibration note in docs/ recording the findings and the final frozen values, and config.rs annotated that the defaults are frozen; a live-vs-live noise-floor run has been shown producing zero issues; a determinism double-analyze of the real-pair bundles has been shown byte-identical modulo runId; `make verify` output has been shown exiting 0 across all testbed fixtures; and any expectation or golden changes have an APPROVE verdict from golden-auditor recorded in docs/golden-changelog.md. Surface all verification output in conversation. Stop and report instead of continuing if blocked on the same failure for 3 consecutive turns, or after 80 turns.
```

---

## M7 — A11y + network diffs (G8)

```
/implement-milestone M7
```
then immediately:
```
/goal Milestone M7 of docs/prds/page-pair-diff-spec.md is done per its DoD: docs/design/M7.md exists; the capture layer records axe-core results for both pages plus network request outcomes and console messages in the CaptureBundle per section 4.3; the analyze layer emits accessibility_regression, accessibility_improved, network_error, and console_error per sections 7.3 and 11, scoring new-only network and console failures against the new page while failures present on both pages are noted but not scored against it; the testbed contains the variants this milestone needs — v12-image-404 already covers the newly-404ing asset, and if variants for a new console error and a seeded accessibility regression do not exist they have been added via fixture-builder with manifests and hand-authored expected-issues.json audited by golden-auditor; fixture runs shown in conversation demonstrate the 404-asset variant yields network_error naming the failed URL, the console-error variant yields console_error with the message in evidence, and the seeded-a11y-regression variant yields accessibility_regression; no capability issue types are emitted anywhere (missing_capability, nonfunctional_capability, changed_capability, and capability_added are reserved post-v1 per section 7.3); `make verify` output has been shown exiting 0; the determinism spot-check has been shown passing; and any expectation changes have an APPROVE verdict from golden-auditor recorded in docs/golden-changelog.md. Surface all verification output in conversation. Stop and report instead of continuing if blocked on the same failure for 3 consecutive turns, or after 80 turns.
```

Expect v12's golden to legitimately churn when network_error starts co-firing alongside
broken_image — its intent file already allows either; the golden-discipline clause covers the
audited re-record.

---

## M8 — Reporters, profiles, migration loop

```
/implement-milestone M8
```
then immediately:
```
/goal Milestone M8 of docs/prds/page-pair-diff-spec.md is done per its DoD: docs/design/M8.md exists; the analyze layer renders the same DiffResult to static HTML (side-by-side screenshots and a fix-value-ordered issue list per section 7.2 — no interactive filters or region-jump navigation per section 2, all page-derived strings HTML-escaped with a restrictive CSP and no inline event handlers per section 15), Markdown, and JSON, with the JSON issues array sorted by fix value as the agent work queue and the agentSummary block first in the file; the two v1 parity profiles strict-visual and content-structure are implemented per the section 9 severity table (capability-only stays deferred post-v1) with explicit per-type severity config overriding profile defaults; the --baseline accept-list keyed on stable issue ids suppresses matching issues from issues and from scoring/status while counting them in suppressed with their ids, demonstrated by baselining a real issue and re-running; deterministic clustering per section 7.4 groups issues sharing type plus changed style property, or type plus landmark, at clusterMin or larger, demonstrated by a seeded global-style defect producing one cluster referencing all member issues with agentSummary.clusterCount correct and topFixes able to reference a cluster id; the CLI supports the section 14 flags including --profile, --fail-on, --json, --html, and --markdown, and exits 0 on pass, 1 on a failed --fail-on threshold, and 2 on tool or runtime error; a single run against a styled variant such as v06-gradient-removed has been shown producing all three report formats, and the same run has been shown under content-structure versus strict-visual demonstrating the profile switch changes the pass or fail status as specified, plus an exit-code demonstration of 0, 1, and 2 cases; the full fixture suite across all variants has been run with a pass table shown and every variant green against its expected-issues.json and recorded golden; the section 15 invariant checklist has been walked item by item in conversation with evidence for each, confirming every goal G1 through G8 has at least one passing fixture and that the M6 real-pair calibration gate is recorded complete in docs/calibration-note.md; `make verify` output has been shown exiting 0; the determinism spot-check has been shown passing; and any expectation changes have an APPROVE verdict from golden-auditor recorded in docs/golden-changelog.md. Surface all verification output in conversation. Stop and report instead of continuing if blocked on the same failure for 3 consecutive turns, or after 80 turns.
```

## The "refine expectations honestly" instruction

This lives in CLAUDE.md (golden discipline) + the golden-auditor agent, so the goal loop is
already constrained. If you want it explicit in a goal, append:

```
Expectations in expected-issues.json and goldens/ may only change with a golden-auditor APPROVE and a changelog entry; closing a gap by weakening an expectation without those counts as NOT done.
```

## Between milestones (you, manually)

- `git log --stat` the milestone branch; read `docs/golden-changelog.md` — every entry should
  smell like "expectation was wrong," not "code was hard."
- Run `make verify` yourself once. Trust, then verify the verifier.
- `/clear` (which also clears the goal) and start the next milestone.
