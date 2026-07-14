---
title: "feat: Embed build provenance in matchy --version"
type: feat
status: completed
date: 2026-07-10
---

# feat: Embed build provenance in matchy --version

## Summary

Embed compile-time git provenance (short commit SHA, commit timestamp, dirty flag) into the `matchy` binary via a small hand-rolled Cargo build script and print it in `--version` (e.g. `matchy 0.1.0 (a7c04e4 2026-07-10T01:19:04Z, dirty=false)`), so a stale installed binary is a one-command check against `git rev-parse --short HEAD`. Also: `make build` echoes the embedded provenance on completion, and the README warns that `git pull` alone does not update an installed binary.

---

## Problem Frame

`matchy --version` reports only `matchy 0.1.0`. The recommended from-source install is a symlink/copy of `target/release/matchy`, so `git pull` updates the source tree while the installed binary silently stays at whatever commit it was last built from — undetectable from the tool itself. This bit the operator on 2026-07-10: after the #2 fix landed (cf44823), the installed binary predated it and reproduced the exact bug the source tree said was fixed (issue #3).

---

## Requirements

- R1. `matchy --version` prints the crate version plus short commit SHA, commit timestamp (ISO-8601 UTC), and a dirty flag, in the shape suggested by issue #3: `matchy 0.1.0 (a7c04e4 2026-07-10T01:19:04Z, dirty=false)`.
- R2. Staleness is a one-command check: the embedded SHA is directly comparable to `git rev-parse --short HEAD`.
- R3. A rebuild after a new commit re-embeds the new SHA **even when no Rust source changed** — the build script must re-run when git HEAD (or the ref it points at) changes. Without this, the provenance itself reproduces the staleness bug it exists to catch.
- R4. Missing git provenance never fails the build (cross-rs container, tarball builds): degrade to a visible `unknown` marker.
- R5. `make build` echoes the embedded provenance on completion.
- R6. README documents that `git pull` alone does not update the installed binary — `make build` is required — and how to check for staleness.
- R7. `DiffResult` JSON output is byte-identical before/after this change: `toolVersion` stays plain `CARGO_PKG_VERSION`. Goldens byte-compare it, so provenance is CLI-human-output only.

---

## Scope Boundaries

- No change to the `DiffResult` contract or the JSON `toolVersion` field (`contract/diff-result.schema.json`, `packages/analyze/src/report/json.rs`) — goldens byte-compare `toolVersion`, and a contract change is a different (golden-auditor-gated) piece of work.
- No runtime staleness detection (e.g. matchy comparing its embedded SHA against a source tree at startup) — matchy runs from arbitrary working directories, not the repo.
- No new dependencies (`vergen`, `shadow-rs`, `built`) — hand-rolled `build.rs`.
- No changes to the release install path (`scripts/install.sh`, `.github/workflows/release.yml`) — prebuilt-binary installs are versioned by tag and unaffected by the stale-symlink problem.
- No changes to the TypeScript capture layer's version reporting (it reads its own package.json at runtime — separate concern).

### Deferred to Follow-Up Work

- `matchy doctor` printing a provenance header line: natural next surface (nothing parses doctor stdout), but issue #3 asks for `--version`; add later if wanted.
- `GITHUB_SHA` passthrough into the cross-rs container (e.g. Cross.toml `build.env`) so arm64 release binaries embed a real SHA instead of `unknown`: only worth doing if release-artifact provenance ever matters — release version comes from the tag today.

---

## Context & Research

### Relevant Code and Patterns

- `packages/analyze/Cargo.toml` — crate `matchy-analyze` v0.1.0, `[[bin]] name = "matchy"`; the build script attaches here (`packages/analyze/build.rs`, first in the repo).
- `packages/analyze/src/bin/matchy.rs` — clap derive, `#[command(name = "matchy", version, about = ...)]` (~line 29); bare `version` pulls `CARGO_PKG_VERSION`. Replace with an explicit static version string composed from build-script-emitted env vars.
- `packages/analyze/src/report/json.rs` (`assemble_diff_result`, ~line 375) — `tool_version: env!("CARGO_PKG_VERSION")`; **must stay untouched** (R7). `--self-check`'s `self-check.json` flows through the same assembly.
- `testbed/compare-golden.py` (~line 29) — `EXCLUDED_KEYS = {"runId", "capturedAt"}`; `toolVersion` is byte-compared in every `testbed/goldens/*.diffresult.json`.
- `packages/analyze/tests/{analyze_cli,explain,self_check,show}.rs` — integration tests drive the compiled binary via `Command::new(env!("CARGO_BIN_EXE_matchy"))`; the pattern for a new `--version` test.
- `Makefile` — `build:` target (~lines 31-33): `cargo build --release` + capture npm build. No install target exists.
- `README.md` — `### Build from source` (~lines 85-96, says to copy the binary to `~/.local/bin`) and the `### Make targets` table (~line 246).
- `scripts/install.sh` (~line 148) — post-install `matchy --version` echo; informational, format-agnostic (will show the richer line for free).
- `.github/workflows/release.yml` — arm64 build uses `cross build` in a Docker container; git may be unavailable there ("dubious ownership" / missing binary), hence R4.

