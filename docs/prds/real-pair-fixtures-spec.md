# Real-Pair Regression Fixtures — Spec (testbed Tier 3)

> **Audience:** an agentic coding tool implementing this feature against the page-pair-diff repo.
> **Status:** proposed feature spec (authored 2026-06-11). **Subordinate to** `docs/prds/page-pair-diff-spec.md` (the v3 build spec); where this document conflicts with it, the build spec wins. Section references like "§7.3" point at the build spec unless prefixed "this spec".
> **One-line mission:** turn any real old/new URL pair where `matchy` missed a defect (false negative) or flooded noise (false positive) into a **frozen, deterministic, asserted** regression fixture that locks the corrected behavior in forever.
> **Relationship to existing work:** this operationalizes the one-time **M6 real-pair calibration** artifacts (build spec §12) into a permanent, CI-gated **third testbed tier**. It introduces no new analysis capability — it wraps the already-built `matchy analyze --old-bundle/--new-bundle` replay path in fixtures and a harness.

---

## 0. Problem statement (why this exists)

The user's intended loop, in their words: *"As I use this tool, I will hit old and new URL examples where matchy misses things. We can then add this to our test bench and fix the tool to detect these issues."*

That loop is **capture a real-world failure → freeze it → write the intent → fix the code → keep it green forever.** The repo does not support it today:

| | What it is | Captures real URLs? | Asserted automatically? | Committed & replayable? | In `make verify`? |
|---|---|---|---|---|---|
| **Tier 1 — permutation variants** (`testbed/variants/v01..v21`) | Frozen golden + exactly one synthetic change, served from localhost | No — synthetic mutations of one golden | Yes (`expected-issues.json`) | Sites committed; captured live each run | Yes |
| **Tier 2 — M6 calibration pairs** (`calibration/r1..r3`) | Two arbitrary live/archived URLs, run once to tune constants | Yes | **No** — human-triaged in `docs/calibration-note.md` | **No** — bundles are gitignored (`calibration/.capture/`); only a one-shot `diff-result.json` is kept | No |
| **Tier 3 — real-pair fixtures** *(this spec)* | Two real captures **frozen into the repo**, replayed offline and asserted | Yes | **Yes** (`expected-issues.json`, reused contract) | **Yes** — bundles committed; byte-deterministic replay | Yes |

