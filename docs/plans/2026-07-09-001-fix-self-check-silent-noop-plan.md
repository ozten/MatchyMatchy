---
title: "fix: --self-check silent no-op (prefix rejection + swallowed failure)"
type: fix
status: completed
date: 2026-07-09
---

# fix: --self-check silent no-op (prefix rejection + swallowed failure)

## Summary

Make `matchy --self-check` actually run and make its failures visible: widen the capture-config prefix vocabulary so the second capture is accepted, and promote any self-check pipeline failure to a machine-readable `RunWarning` in the main DiffResult (exit code unchanged). Also repair the downstream `pair-add.py` knownDrift extractor, which research showed stays broken even after the prefix fix.

---

## Problem Frame

`matchy --self-check` never performs its second capture: the Rust runner passes `prefix: "old-selfcheck"` (`packages/analyze/src/bin/matchy.rs`, `run_self_check`), but the capture layer's zod schema (`packages/capture/src/schema.ts`, `CaptureConfigSchema`) only allows `"old" | "new"`, so every viewport's self-check capture is rejected with `INVALID_CONFIG`. The failure path is an `eprintln` + `continue`; when all viewports fail, `run_self_check` returns an empty warning list, so the run completes with exit 0, no `self-check.json`, and `warnings: []` — a silent no-op whose only trace is stderr.

This is a literal recurrence of Root Cause 2 documented in `docs/bugs/ROOT-CAUSE-AND-PLAN.md` ("capture was designed best-effort: log, continue… no warnings/degraded channel") — the `warnings[]` channel added in contract v1.1 exists precisely so capture degradation is promoted to run-level output. The stakes are higher than one flag: WP-H made `--self-check` the volatility-probe mechanism, and median-of-N capture / auto-mask were explicitly deferred *because* "self-check + the warnings channel give visibility into residual live-page volatility." A silently broken self-check invalidates that deferral rationale. It also starves `testbed/pair-add.py`'s `knownDrift` seeding (p01's knownDrift had to be hand-authored).

Reported as issue #2 (found 2026-07-09 while measuring the capture noise floor for the hiya.com-cms migration gate calibration).

---

## Requirements

- R1. `matchy --self-check` performs the second capture successfully: the capture layer accepts the self-check prefix and `self-check.json` is written on a healthy run.
- R2. When the old-vs-old probe finds drift, the main DiffResult carries the existing `volatile_capture` warning (existing behavior, now reachable).
- R3. Any self-check pipeline failure — capture rejection/failure, missing bundle, analysis error, `self-check.json` write failure — surfaces as a machine-readable warning in the main DiffResult `warnings[]` and in rendered reports. Never a silent no-op; stderr-only is not sufficient.
- R4. Exit code semantics are unchanged: self-check failure never alters the exit code (warnings must not feed `compute_exit_code`).
- R5. Determinism invariants hold (fixed warning order, sorted context maps) and **zero bytes change** in existing goldens, variant expectations, or Tier-3 pair fixtures.
- R6. `pair-add.py` knownDrift seeding works end-to-end from a real matchy run — the extractor reads the warning shape matchy actually emits, from the file it actually lands in.
- R7. `--self-check` is documented accurately (README + `--help` agree with real behavior).

---

## Scope Boundaries

- **No exit-code change and no strict-mode flag.** A hard error (exit 2) and a `--strict-self-check` promotion flag were both considered and rejected with the user: exit 2 is reserved for main-capture/tool failure (`docs/design/M1.md`, `docs/design/M8.md`), and `pair-add.py` treats exit 2 as a hard abort — a flaky diagnostic probe must not kill captures.
- **No contract schema change, no schemaVersion bump.** `RunWarning.code` is a free string in both `contract/diff-result.schema.json` and the Rust contract type; a new code value is not a contract change.
- **No re-seeding of p01's knownDrift.** Re-freezing an existing pair via `pair-refresh` is a golden change requiring `golden-auditor` sign-off; out of scope here.
- **No changes to issue-id stability.** Content-addressed issue IDs not surviving re-captures is a separate tracked bug; knownDrift stays warning-shaped (not id-keyed) so this fix does not silently reintroduce id-keyed suppression assumptions.
- **No new volatility features** (median-of-N capture, auto-mask) — those remain deferred; this fix restores the visibility their deferral depends on.