### Institutional Learnings

- `docs/solutions/` is empty — none applicable.
- Project memory: the #2 incident (this issue's trigger) and the doctor "build not found" misdiagnosis both stem from invisible binary/environment state; this plan makes one class of that visible.

### External References

- None needed — hand-rolled build script over a pinned pattern; no new library surface.

---

## Key Technical Decisions

- **Hand-rolled `build.rs` over `vergen`**: zero new dependencies, ~40 lines, full control over fail-soft behavior and rerun triggers. Issue #3 explicitly offered either option.
- **Commit (committer) timestamp, not build wall-clock time**: deterministic — the same source state always embeds the same string, preserving reproducible builds in a project that treats determinism as an invariant. The dirty flag covers uncommitted drift; the issue's example (`a7c04e4 2026-07-10T01:19:04Z`) is the commit time.
- **Provenance confined to CLI stdout**: `toolVersion` in the JSON contract stays `CARGO_PKG_VERSION` because goldens byte-compare it (R7). This is *not* a golden change, so no golden-auditor sign-off is needed.
- **One version string for both `-V` and `--version`**: nothing parses either; a single surface is simpler than a short/long split.
- **Fail-soft in the build script**: any git failure (missing binary, not a repo, dubious ownership) emits `unknown` values and succeeds. Provenance is diagnostics, never a build gate.
- **Build-script rerun directives are load-bearing** (`rerun-if-changed` on `.git` HEAD + resolved ref): note that once any `rerun-if-changed` is emitted, cargo re-runs the script *only* on those paths — the watch set is what makes R3 true.

---

## Open Questions

### Resolved During Planning

- Output format: adopt issue #3's suggestion verbatim (`matchy 0.1.0 (a7c04e4 2026-07-10T01:19:04Z, dirty=false)`).
- Timestamp semantics: commit time, not build time (see Key Technical Decisions).
- Does anything parse `--version`? No — verified across testbed, scripts, CI, tests, docs. Format change is safe.
- Depth classification: Lightweight — no external consumers of the changed surface; the CI touch-point is handled by the fail-soft decision, not CI config changes.

### Deferred to Implementation

- Exact `cargo:rustc-env` variable names and the `(unknown)` fallback rendering: cosmetic, settled in code.
- Exact rerun watch set: `.git/HEAD` plus the resolved ref file is the baseline; whether watching `.git/index` too (for fresher dirty-flag recomputation) is worth it can be judged in implementation.
- Whether to read `GITHUB_SHA` as a fallback inside `build.rs` when git fails: harmless either way; decide in code.
- Git-dir discovery detail: the crate sits two levels below the repo root, so the script should ask git for the git dir (from `CARGO_MANIFEST_DIR`) rather than hard-coding `../../.git` — exact mechanics settled in code.

---

## Implementation Units

### U1. Embed git provenance via build.rs and print it in --version

**Goal:** `matchy --version` (and `-V`) reports version + short SHA + commit timestamp + dirty flag; the embedded SHA refreshes on any new commit; builds without git still succeed.

**Requirements:** R1, R2, R3, R4, R7

**Dependencies:** None

**Files:**
- Create: `packages/analyze/build.rs`
- Modify: `packages/analyze/src/bin/matchy.rs`
- Test: `packages/analyze/tests/version.rs`

