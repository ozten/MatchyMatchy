---
description: Implement one milestone of the page-pair-diff spec against the testbed
argument-hint: <milestone, e.g. M1>
---

Implement milestone $ARGUMENTS of `docs/prds/page-pair-diff-spec.md` for real, verified against the
testbed. Read `CLAUDE.md` first; follow the model-routing policy and golden discipline strictly.

## Loop
1. **Design (you).** Re-read the spec sections this milestone covers plus §3.3 (determinism) and
   §15 (invariants). Write `docs/design/$ARGUMENTS.md`: modules, types/signatures, algorithms,
   contract deltas, acceptance criteria, and which testbed variants must pass. For M3+ this is
   where the hard thinking lives (matching weights, banding, sequence diff) — do it yourself.
2. **Build (delegate).** Dispatch code-implementer with the design brief, in reviewable chunks.
   Review each diff yourself for the determinism invariants before accepting.
3. **Verify (delegate).** Dispatch test-runner: `make verify`, then `make fixture VARIANT=...`
   for every variant this milestone claims. Also request a determinism spot-check (same bundles
   twice → byte-identical DiffResult).
4. **Close the gap (you decide, agents execute).** For each failing fixture, diagnose:
   - Tool wrong → write a fix brief for code-implementer. This is the default.
   - Expectation wrong → draft the correction + rationale, dispatch golden-auditor, and only on
     APPROVE update the expectation and append to `docs/golden-changelog.md` with the verdict.
   - Genuinely ambiguous → surface it to me with both readings; do not pick silently.
5. **Promote goldens.** Once a variant passes its `expected-issues.json` intent check, record the
   full DiffResult as `goldens/<variant>.diffresult.json` (runId/timestamps excluded, float
   tolerances on scores) and wire it into `make verify`.
6. **Report.** Summarize: what shipped, fixture pass table, any golden-changelog entries, open
   risks, and whether the milestone's DoD from spec §12 is met verbatim.
7. **Clean up scheduling.** If you used ScheduleWakeup/cron fallback timers while background
   agents ran, run CronList and CronDelete any pending entries before reporting — completion
   notifications make the timers redundant, and stale ones outlive the task.

Do not start the next milestone. Do not mark $ARGUMENTS done while any mapped fixture is red or
any expectation change lacks an APPROVE.