### Deferred to Follow-Up Work

- Capture the cross-layer vocabulary-drift lesson (Rust-emitted literals vs TS zod enums outside `/contract`) as an institutional learning doc after the fix lands.
- Consider automated knownDrift-driven suppression once issue-id stability is fixed (separate bug).

---

## Context & Research

### Relevant Code and Patterns

- `packages/analyze/src/bin/matchy.rs` — `run_self_check` (~517-639): per-viewport second capture, `eprintln` + `continue` on failure, `Ok(vec![])` when all viewports fail (the silent no-op), `self-check.json` written as a full old-vs-old DiffResult, `volatile_capture` warning emitted only when the probe finds ≥1 issue (with `BTreeMap` `byType` context — the determinism pattern to copy).
- `packages/analyze/src/orchestrate.rs` — `CaptureConfigParams.prefix` is a plain `&str`; the `old|new` constraint exists **only** in the TS zod schema. `run_capture` resolves the capture script via `MATCHY_CAPTURE_PATH` (~80-141) — the hermetic-test hook.
- `packages/capture/src/schema.ts` — `CaptureConfigSchema.prefix: z.enum(["old", "new"]).optional()`; rejection surfaces as `INVALID_CONFIG` from `packages/capture/src/capture.ts`. Filenames derive from prefix: `<vp>/<prefix>.png`, `<prefix>-vp.png`, `<prefix>.bundle.json`. Doctor mode sends no prefix — unaffected.
- `packages/analyze/src/contract.rs` — `RunWarning { code: String, message, context: Option<Value> }`; `context` always serialized (null when None) to keep golden key-sets stable.
- `packages/analyze/src/report/json.rs` — `build_warnings` constructs the existing codes (`capture_step_failed`, `capture_integrity_delta`, `capture_retried_without_time_freeze`, `baseline_stale_ids`); deterministic ordering documented + unit-tested (`test_extra_warnings_appended_last`: extra warnings appended last, `volatile_capture` last today).
- Warning rendering: `packages/analyze/src/report/markdown.rs` (`## Warnings` blockquotes) and `packages/analyze/src/report/html.rs` (warnings section) — both render in compact and full disclosure modes; code + message only.
- Test patterns: `packages/capture/tests/schema.test.ts` (vitest, known-good-object + mutation style; `CaptureConfigSchema` has **zero coverage today**); `packages/analyze/tests/analyze_cli.rs` (drives the real binary via `CARGO_BIN_EXE_matchy` with synthetic bundles in temp dirs — fully hermetic).
- `testbed/pair-add.py` — runs `matchy … --self-check`, treats exit 0/1 as success and 2 as hard abort; freeze allowlist already excludes `old-selfcheck.*` and `self-check.json`; `_extract_volatile_capture_warnings` → `pair.json["knownDrift"]`.

### Institutional Learnings

- `docs/bugs/ROOT-CAUSE-AND-PLAN.md` (Chain 2, WP-E/WP-H): capture degradation must be promoted to `warnings[]`, never swallowed — the sanctioned remedy this plan applies.
- `docs/golden-changelog.md` (v1.1/v1.2 entries): schemaVersion bumps + golden re-records were needed for new *required fields*; a new warning *code value* needs neither. All 21 goldens carry `warnings: []` and no hermetic path passes `--self-check` → expected golden delta is zero; any moved byte is a regression signal, not a re-record occasion.
- `docs/design/M1.md` / `docs/design/M8.md`: exit codes 0/1/2; `compute_exit_code` keys off issue severities only.
- Known gap (first instance, undocumented): the CLAUDE.md TS↔Rust CI validation invariant covers only `/contract/*.schema.json`; the capture *config* schema is outside that guard, which is why this drift survived. U1 adds a vocabulary guard test.

### External References

- None needed — fully local bug with strong in-repo patterns (external research deliberately skipped).

---

## Key Technical Decisions

