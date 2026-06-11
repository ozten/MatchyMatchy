# Issue: Weak cross-page element pairings produce misleading style_changed issues

**Status:** FIXED (2026-06-11 — see ROOT-CAUSE-AND-PLAN.md and docs/golden-changelog.md)
**Found:** 2026-06-11, during a real migration gate (Webflow staging page vs local Next.js port)
**Severity:** P1 — bad pairings inflate per-section style counts and hold the style score near 0.001 even after real fixes land, making the gate signal untrustworthy
**Area:** diff engine / scoring

---

## Summary

The pairing algorithm sometimes matches old-page elements that carry UA-default styles (unvisited-link `rgb(0, 0, 238)`, `display: block`, `border-radius: 0px`) against new-page CTA pill buttons and footer links that are deliberately styled. The resulting `style_changed` issues are not real regressions — they reflect a wrong pairing, not a real style divergence. Because the style score is computed over all `style_changed` issues regardless of pairing confidence, these phantom diffs keep the score at `~0.001` even when the implementer has correctly ported every styled element on the page.

In `/tmp/matchy-nr-2/diff-result.json`, 208 `style_changed` issues are anchored to `nearestHeading = "Start for free with Hiya Number Registration"`. Among them, 20 flag `color: rgb(0, 0, 238)` (UA unvisited-link blue) on the old side, 4 flag `border-radius: 0px → 999px`, and 6 flag `display: block → flex` — all patterns that indicate the old element is an unstyled default `<a>` being compared to a styled CTA pill in the new page. Score distribution in that heading cluster spans 0.62 to 1.0, with 106 issues having `band: null` (no confident band assignment).

## Environment

- matchy 0.1.0 (d5f0713)
- Linux; node v24.15.0; Chrome Headless Shell 148.0.7778.96 (pw chromium-headless-shell v1223)
- old=`https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration`
- new=`http://localhost:3001/products/connect/number-registration`
- desktop 1440x1000 + mobile 390x844; profile `content-structure`
- diff-result files: `/tmp/matchy-nr-1/diff-result.json`, `/tmp/matchy-nr-2/diff-result.json`

## Reproduction

Run matchy against the number-registration page pair with the `content-structure` profile. Open `diff-result.json` and filter `style_changed` issues where `locator.anchors.nearestHeading == "Start for free with Hiya Number Registration"`. Sort by `evidence.match.score` ascending. Inspect the `evidence.old` fields of the bottom quartile.

## Observed

Five representative issues from `/tmp/matchy-nr-2/diff-result.json`, all anchored to `nearestHeading = "Start for free with Hiya Number Registration"`:

**Issue 1 — UA link-blue color flagged as style drift**
```
id: issue_64491e22e6fb
message: color changed from rgb(0, 0, 238) to rgb(10, 19, 48)
cssSelectorOld: div:nth-of-type(2) > ... > a:nth-of-type(2)
cssSelectorNew: div:nth-of-type(1) > ... > a:nth-of-type(2)
evidence.match.band: matched   evidence.match.score: 0.9
evidence.old.color: rgb(0, 0, 238)
evidence.new.color: rgb(10, 19, 48)
```

`rgb(0, 0, 238)` is the browser UA default for unvisited links. The old element was never styled; the new element is intentionally branded. This pair should not produce a style regression.

**Issue 2 — border-radius 0px → 999px (flat link vs pill button)**
```
id: issue_33ff2e4342b6
message: border-radius changed from 0px to 999px
cssSelectorOld: div:nth-of-type(2) > ... > a:nth-of-type(1)
cssSelectorNew: div:nth-of-type(1) > ... > a:nth-of-type(1)
evidence.match.band: matched   evidence.match.score: 0.9
evidence.old.border-radius: 0px
evidence.new.border-radius: 999px
```

**Issue 3 — display block → flex (inline text link vs flex CTA)**
```
id: issue_85212bdb7805
message: display changed from block to flex
cssSelectorOld: div:nth-of-type(2) > ... > a:nth-of-type(2)
cssSelectorNew: div:nth-of-type(1) > ... > a:nth-of-type(1)
evidence.match.band: matched   evidence.match.score: 0.9
evidence.old.display: block
evidence.new.display: flex
```

**Issue 4 — text-align center → start (likely cross-section structural mismatch)**
```
id: issue_7a706602375d
message: text-align changed from center to start
cssSelectorOld: div:nth-of-type(3) > ... > h2:nth-of-type(1)
cssSelectorNew: section:nth-of-type(6) > ... > h2:nth-of-type(1)
evidence.match.band: matched   evidence.match.score: 1.0
```

**Issue 5 — lowest-confidence pairing (score 0.62, no band)**
```
id: issue_02a5a9d253fc
message: margin-top changed from 0px to 32px
cssSelectorOld: div:nth-of-type(3) > ... > div:nth-of-type(1)   (10-level deep div chain)
cssSelectorNew: section:nth-of-type(6) > div:nth-of-type(1)     (4-level)
evidence.match.band: null   evidence.match.score: 0.6214
```

Score distribution in this heading cluster: 12 issues at score 0.6214, 11 at 0.6571, 106 total with `band: null`.

## Expected

Style issues generated from pairs where `evidence.match.score` is below a configurable threshold (suggested default: 0.75 or profile-supplied) should be segregated into a distinct `uncertain_pairing` bucket. They should be reported separately in `report.md` (not mixed with confident-pair diffs) and **excluded from the style score computation**. This prevents a large population of structurally mismatched elements from keeping the score pinned near zero regardless of actual style work completed.

The `band: null` signal (no matching band assigned) should be a cheap first-order gate: if the engine could not assign a band, the pair is suspect and should not count against the style score as a confirmed regression.

## Evidence

From `/tmp/matchy-nr-2/diff-result.json`:

- Total `style_changed` issues: 1592
- `style` score: 0.0011947431302270011 (page status: fail)
- Issues near `"Start for free with Hiya Number Registration"` heading: 208 style_changed
  - `band: null` (no confident band): 106 of 208
  - Issues with `evidence.old.color = rgb(0, 0, 238)` (UA link blue): 20
  - Issues with `border-radius: 0px → 999px`: 4
  - Issues with `display: block → flex`: 6
- At score 0.6214 (lowest observed): 12 issues, all crossing a 10-level `div`-chain old selector vs a 4-level `section` new selector — structurally incommensurable elements

## Suggested fix direction

1. After the pairing stage, partition `style_changed` emissions into two buckets by `evidence.match.score` and `band`:
   - `confident` (score ≥ threshold AND band is not null): included in style score
   - `uncertain_pairing` (below threshold OR band null): reported separately, excluded from score
2. Make the threshold configurable in the profile TOML (e.g. `[scoring] min_pairing_score_for_style = 0.75`).
3. Add a structural-plausibility heuristic as a secondary gate: if old and new selectors differ in depth by more than N levels, demote to `uncertain_pairing` regardless of score.
4. In `report.md`, emit an `UNCERTAIN PAIRINGS` sub-section listing these issues with a note that they could not be confidently matched and may not represent real regressions.