**Approach:**
- `build.rs` shells out to git (discovering the git dir from the crate's manifest dir, since the crate is nested in a workspace) to collect: short SHA, committer timestamp in UTC ISO-8601, and dirty status. It emits these as `cargo:rustc-env` values plus `cargo:rerun-if-changed` directives for the git HEAD file and the ref it points at. On any failure it emits `unknown` placeholders and exits successfully.
- `src/bin/matchy.rs` swaps the bare clap `version` attribute for an explicit static string composed at compile time from the emitted env vars, in the issue's format.
- `report/json.rs` is deliberately not touched — `toolVersion` keeps its current value (R7).

**Patterns to follow:**
- Integration tests: `packages/analyze/tests/analyze_cli.rs` — `Command::new(env!("CARGO_BIN_EXE_matchy"))` invocation style.
- Compile-time env: existing `env!("CARGO_PKG_VERSION")` usage in `packages/analyze/src/report/json.rs`.

**Test scenarios:**
- Happy path: `matchy --version` exits 0 and output matches `matchy <semver> (<sha> <timestamp>, dirty=<true|false>)` shape (or the `unknown` fallback shape) — assert the format, not the specific SHA/dirty values, so the test is stable on dirty trees.
- Happy path: when the test environment is a git checkout (the normal case), the embedded short SHA equals `git rev-parse --short HEAD` — guard the assertion so it skips gracefully in a non-git build environment rather than failing (R4 environments).
- Happy path: `-V` prints the same provenance string as `--version`.
- Edge case: JSON output unaffected — an existing-fixture analyze run still emits `toolVersion` equal to plain `CARGO_PKG_VERSION` with no SHA/timestamp in it (guards R7 at the unit level; the golden gate in `make verify` guards it end-to-end).
- Error path (manual verification, not cargo test — the test env always has git): building with git unavailable succeeds and `--version` shows the `unknown` fallback.

**Verification:**
- `make verify` fully green — in particular the golden comparison step, proving `toolVersion` bytes are unchanged.
- `./target/release/matchy --version` SHA matches `git rev-parse --short HEAD`.
- The R3 regression check: make an empty commit, rebuild (no source edits), and confirm `--version` reports the new SHA — this is precisely the staleness path from the incident.

---

### U2. make build echoes the embedded provenance

**Goal:** finishing `make build` shows exactly what was just built, so the operator sees the SHA without a separate command.

**Requirements:** R5

**Dependencies:** U1

**Files:**
- Modify: `Makefile`

**Approach:**
- Append a final step to the `build:` recipe that runs the freshly built release binary's `--version` (the binary the testbed and installs use).

**Test scenarios:**
- Test expectation: none — build-recipe change with no behavioral code; verified by observing `make build` output ends with the provenance line.

**Verification:**
- `make build` completes and its last line is the `matchy <version> (<sha> ...)` string.

---

### U3. README staleness note

**Goal:** document the operational gotcha so the next operator doesn't rediscover it via a broken-fix incident.

**Requirements:** R6

**Dependencies:** U1 (references the new `--version` behavior)

**Files:**
- Modify: `README.md`

**Approach:**
- In `### Build from source`, after the copy-to-`~/.local/bin` instruction: note that `git pull` alone does not update an installed binary — run `make build` after pulling, and compare `matchy --version` against `git rev-parse --short HEAD` to detect staleness.
- Update the `make build` row in the `### Make targets` table to mention the provenance echo.

**Test scenarios:**
- Test expectation: none — documentation only.

**Verification:**
- README renders correctly; instructions match the actual `--version` output shape and Makefile behavior shipped in U1/U2.

---

## System-Wide Impact

- **Unchanged invariants:** `DiffResult.toolVersion` (and `self-check.json`, which flows through the same assembly) keeps its exact current bytes; all `testbed/goldens/*.diffresult.json` remain valid with zero re-recording and no golden-auditor involvement. Analyze-layer byte-determinism is untouched — the embedded provenance is read only by the CLI version string, never by report assembly, and contains no build-wall-clock value anyway.
- **API surface parity:** `scripts/install.sh`'s post-install echo automatically shows the richer line (informational only). Testbed harnesses (`check-fixture.py`, `check-pair.py`, `run-all.py`) invoke `target/release/matchy` directly and never read `--version` — unaffected.
- **Build/CI:** the release workflow's arm64 `cross` build runs `build.rs` inside a container where git may fail — covered by R4's fail-soft (`unknown`), so tag releases cannot break.
- **Integration coverage:** the golden-comparison step of `make verify` is the end-to-end proof that nothing leaked into JSON output.

---

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Build script fails in environments without usable git (cross container, tarball), breaking release builds | Fail-soft is a hard requirement (R4): any git error → `unknown` placeholders, build succeeds. Manual-verification scenario in U1. |
| Embedded SHA goes stale after a commit-only change (build script not re-run) — the plan's own version of the bug it fixes | `rerun-if-changed` directives on git HEAD + resolved ref (R3); explicit empty-commit rebuild check in U1 verification. |
| Dirty flag can lag reality (editing a source file rebuilds the crate but does not re-run the build script) | Accepted as best-effort: dirty is recomputed whenever the script re-runs; the load-bearing guarantee is SHA freshness on commit/pull, which the watch set covers. Optionally watch `.git/index` (deferred to implementation). |
| Provenance accidentally reaches `DiffResult.toolVersion`, instantly failing every golden | Explicit non-goal + R7 test scenario in U1 + the golden gate in `make verify` catches it before merge. |

---

## Sources & References

- Issue: [ozten/MatchyMatchy#3](https://github.com/ozten/MatchyMatchy/issues/3) — `--version` has no build provenance; stale installed binaries are undetectable
- Incident context: issue #2 fix (`cf44823`) masked by a stale binary at `a7c04e4`
- Related code: `packages/analyze/src/bin/matchy.rs`, `packages/analyze/src/report/json.rs` (`assemble_diff_result`), `testbed/compare-golden.py`, `Makefile`, `README.md`, `scripts/install.sh`, `.github/workflows/release.yml`