- **Widen the capture schema, don't re-route the runner** *(user-confirmed)*: accept the self-check prefix in `CaptureConfigSchema`. The self-check bundle lands alongside `old`/`new` in the viewport dir as the runner already intends; `pair-add.py` already anticipates and excludes those files. The alternative (reuse `"old"` into a separate directory) means new output layout and more Rust path-handling for no benefit.
- **Enum extension, not a free-form slug**: add `"old-selfcheck"` to the enum rather than relaxing `prefix` to a pattern-validated string. The prefix drives filenames on disk; a closed vocabulary keeps that input strictly validated, and future prefixes become deliberate schema edits pinned by tests.
- **Failure → warning, exit unchanged** *(user-confirmed)*: any self-check failure emits a `RunWarning` and the run completes. Consistent with the documented exit-code convention (2 = main-capture/tool failure only) and with `pair-add.py`'s exit-2-is-fatal handling. Warnings never influence `compute_exit_code`.
- **One warning code, `self_check_failed`, covering all probe failure modes** (capture failure, missing bundle, analysis error, `self-check.json` write failure): a single machine-stable snake_case code with per-stage detail in a structured, deterministically-ordered `context` (failed viewports and reasons; sorted collections per the CLAUDE.md determinism invariant). Existing stderr `eprintln`s stay for interactive visibility.
- **Partial failure still yields a probe result**: if some viewports fail, `self-check.json` is written from the surviving viewports and `self_check_failed` lists only the failed ones (it can coexist with `volatile_capture` from the survivors). Only a total failure skips `self-check.json`.
- **Extra-warning ordering stays fixed and documented**: the extra block remains appended after generated warnings in a fixed relative order (`volatile_capture` / `self_check_failed`); the exact relative order is settled during implementation and the `test_extra_warnings_appended_last` assertion updated coherently.
- **`pair-add.py` extractor reads the main `diff-result.json` `warnings[]`**: matchy emits `volatile_capture` only into the *main* DiffResult — it never appears in `self-check.json`, whose own `warnings` carry only capture-degradation codes generated from the probe captures themselves (e.g., `capture_retried_without_time_freeze`) — and the code is a *value* of `RunWarning.code`, not a dict key. The current extractor key-scans `self-check.json` and can never match — this is why knownDrift seeding stays broken even after the prefix fix, and why U4 is in scope.

---

## Open Questions

### Resolved During Planning

- Hard error vs warning on self-check failure: **warning, exit unchanged** (user choice; convention-consistent).
- Widen schema vs reroute runner to an allowed prefix: **widen schema** (user choice).
- Enum extension vs validated slug: **enum extension** (closed vocabulary for a filename-driving input).
- Is the `pair-add.py` extractor fix in scope: **yes** — the confirmed scope includes "unblocks knownDrift seeding," and research showed seeding stays broken without it.

### Deferred to Implementation

- Exact field names inside `self_check_failed.context` (e.g., how failed viewports and reasons are keyed): settled while writing the code; must serialize deterministically.
- Relative order of `volatile_capture` vs `self_check_failed` within the extra-warnings block: either is fine; pick one, document it, and pin it in the ordering test.
- Whether the hermetic stub capture script is a shared test helper or per-test fixture: follow whatever `tests/analyze_cli.rs` makes most natural.

---

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

Probe outcome → observable behavior (today, every row collapses to "second capture rejected → exit 0, nothing written, `warnings: []`"):

| Probe outcome | `self-check.json` | Warnings added to main DiffResult | Exit code |
|---|---|---|---|
| All viewports captured, no drift | written | none | unchanged |
| All viewports captured, drift found | written | `volatile_capture` | unchanged |
| Some viewports fail | written (from survivors) | `self_check_failed` (+ `volatile_capture` if survivors show drift) | unchanged |
| All viewports fail | not written | `self_check_failed` | unchanged |
| Probe ran, `self-check.json` write fails | attempted | `self_check_failed` (write-stage reason) | unchanged |

Data flow: `run_full` → `run_self_check` → `Vec<RunWarning>` → `assemble_diff_result(extra_warnings)` → `warnings[]` in `diff-result.json` → markdown/HTML `Warnings` section → (`pair-add.py` reads `volatile_capture` entries → `pair.json.knownDrift`).

