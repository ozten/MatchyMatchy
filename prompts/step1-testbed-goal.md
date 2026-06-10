# Step 1 — Testbed: prompts to paste

Start the session on Fable 5 (pick it via `/model` or launch with `claude --model claude-fable-5`;
subagents stay pinned to sonnet/haiku regardless).

## 1. Kick off the work

```
/build-testbed https://YOUR-TARGET-SITE.example.com/some-content-rich-page
```

(Pick a page with a form, gradient/distinctive styling, several sections, links and images —
a marketing homepage or product page is ideal. The command will push back if the page is too thin.)

## 2. Immediately set the goal so it runs to completion

```
/goal The testbed is complete and verified: (1) the golden page and all assets are captured under testbed/golden/ and served at localhost:3000 with CAPTURE-NOTES.md documenting determinism strips; (2) at least 14 single-change variants exist under testbed/variants/, each with site/, serve.py on its assigned port 3001+, manifest.json, and a hand-authored expected-issues.json, together covering goals G1 through G6 plus the render-equivalent negative case from spec section 13.2; (3) `make testbed-check` has been run and its output shown, exiting 0 with every server responding 200 and every manifest and expected-issues file validating against schema; (4) docs/testbed-report.md exists summarizing every variant. Surface the testbed-check output and the report in conversation as proof. Stop and report blockers instead of continuing if the same step fails 3 turns in a row, or after 50 turns.
```

Notes:
- The goal evaluator only sees what Claude surfaces in chat, so the condition demands the
  verification output be shown — keep that clause.
- Non-interactive variant if you want to walk away:
  `claude -p "/build-testbed <url>" ...` then resume, or run the whole thing as
  `claude -p "/goal <condition above, prefixed with: First run the /build-testbed workflow for <url>. Then: ...>"`.
- After it finishes, spot-check 3 expected-issues.json files yourself — they're the contract
  your implementation will be graded against.
