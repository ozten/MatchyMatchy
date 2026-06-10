# page-pair-diff — Claude Code kit

Drop these files into your repo root, copy the spec to `docs/page-pair-diff-spec.md`, and you're set.

```
your-repo/
  CLAUDE.md                          # model routing + golden discipline + invariants
  docs/page-pair-diff-spec.md        # <- copy your spec here (paths in the kit assume this)
  .claude/
    agents/
      fixture-builder.md             # sonnet  — testbed capture + permutations
      code-implementer.md            # sonnet  — writes code to your design briefs
      test-runner.md                 # haiku   — runs verification, structured summaries, read-only
      golden-auditor.md              # inherit (=Fable) — gates every expectation/golden change
    commands/
      build-testbed.md               # /build-testbed <url>
      implement-milestone.md         # /implement-milestone M<N>
  prompts/
    step1-testbed-goal.md            # exact /goal text for step 1
    step2-implement-goal.md          # per-milestone /goal pattern for step 2
```

## How the cost routing works
- Launch the session on Fable 5 (`/model`, or `claude --model claude-fable-5`). The main loop
  does design, fixture diagnosis, expected-output authoring, and review — the high-leverage thinking.
- Subagents pin their own models via frontmatter: sonnet for code/fixtures, haiku for test runs.
  They don't inherit Fable, so the bulk token spend (file churn, test logs) lands on cheap models.
- golden-auditor uses `model: inherit` deliberately: judging whether an expectation change is
  honest is exactly the task worth frontier reasoning, and it runs read-only with few turns.

## Run order
1. `git init`, commit the kit + spec. Restart Claude Code so agents load (disk-edited agents load at session start).
2. Follow `prompts/step1-testbed-goal.md` (one session).
3. Spot-check expected-issues.json files by hand.
4. Follow `prompts/step2-implement-goal.md`, one session per milestone M1–M7.

## Why milestone-sized goals
The spec's §12 DoDs are already written as completion conditions; the /goal evaluator judges only
what Claude shows in conversation, so each goal condition demands the verification output be
surfaced. Bounded clauses ("stop after N turns / 3 repeated failures") prevent runaway loops.