---

## Implementation Units

### U1. Widen the capture-config prefix vocabulary and pin it with tests

**Goal:** The capture layer accepts `"old-selfcheck"` as a prefix, and the config schema's vocabulary is test-pinned for the first time so it can't silently drift from what the Rust runner emits.

**Requirements:** R1

**Dependencies:** None

**Files:**
- Modify: `packages/capture/src/schema.ts`
- Test: `packages/capture/tests/schema.test.ts`

**Approach:**
- Extend the `prefix` enum in `CaptureConfigSchema` with `"old-selfcheck"`.
- `CaptureConfigSchema` currently has zero test coverage — add a coverage block following the existing known-good-object + mutation style used for bundles in this file.
- Include a cross-layer guard: a test that validates the exact prefix literals the Rust runner emits (with a comment cross-referencing `run_self_check` in `packages/analyze/src/bin/matchy.rs`), so a future prefix added on the Rust side without a schema edit fails a named test rather than failing silently at runtime.

**Patterns to follow:**
- `packages/capture/tests/schema.test.ts` known-good + mutation pattern.

**Test scenarios:**
- Happy path: configs with prefix `"old"`, `"new"`, `"old-selfcheck"` all parse successfully.
- Edge case: config with prefix omitted parses (doctor mode sends none).
- Error path: an unknown prefix (e.g., `"foo"`) and a near-miss (e.g., `"old-selfcheck2"`) are rejected with the enum error.
- Guard: the literal(s) emitted by the Rust runner validate against the schema (vocabulary pin).

**Verification:**
- Capture package tests green; `make build` rebuilds `dist/capture.cjs` and a self-check-shaped config is no longer rejected with `INVALID_CONFIG`.

---

### U2. Promote self-check failures to a `RunWarning` (Rust)

**Goal:** No self-check failure path is silent: every failure mode contributes to a `self_check_failed` warning in the main DiffResult, while exit-code semantics and byte-determinism are untouched.

**Requirements:** R3, R4, R5

**Dependencies:** None (independently testable; real-world path also needs U1)

**Files:**
- Modify: `packages/analyze/src/bin/matchy.rs` (`run_self_check`)
- Modify (if ordering docs/tests live there): `packages/analyze/src/report/json.rs`
- Test: inline `#[cfg(test)]` in the touched modules

