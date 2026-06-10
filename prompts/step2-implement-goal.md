# Step 2 — Implementation: prompts to paste

Run one goal per milestone rather than one mega-goal for the whole spec. Each milestone has a
crisp DoD in spec §12, which makes the evaluator's job (and your auditing job) tractable, and a
fresh session per milestone keeps Fable's context clean. (`/goal` allows one active goal per
session anyway.)

## Per-milestone pattern (repeat for M1 → M7)

```
/implement-milestone M1
```

then immediately:

```
/goal Milestone M1 of docs/page-pair-diff-spec.md is done per its DoD: docs/design/M1.md exists; the capture package produces a schema-valid CaptureBundle for two live URLs; analyze emits old.png, new.png, diff.png, page-height delta, and a DiffResult validating against contract/diff-result.schema.json; `make verify` output has been shown exiting 0; a determinism spot-check (same bundles run twice, byte-identical DiffResult excluding runId/timestamps) has been shown passing; every testbed variant mapped to M1 passes its expected-issues.json check with the fixture run output shown; and any expectation changes have an APPROVE verdict from golden-auditor recorded in docs/golden-changelog.md. Surface all verification output in conversation. Stop and report instead of continuing if blocked on the same failure for 3 turns, or after 80 turns.
```

For M2–M7, swap the milestone-specific clause for that milestone's DoD from spec §12, e.g.:

- **M2:** "...the trailing-slash, redirect-chain, es_MX and es-mx variants produce url_trailing_slash, url_redirect_chain, locale_separator_invalid, locale_case_invalid with correct remediation from/to fields..."
- **M4:** "...the gradient-removed variant yields background_gradient_lost with from/to evidence, and the spacing/color variant yields style_changed with property-level from/to and a greppable anchor set, with no source component named anywhere in the DiffResult..."
- **M5:** "...the swapped-sections variant yields exactly one component_swapped and zero missing/added issues for those sections..."

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
