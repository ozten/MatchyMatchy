# Bug reports — 2026-06-11 field test

Twelve reports from matchy's first real migration gate: a Webflow staging page
(`/products/connect/number-registration` on the hiya.com staging site) compared
against a local Next.js re-implementation, ~10 runs over one working session.
Environment: matchy 0.1.0 (d5f0713), Linux, node v24.15.0, Chrome Headless
Shell 148.0.7778.96 (playwright chromium-headless-shell v1223). All numbers in
these reports were verified against the on-disk run artifacts
(`/tmp/matchy-nr-*/diff-result.json`, capture bundles) before filing.

**Status (2026-06-11): all 12 FIXED.** Root-cause analysis (5-whys) and the
work-package plan are in [ROOT-CAUSE-AND-PLAN.md](./ROOT-CAUSE-AND-PLAN.md);
the resulting contract v1.1 golden re-record is documented and audited in
`docs/golden-changelog.md`. Deferred follow-ups (median-of-N capture,
uncertain-pairing confidence scaling, selector-depth heuristics) are listed at
the end of the plan doc.

| # | Pri | Report | One line |
|---|-----|--------|----------|
| 01 | P0 | [time-freeze corrupts baseline capture](./p0-01-time-freeze-corrupts-baseline-capture.md) | Frozen clock crashes target-page Swiper; baseline silently loses a whole section; no integrity warning. Workaround: `--no-freeze-time`. |
| 02 | P0 | [issue ids unstable across runs](./p0-02-issue-ids-unstable-across-runs.md) | Volatile href query params are hashed into ids; `--baseline` accept-lists can't express durable acceptances. |
| 03 | P1 | [run-to-run variance](./p1-03-run-to-run-variance.md) | Errors ranged 116–155 (and scores swung) across runs with zero candidate changes; gating on counts/scores flaps. |
| 04 | P1 | [weak-pairing style noise](./p1-04-weak-pairing-style-noise.md) | Sub-threshold element pairings (many `band: null`) emit style diffs between unrelated elements; style score pins near 0. |
| 05 | P1 | [computed-value equivalence rules](./p1-05-computed-value-equivalence-rules.md) | Invisible zero-width border colors + `start`↔`left` text-align = 15–26% of all style_changed issues. |
| 06 | P1 | [landmark scoping](./p1-06-landmark-scoping.md) | Chrome (nav/footer) dominates errors on every page run (107/129); want `--scope main` + per-landmark scores. |
| 07 | P2 | [responsive image dimensions mode](./p2-07-responsive-image-dimensions-mode.md) | naturalWidth compare flags intentional same-aspect CDN downscales; opt-in responsive mode. |
| 08 | P2 | [capture.cjs resolution](./p2-08-capture-cjs-resolution.md) | No ancestor walk from the binary; running outside the repo cwd fails despite the bundle existing in the binary's repo. |
| 09 | P2 | [browser revision mismatch UX](./p2-09-browser-revision-mismatch-ux.md) | Triple-printed Playwright banner; doctor accepts any chromium instead of the pinned build (1217 vs 1223). |
| 10 | P2 | [report.md grouping](./p2-10-report-md-grouping.md) | Flat 1705-row table with per-viewport duplicates; group by landmark → nearestHeading. |
| 11 | P2 | [page-height attribution](./p2-11-page-height-attribution.md) | 419px page-height delta reported with `bbox: null`; attribute the delta per section/landmark. |
| 12 | P2 | [localhost protocol downgrade](./p2-12-localhost-protocol-downgrade.md) | error-severity https→http on localhost candidates, with a nonsensical `https://localhost` remediation; add dev-env handling. |

Filing note: these were drafted for `gh issue create` against ozten/MatchyMatchy,
but the active fine-grained PAT lacks the `Issues: Read and write` permission
(HTTP 403, `x-accepted-github-permissions: issues=write`). Once the token is
updated, each file's H1 + Severity line maps directly onto an issue title +
`P0/P1/P2` label.