**The gap is the convenience tier, not the engine.** The hard part is already built: `matchy analyze --old-bundle PATH --new-bundle PATH --out DIR` (see `packages/analyze/src/bin/matchy.rs`) analyzes directly from two saved `CaptureBundle` files with **zero capture, zero network, zero browser** — and is byte-deterministic by the §15 invariant. A live URL "with slightly wrong details" is non-deterministic and drifts or vanishes (M6's R1 accreted 491 issues over a year of live drift). The fix is **capture once, freeze both bundles into the repo, and replay them forever.** This spec defines the fixtures, manifest, harness, goldens, authoring tooling, and privacy rules around that primitive.

---

## 1. Goals & non-goals

### Goals
- **G-R1.** Let a contributor add a regression fixture from two real URLs with one command, and have it become a permanent, CI-gated test.
- **G-R2.** Make the fixture **deterministic and hermetic**: it replays from frozen bundles, so it runs in CI even where Chromium/Playwright/the testbed servers are absent.
- **G-R3.** Encode **intent, not current output** — a fixture for a missed defect must be **red on purpose** until the code is fixed (false-negative TDD); a fixture for a noise flood asserts a ceiling (false-positive regression).
- **G-R4.** Reuse the existing matcher contract (`testbed/schemas/expected-issues.schema.json`) and golden machinery unchanged wherever possible; add the minimum new surface.
- **G-R5.** Keep committed captures **safe to commit** — redaction-clean, PII-reviewed, size-budgeted.
- **G-R6.** Provide a clean promotion path: a Tier-2 calibration pair that exposed a bug can become a Tier-3 fixture.

### Non-goals (v1 of this feature)
- **No live re-capture in CI.** Fixtures replay frozen bundles only. Re-capture is an explicit, manual `pair-refresh` action.
- **No crawling, auth, or interaction.** Same constraints as build spec §2 / §15. Capturing the pair uses the existing `matchy` capture path with its existing flags.
- **No new analysis features.** If a fixture is red because matchy genuinely cannot yet detect the defect, fixing that is downstream work scoped by whichever build-spec goal (G1–G8) it falls under — not part of this feature.
- **No change to the matcher DSL semantics.** The `expected-issues.json` contract is reused as-is; only its *application domain* widens from variants to pairs.
- **Not a replacement for Tier 1.** Synthetic single-change variants remain the precise, minimal feature tests. Tier 3 is for real-world-shaped inputs the synthetic golden cannot model.

---

## 2. Anatomy of a frozen real-pair fixture

```
testbed/pairs/
  <case-id>/                     # e.g. p01-acme-pricing-gradient
    old.bundle.json              # COMMITTED frozen CaptureBundle for the "old" page
    new.bundle.json              # COMMITTED frozen CaptureBundle for the "new" page
    pair.json                    # provenance + integrity manifest (this spec §3)
    expected-issues.json         # intent matchers — reuses testbed/schemas/expected-issues.schema.json
    baseline.json                # OPTIONAL committed --baseline accept-list for this pair
testbed/goldens/
  <case-id>.diffresult.json      # OPTIONAL byte-exact golden, recorded after the fixture goes green
testbed/.runs/
  <case-id>/diff-result.json     # working output (gitignored, like variant runs)
```

Rules:
- **`<case-id>`** is `p<NN>-<slug>` (e.g. `p01-acme-pricing-gradient`). The `p` prefix guarantees no collision with Tier-1 `v<NN>-…` names, which matters because pair goldens share `testbed/goldens/` and the Makefile golden loop keys on `basename`.
- **Exactly two committed bundles per case**, named `old.bundle.json` / `new.bundle.json`. These are the source of truth; they are never regenerated by `make verify`.
- A pair captures **one coherent real-world observation** (one missed defect, or one false-positive flood). Like a Tier-1 variant it should be tightly scoped so the assertion is legible — but unlike a variant it may legitimately contain incidental real-world drift, which is declared in `pair.json.knownDrift` and pinned out with `forbidden`/`maxIssues` matchers.

---

## 3. `pair.json` — provenance & integrity manifest

Real pairs come from the wild, so the fixture must record **where each capture came from, when, under what capture config, and with what content hash** — otherwise a committed bundle is an unauditable blob. A new schema `testbed/schemas/pair.schema.json` validates it.

```json
{
  "caseId": "p01-acme-pricing-gradient",
  "description": "Hero CTA lost its linear-gradient background in the new build; matchy emitted no style issue (false negative).",
  "demonstrates": "false-negative",
  "discoveredVia": "manual matchy run during the acme.com migration, 2026-06-09; see issue #142",
  "goals": ["G4", "G1"],
  "profile": "content-structure",
  "viewport": "desktop=1440x1000",
  "old": {
    "url": "https://web.archive.org/web/20250603143211if_/https://acme.com/pricing",
    "finalUrl": "https://acme.com/pricing",
    "capturedAt": "2026-06-09T17:22:04Z",
    "sha256": "<64-hex of old.bundle.json>",
    "chromiumBuild": "148.0.7778.0"
  },
  "new": {
    "url": "https://acme.com/pricing",
    "finalUrl": "https://acme.com/pricing",
    "capturedAt": "2026-06-09T17:22:51Z",
    "sha256": "<64-hex of new.bundle.json>",
    "chromiumBuild": "148.0.7778.0"
  },
  "captureFlags": ["--hide", ".cookie-banner,.chat-widget", "--mask", ".timestamp"],
  "baseline": null,
  "knownDrift": [
    "new page added a testimonials section below the fold — declared, pinned out via forbidden/maxIssues"
  ],
  "frozen": true,
  "refreshPolicy": "never"
}
```

| Field | Required | Purpose |
|---|---|---|
| `caseId` | yes | Matches the directory name; `^p[0-9]{2,}-[a-z0-9-]+$`. |
| `description` | yes | One sentence: what real-world defect/observation this pins. |
| `demonstrates` | yes | `"false-negative"` \| `"false-positive"` \| `"true-positive"` \| `"mixed"`. Drives review expectations. |
| `discoveredVia` | yes | Provenance: how the pair was found (free text; link an issue if one exists). |
| `goals` | yes | G-codes (build spec §1) the pair exercises. Array of `^G[1-8]$`. |
| `profile` | yes | Parity profile to analyze under (build spec §9). The harness passes `--profile`. |
| `viewport` | yes | The viewport baked into the bundles. Informational for replay (analyze reads it from the bundle), required for re-capture reproducibility. |
| `old` / `new` | yes | Per-side provenance. `url` (as requested), `finalUrl` (post-redirect, from the bundle), `capturedAt` (ISO-8601 UTC), `sha256` of the committed bundle file, `chromiumBuild` from the bundle's environment fingerprint. |
| `captureFlags` | yes | Exact `matchy` capture flags used, so `pair-refresh` reproduces the capture. Empty array if none. |
| `baseline` | no | Relative path to a committed `--baseline` accept-list for this pair, or `null`. |
| `knownDrift` | no | Human-declared incidental differences that are NOT the defect under test (the analog of a variant manifest's `knownOnEffects`). Each must be pinned out by a `forbidden`/`maxIssues` matcher. |
| `frozen` | yes | Always `true` in v1; reserved for a future "live" mode. |
| `refreshPolicy` | yes | `"never"` (default) \| `"on-demand"`. Documents whether re-capture is ever expected. |

**Integrity rule (hard):** the harness recomputes the SHA-256 of `old.bundle.json` and `new.bundle.json` and **fails loudly** if either differs from `pair.json`. This catches accidental edits, partial commits, LFS mishaps, or line-ending corruption. Updating a bundle is therefore a deliberate act: re-run `pair-refresh`, which rewrites both the bundle and its recorded hash (and is itself a golden-discipline event — see §5).

---

## 4. `expected-issues.json` — intent (reused contract)

Pairs **reuse `testbed/schemas/expected-issues.schema.json` verbatim** — the same `status` / `required` / `forbidden` / `maxIssues` / `clusters` / `notes` matchers Tier-1 variants use. No new matcher grammar.

What changes is the **authoring discipline**, because a real pair is added precisely *because the tool is currently wrong about it*:

- **False-negative case (matchy missed a defect).** Write a `required` matcher describing the issue the tool *should* emit (type from the §7.3 taxonomy + minimal anchors/evidence). The fixture is **red on purpose** until the analysis code is fixed. This is the TDD entry point the user asked for. The required matcher is **authored from intent** — what the page actually got wrong — **never scraped from the current (wrong) output.**
- **False-positive case (matchy flooded noise).** Add `forbidden` matchers for the bogus issue classes and/or a `maxIssues` ceiling. The fixture is red until the matcher/scoring is tightened. This is the regression analog of M6's R2/R3 noise-floor checks.
- **True-positive / mixed case.** Use `required` for the genuine diffs that must always be caught, `forbidden`/`maxIssues` for the declared `knownDrift`, and a `status` assertion. Useful for promoting a calibration pair into a standing guard.

**Golden-discipline alignment (CLAUDE.md, non-negotiable).** A failing real-pair fixture is, by default, **a bug in the code, not in the expectation** — identical to Tier-1 rules. An `expected-issues.json` for a pair may only be weakened/changed when the *expectation itself* was wrong (over-specified, contradicted the spec), and only with: (1) a `docs/golden-changelog.md` entry citing the spec section, and (2) an APPROVE verdict from the `golden-auditor` subagent. Adding a brand-new red fixture is **not** a golden change and needs no auditor sign-off — it is the normal way to file "matchy should catch this."

---

## 5. The replay-and-assert harness

A new script `testbed/check-pair.py`, the Tier-3 sibling of `check-fixture.py`. It **must reuse `check-fixture.py`'s matcher engine** (the `_type_matches` / `_substring` helpers and the required/forbidden/maxIssues/clusters/status evaluator), not reimplement it — import them, or shell out to `check-fixture.py` in its existing `--expected`/`--diff-result` "unit mode" (which already evaluates matchers with no servers and no `matchy` run). Reimplementing the matcher would let the two tiers drift.

`python3 testbed/check-pair.py <case-id> [--matchy PATH] [--skip-run]` does:

1. **Load & validate** `testbed/pairs/<case-id>/pair.json` against `pair.schema.json`.
2. **Integrity check** — recompute SHA-256 of `old.bundle.json` / `new.bundle.json`; abort (exit 2) on mismatch with `pair.json`.
3. **Replay** (unless `--skip-run`): invoke the byte-deterministic offline path
   ```
   matchy analyze \
     --old-bundle testbed/pairs/<case-id>/old.bundle.json \
     --new-bundle testbed/pairs/<case-id>/new.bundle.json \
     --out testbed/.runs/<case-id>/ \
     --profile <pair.json.profile> \
     [--baseline testbed/pairs/<case-id>/<pair.json.baseline>]
   ```
   Output `testbed/.runs/<case-id>/diff-result.json`. **No servers, no Playwright, no network.**
4. **Schema-validate** the emitted `diff-result.json` against `/contract/diff-result.schema.json` (as `check-fixture.py` does).
5. **Evaluate** `expected-issues.json` via the shared matcher engine.
6. **Exit codes:** `0` all matchers satisfied; `1` an assertion failed (the actionable red state); `2` harness/tool error (bad manifest, hash mismatch, `matchy analyze` non-{0,1} exit, schema violation).

**Hermeticity is a feature.** Because Tier-3 runs entirely from frozen JSON, it can run in the `cargo test` / unit lane and in minimal CI images, and it strengthens the **byte-exact analysis-golden suite** (build spec §13.3) with real-world-shaped inputs — exactly the suite that proves the §15 determinism invariant.

---

## 6. Byte-exact goldens for pairs

Because replay is deterministic (identical bundle bytes → identical `DiffResult` modulo `runId`/timestamps), pair goldens belong to the **byte-exact** analysis-golden suite, not the tolerant end-to-end suite.

- After a fixture goes green, optionally record `testbed/goldens/<case-id>.diffresult.json` (the recorded output with `runId`/timestamps excluded).
- The **existing** Makefile golden step already globs `testbed/goldens/*.diffresult.json` and diffs each against `testbed/.runs/<basename>/diff-result.json` via `compare-golden.py` (excludes `runId`/timestamps, float tolerance 1e-4). With the `p`-prefixed case-id and the harness writing to `testbed/.runs/<case-id>/`, **pair goldens are picked up with no Makefile change** — provided the pair harness runs *before* the golden-comparison step (see §7).
- Re-recording a pair golden after an approved behavior change is fine; the `docs/golden-changelog.md` entry covers it (same rule as Tier 1).

---

## 7. Authoring workflow & Makefile surface

Three new `make` targets; one insertion into `verify`.

**Add a fixture (capture + freeze + scaffold):**
```
make pair-add CASE=p01-acme-pricing-gradient \
     URL_OLD="https://web.archive.org/.../https://acme.com/pricing" \
     URL_NEW="https://acme.com/pricing" \
     [PROFILE=content-structure] [VIEWPORT=desktop=1440x1000] \
     [HIDE=".cookie-banner"] [MASK=".timestamp"]
```
This:
1. Runs the live capture path `matchy --old URL_OLD --new URL_NEW --out <tmp> …` once (the only step that touches the network/browser).
2. **Freezes** the two produced `*.bundle.json` files into `testbed/pairs/<CASE>/old.bundle.json` / `new.bundle.json`.
3. Computes SHA-256s and writes a `pair.json` scaffold (URLs, `finalUrl`, `capturedAt`, `chromiumBuild`, flags, hashes).
4. Runs `matchy analyze` on the frozen bundles to seed `testbed/.runs/<CASE>/diff-result.json`.
5. Writes an **`expected-issues.json` STUB** — `status` + **empty** `required`/`forbidden` + a `notes` reminder. **It does NOT auto-populate `required` from the current output**, because the current output is presumed wrong (that's why the pair is being added). The human/orchestrator then authors the intent per §4.
6. Runs the redaction & PII gate (§8); refuses to leave bundles in place if it trips.

**Run one fixture:** `make pair CASE=p01-acme-pricing-gradient` → `python3 testbed/check-pair.py p01-acme-pricing-gradient`.

**Refresh a fixture (deliberate re-capture):** `make pair-refresh CASE=…` re-runs capture with the recorded `captureFlags`, rewrites both bundles and their hashes. This is a golden-discipline event: note it in `docs/golden-changelog.md`.

**Wire into `verify`:** add a step **before** the existing "golden comparisons" step that iterates committed pairs:
```
@echo "=== N/…  real-pair regression fixtures (Tier 3) ==="
@for d in testbed/pairs/*/ ; do \
    case=$$(basename "$$d"); \
    python3 testbed/check-pair.py "$$case" || exit 1; \
done
```
Ordering matters: the pair harness must write `testbed/.runs/<case>/diff-result.json` before the golden loop diffs it. Because Tier 3 needs no servers, this step can run even when `testbed-up` was skipped.

---

## 8. Privacy, redaction & repo weight

Committing real captures to a shared repo raises two risks the build spec already arms us against — this feature must enforce them at the freeze boundary.

- **Redaction is mandatory and verified.** The capture layer already redacts `Authorization`/`Cookie`/`Set-Cookie` headers and known token-bearing query params (build spec §14, "Privacy, redaction, locality"). `pair-add` MUST refuse to freeze a bundle whose `determinism`/redaction metadata indicates redaction did not run, and MUST scan `network.requests[].url` and `redirectChain` for unredacted token-shaped params, failing closed.
- **Human PII review gate.** `pair-add` prints a short manifest of external origins, captured text length, and any inline data URIs, and requires explicit confirmation (or a `--yes` flag in headless use) before the bundles are written into the tracked tree. Real pages can carry personal data in visible text — committing it is a deliberate, reviewed act.
- **`.gitignore` change (the one unavoidable repo edit, to be made at implementation time, not now).** Today `calibration/.capture/` is ignored, which is correct for Tier-2. Tier-3 bundles under `testbed/pairs/**` must be **tracked**; `testbed/.runs/` stays ignored. Add a positive rule for `testbed/pairs/**/*.bundle.json` (and `pair.json`, `expected-issues.json`, `baseline.json`).
- **Repo weight.** `CaptureBundle`s can be large (M6 diff-results were ~1 MB+). Mitigations, in order of preference: (a) cap capture output at freeze time using the existing `maxTextLength` and `probeLinks:false` capture knobs; (b) set a per-bundle size budget (proposed **2 MB**) that `pair-add` warns past and `check-pair.py` logs; (c) prefer a handful of high-signal pairs over many. Git-LFS is explicitly **out of scope** for v1 — bundles are plain JSON so they stay diffable and greppable.

---

## 9. CLI requirements on `matchy`

The replay path exists, but this feature pins three requirements an implementing agent must verify (and fix if absent — these are small, in-scope CLI corrections, not new analysis):

- **R-CLI-1.** `matchy analyze --old-bundle PATH --new-bundle PATH --out DIR` writes `<DIR>/diff-result.json` in the same shape and location as the live `matchy` run, and validates it against `/contract/diff-result.schema.json`. Exit codes match build spec §14 (`0`/`1`/`2`).
- **R-CLI-2.** `matchy analyze` honors the global `--profile`, `--baseline`, and `--fail-on` flags (they are declared `global = true` in `matchy.rs`). Confirm propagation into the analyze branch; add it if the analyze path ignores them. `--viewport` is irrelevant to analyze (the bundle carries its own viewport) and may be ignored.
- **R-CLI-3.** Promote `matchy analyze` from its current internal "for determinism verification" status to a **documented, supported entrypoint** in build spec §14 (CLI & config), since Tier-3 fixtures and the user-facing "replay a saved pair" workflow now depend on it.

---

## 10. Determinism & invariants (checklist for the implementing agent)

- [ ] Tier-3 replay is **byte-deterministic**: same committed bundles → identical `diff-result.json` (modulo `runId`/timestamps). Verified by the byte-exact golden when one is recorded (build spec §13.3, §15).
- [ ] `check-pair.py` **reuses** `check-fixture.py`'s matcher engine — no second implementation of the `expected-issues` DSL.
- [ ] `expected-issues.json` and its schema are **reused unchanged**; only the application domain widens.
- [ ] SHA-256 integrity of both bundles is enforced on every run; mismatch is a hard error.
- [ ] Committed bundles are **redaction-clean** and **PII-reviewed**; the freeze step fails closed otherwise.
- [ ] Tier-3 runs **hermetically** — no testbed servers, no Playwright, no network — so it executes in minimal CI.
- [ ] A failing fixture defaults to **fix-the-code**; expectation changes require a `golden-changelog.md` entry + `golden-auditor` APPROVE (CLAUDE.md golden discipline).
- [ ] `<case-id>` uses the `p<NN>-` prefix; no collision with Tier-1 `v<NN>-` golden names.
- [ ] No new analysis capability, crawler, auth, or live-capture-in-CI is introduced.

---

## 11. Proposed milestone (M9) & definition of done

Slots after M8 in the build spec's build order (and supersedes the one-shot nature of the M6 calibration artifacts).

**M9 — Real-pair regression fixtures.**
1. `testbed/schemas/pair.schema.json` + validation wired into the harness.
2. `testbed/check-pair.py` (replay + integrity + reuse of the Tier-1 matcher engine).
3. `make pair-add`, `make pair`, `make pair-refresh`; `verify` step inserted before golden comparisons; `.gitignore` updated to track `testbed/pairs/**`.
4. R-CLI-1..3 confirmed/implemented; build spec §14 documents `matchy analyze`.
5. **At least one seed fixture committed**, ideally promoted from an M6 calibration pair (R1 or R3 are natural candidates: capture once, freeze, author intent + `knownDrift` pin-outs).
6. Privacy/redaction gate implemented and tested against a bundle with a token-bearing URL.

**DoD:** `make pair-add` on a fresh URL pair produces a frozen, schema-valid, redaction-clean fixture with a stub expectation; `make verify` runs every committed pair hermetically (no servers) and gates on it; at least one committed pair demonstrates a real false-negative or false-positive caught and locked; the loop "capture real failure → freeze → write intent (red) → fix code → green → optional byte-golden" is exercised end-to-end and documented in the README/CLAUDE.md testbed section.

---

## 12. Open questions & alternatives considered

- **One harness or two?** This spec proposes a separate `check-pair.py` that *reuses* the matcher engine, rather than overloading `check-fixture.py` with a `--pair` mode. Rationale: the capture/replay front-halves differ (localhost live capture vs. frozen-bundle replay) while the matcher back-half is shared; a thin sibling keeps each entrypoint legible. **Alternative:** extend `check-fixture.py`. Acceptable if the front-half branching stays small. *Decision left to implementation; the non-negotiable is engine reuse, not file count.*
- **Goldens directory: shared vs. dedicated.** This spec reuses `testbed/goldens/` (zero Makefile change, relies on the `p`-prefix). **Alternative:** `testbed/pairs-goldens/` with its own loop — cleaner separation, but adds a Makefile branch. Reuse preferred unless prefix collisions prove fragile.
- **Bundle size / LFS.** Plain committed JSON chosen for diffability; a size budget guards weight. If real pairs routinely exceed a few MB, revisit Git-LFS or a `maxTextLength` default specifically for frozen fixtures. *Deferred.*
- **Live "soft" pairs.** A future `frozen:false` mode could re-capture a stable internal URL pair on demand for smoke testing. Explicitly **out of scope** here — it reintroduces the nondeterminism this tier exists to eliminate. The `frozen`/`refreshPolicy` fields reserve room for it.
- **Auto-deriving `required` matchers.** Rejected for false-negative/false-positive cases (the current output is the thing being corrected). Could be offered only for `demonstrates: "true-positive"` promotion of an already-trusted calibration pair, behind an explicit flag, with `golden-auditor` review. *Deferred.*