**Approach:**
- Collect per-viewport failures (capture failure, missing `old.bundle.json`, analysis error) instead of `continue`-and-forget; after the loop, fold any failures into a single `self_check_failed` `RunWarning` with a deterministically-ordered context (failed viewports + stage/reason; sorted collections, mirroring `volatile_capture`'s `BTreeMap` `byType`).
- Partial failure: still assemble and write `self-check.json` from surviving viewports; total failure: skip the file but return the warning (replacing today's `return Ok(vec![])`).
- Fold a `self-check.json` write failure into the same warning code with a write-stage reason.
- Keep existing `eprintln!` stderr lines.
- Do not touch `compute_exit_code`; warnings must remain invisible to it.
- Update the extra-warnings ordering documentation and `test_extra_warnings_appended_last` so the fixed order including the new code is pinned.

**Patterns to follow:**
- `volatile_capture` construction in `run_self_check` (code/message/context shape, determinism).
- `RunWarning` conventions: snake_case machine-stable code, `context` always present (null when none).

**Test scenarios:**
- Happy path: no failures → no `self_check_failed` warning constructed.
- Edge case: two viewports, one fails → warning context lists exactly the failed viewport with its stage/reason; construction is deterministic across runs (sorted keys/collections).
- Edge case: all viewports fail → warning emitted, and the code path returns it instead of an empty vec.
- Error path: `self-check.json` write failure → warning carries a write-stage reason.
- Ordering: extra-warnings block still appended last in the documented fixed order (updated `test_extra_warnings_appended_last`).

**Verification:**
- `cargo test` green; `make verify` shows zero golden/expectation deltas (R5 — any moved byte is a regression in this unit, not a re-record occasion).

---

### U3. Hermetic integration tests for the self-check flow

**Goal:** End-to-end coverage of `--self-check` through the real binary — success, drift, partial failure, and total failure — with no Chromium, network, or testbed servers.

**Requirements:** R1, R2, R3, R4

**Dependencies:** U2 (warning behavior); U1 for the real-world path (note: the stub bypasses the zod schema, which is exactly why U1's vocabulary guard test exists)

**Files:**
- Create: `packages/analyze/tests/self_check.rs`

**Approach:**
- Use the `MATCHY_CAPTURE_PATH` resolution hook (`packages/analyze/src/orchestrate.rs`) to point the binary at a stub Node script that reads the config JSON from stdin, records it (so tests can assert what prefix the runner sent), writes synthetic bundle/screenshot files, and emits a canned `CaptureResponse` line — success or failure per scenario.
- Drive the real binary via `CARGO_BIN_EXE_matchy` with temp dirs, following `packages/analyze/tests/analyze_cli.rs`.

**Test scenarios:**
- Happy path: identical old/self-check bundles → `self-check.json` written, no `volatile_capture` and no `self_check_failed` in `diff-result.json`.
- Happy path (drift): differing self-check bundle → `volatile_capture` present with issue-count context; `self-check.json` written.
- Error path: stub fails the self-check capture (all viewports) → run completes with the same exit code as the equivalent run without `--self-check`, no `self-check.json`, and `self_check_failed` present in `diff-result.json` `warnings[]`.
- Edge case: two viewports, one self-check capture fails → `self-check.json` exists (from the survivor) and `self_check_failed` lists only the failed viewport.
- Integration/regression guard: the recorded config for the second capture carries the self-check prefix (pins the Rust-side literal end-to-end).

**Verification:**
- New integration test file passes hermetically under `cargo test` (and therefore under `make verify`); no test requires Playwright, Chromium, or testbed ports.

---

### U4. Fix `pair-add.py` knownDrift extraction

**Goal:** knownDrift seeding works against matchy's real output: `volatile_capture` warnings are read from where they actually land, in the shape they actually have.

**Requirements:** R6

**Dependencies:** U2 (final warning shapes)

**Files:**
- Modify: `testbed/pair-add.py`
- Test: `testbed/tests/test_pair_add.py`

**Approach:**
- Point the extractor at the main `diff-result.json` `warnings[]`, filtering entries whose `code` equals `volatile_capture` (today it key-scans `self-check.json`, which can never match: the code is a value, not a key, and `volatile_capture` never lands in self-check.json).
- Keep `knownDrift` warning-shaped — do not key it on issue ids (issue-id instability is a separate bug; see Scope Boundaries).
- When the run's warnings include `self_check_failed`, surface a clear operator message that knownDrift could not be seeded (instead of silently writing an empty list).
- Update `test_pair_add.py`'s fabricated fixtures to the real `RunWarning` shape (`{code, message, context}` entries in `warnings[]`).

**Patterns to follow:**
- Existing `pair-add.py` structure and `test_pair_add.py`'s tuple-list test runner (note: not part of `make verify`; run it directly).

**Test scenarios:**
- Happy path: `diff-result.json` containing a `volatile_capture` warning → `pair.json.knownDrift` seeded with it.
- Edge case: clean probe (no `volatile_capture`) → `knownDrift` empty, no operator warning.
- Edge case: `self_check_failed` present → `knownDrift` empty and the operator message is printed.
- Error path: missing/unreadable `diff-result.json` → existing failure behavior preserved.

**Verification:**
- `test_pair_add.py` passes; a manual `pair-add` dry-run (optional, non-CI) seeds knownDrift from a genuinely volatile page.

---

### U5. Document `--self-check` accurately

**Goal:** README and `--help` describe real behavior: what the probe does, both warning codes, `self-check.json`, and exit semantics — closing the gap where issue #2's author believed README documented the flag (it never did; only the clap help text does).

**Requirements:** R7

**Dependencies:** U2 (wording reflects final behavior)

**Files:**
- Modify: `README.md`
- Modify: `packages/analyze/src/bin/matchy.rs` (clap doc-comment, if wording drifts from final behavior)

**Approach:**
- Add `--self-check` to the README's flag/usage documentation: second capture of the old URL, old-vs-old diff written to `self-check.json`, `volatile_capture` warning only when drift is found (current help text implies it's unconditional), `self_check_failed` warning on probe failure, exit code never affected.

**Test scenarios:**
- Test expectation: none — documentation-only unit.

**Verification:**
- README and `--help` statements match the behavior pinned by U3's tests.

---

## System-Wide Impact

- **Interaction graph:** `run_full` → `run_self_check` → `extra_warnings` → `assemble_diff_result` → `warnings[]` → markdown/HTML report warning sections (both disclosure modes) → `pair-add.py` knownDrift seeding. No other consumers of `warnings[]` exist in-repo.
- **Error propagation:** capture-layer `INVALID_CONFIG` for the self-check prefix disappears (U1); genuine capture/analysis failures travel as one structured `RunWarning` instead of stderr-only. Exit codes unaffected by design.
- **API surface parity:** the fix ships in two artifacts — the Rust binary and `dist/capture.cjs`. `scripts/install.sh` places `capture.cjs` next to the binary, so installed copies need the rebuilt bundle; repo-local flows are covered by `make build`.
- **Integration coverage:** U3 covers the Rust↔capture-script process boundary hermetically; a live-Chromium run against a testbed variant remains an optional manual check (not CI-gated).
- **Unchanged invariants:** `contract/*.schema.json` untouched; `schemaVersion` stays 1.2; all 21 variant goldens, expectations, and Tier-3 pair fixtures byte-identical; `compute_exit_code` untouched; doctor mode (sends no prefix) unaffected; `check-pair.py`/`check-fixture.py` unaffected (self-check artifacts are never frozen — `pair-add.py` already excludes `old-selfcheck.*` and `self-check.json` from freezing).

---

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| New warning perturbs deterministic output or an existing golden | Sorted/`BTreeMap` context, fixed extra-warning order, updated ordering unit test; `make verify` must show zero golden deltas — any delta is treated as a bug in the fix |
| Hermetic stub drifts from real capture.cjs behavior (stub bypasses zod validation) | U1's vocabulary guard test pins the schema side; U3 asserts the exact prefix the runner sends; comment cross-references tie the two literals together |
| Installed copies keep a stale `capture.cjs` that still rejects the prefix | Called out in Documentation/Operational Notes; fix ships via rebuilt `dist/capture.cjs` in the next release artifact |
| Scope creep into adjacent bugs (issue-id instability, p01 re-seed) | Explicitly out of scope; knownDrift stays warning-shaped |

---

## Documentation / Operational Notes

- README gains `--self-check` documentation (U5); clap help text corrected if it overpromises (`volatile_capture` is conditional on drift).
- Release note for the next tag: `--self-check` requires the rebuilt `capture.cjs`; previously-installed copies silently no-op.
- After landing, consider a `docs/solutions/` learning on cross-layer vocabulary drift (see Deferred to Follow-Up Work).
- No golden-changelog entry expected: zero golden bytes should move; a brand-new red/green Tier-3 fixture is not being added here.

---

## Sources & References

- Related issue: [ozten/MatchyMatchy#2](https://github.com/ozten/MatchyMatchy/issues/2)
- Related code: `packages/analyze/src/bin/matchy.rs` (`run_self_check`), `packages/capture/src/schema.ts` (`CaptureConfigSchema`), `packages/analyze/src/contract.rs` (`RunWarning`), `packages/analyze/src/report/json.rs` (warning ordering), `testbed/pair-add.py`
- Design provenance: `docs/bugs/ROOT-CAUSE-AND-PLAN.md` (Root Cause 2, WP-E/WP-H), `docs/bugs/p1-03-run-to-run-variance.md`, `docs/plans/2026-06-16-001-feat-real-pair-regression-fixtures-plan.md`
- Conventions: `docs/design/M1.md`, `docs/design/M8.md` (exit codes), `docs/golden-changelog.md` (contract-change precedent)
