# Issue: Add landmark scoping (--scope main) and per-landmark score breakdowns

**Status:** FIXED (2026-06-11 — see ROOT-CAUSE-AND-PLAN.md and docs/golden-changelog.md)
**Found:** 2026-06-11, during a real migration gate (Webflow staging page vs local Next.js port)
**Severity:** P1 — shared chrome (nav/footer) dominates page-level results and causes correctly-ported pages to report `status: fail`, making the gate unusable for page-by-page migration without manual triage
**Area:** diff engine / scoring (enhancement)

---

## Summary

Every matchy run captures a full page including its shared navigation, notification bar, and footer. These chrome elements are identical or near-identical across all pages of a site. When porting pages one by one, the chrome is often in a partially-ported or deliberately-deferred state. Its issues drown out page-specific findings and hold every page in `status: fail` regardless of whether the page-body work is complete.

The problem is structural, not incidental: in `/tmp/matchy-bc-baseline/diff-result.json` — a Branded Call page pair whose port was previously accepted as shipped — matchy reports `status: fail`, `visual: 0.709`, `content: 0.006`, with 322 error-severity issues. Of those 322 errors, 130 are in chrome landmarks (`contentinfo`: 102, `banner`: 4, `navigation`: 4, `form`: 20) vs 178 in `main`. Because the page-body score is conflated with chrome noise, the shipped-and-accepted page registers as failing.

Separately, across multiple NR-page runs, approximately 107–105 of ~129–155 error-severity issues are in chrome landmarks (`contentinfo` / `banner` / `navigation`), leaving only 8–36 errors in `main` per run.

## Environment

- matchy 0.1.0 (d5f0713)
- Linux; node v24.15.0; Chrome Headless Shell 148.0.7778.96 (pw chromium-headless-shell v1223)
- old=`https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration`
- new=`http://localhost:3001/products/connect/number-registration`
- old (bc-baseline)=Webflow branded-call staging URL; new=Next.js branded-call port (previously accepted as shipped)
- desktop 1440x1000 + mobile 390x844; profile `content-structure`
- diff-result files: `/tmp/matchy-bc-baseline/diff-result.json`, `/tmp/matchy-nr-2/diff-result.json`, `/tmp/matchy-nr-1/diff-result.json`

## Reproduction

```bash
python3 -c "
import json
from collections import Counter
with open('/tmp/matchy-bc-baseline/diff-result.json') as f:
    bc = json.load(f)
errors = [i for i in bc['issues'] if i['severity'] == 'error']
lm = Counter(i['locator']['anchors'].get('landmark') or '(none)' for i in errors)
print(bc['status'], bc['scores'])
for k,v in lm.most_common(): print(f'  {k}: {v}')
"
```

## Observed

**`/tmp/matchy-bc-baseline/diff-result.json` — a previously-accepted shipped page:**

```
status: fail
scores: {
  "visual": 0.709,
  "content": 0.006,
  "structure": 1.0,
  "style": 0.0007,
  "accessibility": 0.333,
  "technical": 1.0,
  "hygiene": 0.125
}
Total error-severity issues: 322
```

Error issue distribution by landmark:
```
  main:        178
  contentinfo: 102   ← footer (shared chrome)
  form:         20   ← contact/HubSpot form (chrome-adjacent)
  (none):       14
  banner:        4   ← header (shared chrome)
  navigation:    4   ← nav (shared chrome)
```

Chrome landmark total (contentinfo + banner + navigation): **110 of 322 errors** are from shared chrome. Examples:

```
contentinfo: Link removed: 'Spam Analytics'          (type=missing_link)
contentinfo: Link removed: 'Customer Stories'        (type=missing_link)
contentinfo: Link removed: 'Developer Docs'          (type=missing_link)
banner:      Link removed: 'Get started'             (type=missing_link)
banner:      Link target changed from '/' to '/en-US' (type=changed_link_target)
navigation:  Link text changed from 'Why Hiya?' to '' (type=changed_link_text)
navigation:  Link text changed from 'Resources' to '' (type=changed_link_text)
```

These are footer/nav issues shared with every other page on the site. They are correctly flagged as diffs but should not gate a page-specific parity check.

**`/tmp/matchy-nr-2/diff-result.json` — number-registration page, active migration:**

```
Total error-severity issues: 129
  contentinfo: 99   ← footer
  (none):      14
  main:         8   ← page-body (8 real page-specific errors)
  banner:       4   ← header
  navigation:   4   ← nav
```

Chrome errors: 107 of 129 (83%). Only 8 errors are in `main`. If scored on `main` only, the error count would drop from 129 to 8 and the signal-to-noise ratio would be dramatically better.

**`/tmp/matchy-nr-1/diff-result.json`:**

```
Total error-severity issues: 155
  contentinfo: 98
  main:        36
  (none):      14
  navigation:   4
  banner:       3
```

Chrome errors: 105 of 155 (68%). Real page-body errors: 36.

## Expected

1. **`--scope <landmark>`** flag (or `scope = "main"` in profile TOML) restricts both issue generation and score computation to elements whose `locator.anchors.landmark` matches the given landmark. Issues outside the scope are still captured but moved to a `chrome_scoped` suppressed bucket in `diff-result.json` and reported separately in `report.md` under "Chrome / shared regions (out of scope)".

2. **Per-landmark score breakdown** in both `diff-result.json` and `report.md`:
   ```json
   "scores": {
     "byLandmark": {
       "main":        { "style": 0.72, "content": 0.85, ... },
       "contentinfo": { "style": 0.02, "content": 0.10, ... },
       "banner":      { ... }
     }
   }
   ```

3. **Dedicated chrome run pattern** — documentation recommendation: run matchy once against any page with chrome scope disabled to establish a chrome baseline; then run all page-by-page checks with `--scope main`. Chrome issues are tracked in one place rather than re-reported on every page.

## Evidence

Computed from on-disk files with `python3 -c` + `Counter` over `issues[].locator.anchors.landmark`:

| File | Total errors | Chrome landmark errors | `main` errors | Chrome % |
|------|-------------|----------------------|--------------|---------|
| bc-baseline | 322 | 110 | 178 | 34% |
| nr-2 | 129 | 107 | 8 | 83% |
| nr-1 | 155 | 105 | 36 | 68% |

bc-baseline overall style_changed by landmark: `main: 2349`, `contentinfo: 137`, `form: 93`, `banner: 67` — 297 of 2646 (11%) are in chrome landmarks. The footer alone generates more `style_changed` than most individual page sections.

## Suggested fix direction

- Add `scope` as a first-class config key in the profile TOML. Value is a landmark role string or a list of landmark role strings. Default: `null` (no scoping, current behavior preserved).
- In the diff engine, after landmark assignment, partition issues by scope membership before computing scores.
- Emit `"scopedTo": ["main"]` in the top-level `diff-result.json` metadata when scoping is active so readers know the scores are not full-page.
- In `report.md`, always emit per-landmark issue counts in the summary table even when scoping is not active, so the user can see the distribution.
