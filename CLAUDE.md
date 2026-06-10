# page-pair-diff — project memory

Authoritative spec: `docs/prds/page-pair-diff-spec.md`. Where anything conflicts with it, the spec wins.
Primary deliverable: the `DiffResult` JSON contract (spec §7). Build in milestone order (spec §12); do not advance a milestone until its fixtures pass.

## Model routing policy (cost control — follow strictly)

The main session runs on the frontier model (Fable 5). It is reserved for:
- Architecture and design decisions, contract/schema design, matching-algorithm reasoning
- Reviewing subagent output, diagnosing why a fixture fails, deciding code-vs-expectation when they disagree
- Writing/approving expected outputs and golden-change rationales

Everything mechanical MUST be delegated to subagents:
- `code-implementer` (sonnet): writes code to a design brief you provide. Give it the brief; do not let it design.
- `fixture-builder` (sonnet): downloads pages, builds permutation variants, writes serve scripts and manifests.
- `test-runner` (haiku): runs builds/tests/verification commands and returns structured summaries. Never edits.
- `golden-auditor` (inherit = Fable): independently audits any change to expected outputs or goldens.

Before doing multi-file edits, test runs, or fixture authoring yourself, ask: "is this mechanical?" If yes, delegate. Keep your own context for thinking; let subagents burn theirs on tool output.

## Testbed layout

```
testbed/
  golden/                 # downloaded original page + all assets (NEVER modified)
    site/                 # static files
    serve.py              # serves on :3000
  variants/
    v01-identical/        # :3001  control — must produce status: pass, zero issues
    v02-banner-added/     # :3002  ...one variant per port, one deliberate change each
      site/
      serve.py
      manifest.json        # { name, port, change, goals: ["G4"], description }
      expected-issues.json # hand-authored INTENT (see below)
  run-all.py              # starts golden + every variant; --check verifies all 200
```

Each variant contains exactly ONE deliberate change relative to golden (plus any unavoidable knock-on effects, which the manifest must declare). Variants are static and deterministic: no timestamps, no randomness, fonts/images vendored locally.

## Expected outputs: two tiers, two rules

1. `expected-issues.json` (per variant, hand-authored): the *intent*. Lists required issues
   (type + anchor/property matchers + key evidence like gradient from/to) and forbidden issues
   (e.g. render-equivalent variant must NOT yield missing/added). This is authoritative for what
   the tool SHOULD detect.
2. `goldens/<variant>.diffresult.json` (recorded tool output, added during implementation): the
   byte-level regression baseline, compared with float tolerances, timestamps/runId excluded.

### Golden discipline (non-negotiable)
- The default response to a failing fixture is to FIX THE CODE, never to edit the expectation.
- An expectation may change only when the expectation itself was wrong (over-specified, ambiguous,
  contradicted the spec). Every such change requires:
  1. An entry in `docs/golden-changelog.md`: what changed, why the old expectation was wrong,
     spec section justifying the new one.
  2. An APPROVE verdict from the `golden-auditor` subagent (paste its verdict into the changelog).
- Never weaken an expectation merely because the current code can't meet it. Never delete a
  forbidden-issue assertion to silence a false positive — false positives are bugs.
- Re-recording byte goldens after an approved behavior change is fine; the changelog entry covers it.

## Engineering invariants (from spec §3.3 / §15 — enforce in review)
- Analyze layer is a pure function; byte-deterministic. No HashMap/Object iteration order in
  anything affecting output (BTreeMap or collect-and-sort). Total-order tie-breaks on node id.
  Fixed-order float reductions.
- Confidence bands (matchFloor/noMatchCeil), never a single hard cutoff. Per-signal sub-scores
  written to `evidence.match`.
- Every issue: content-addressed id, anchor-set locator, structured remediation with grep targets.
  Never name a source component.
- Contract lives in `/contract/*.schema.json`; TS zod + Rust serde both validated against it in CI.

## Commands
- `make testbed-up` / `make testbed-check` — start servers / verify all respond 200 + manifests validate
- `make verify` — build + unit + integration + golden comparison; exit 0 = green
- `make fixture VARIANT=v06` — run ppd against one variant and diff against expectations

## Languages
TypeScript for `packages/capture` (Playwright). Rust for `packages/analyze` (start in Rust directly;
the TS-fallback clause in spec §3.1 is NOT taken — we have Rust capacity). Python only for testbed serve scripts.
