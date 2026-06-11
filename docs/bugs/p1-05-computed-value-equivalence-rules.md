# Issue: Diff lacks computed-value equivalence rules: invisible zero-width border colors and text-align start/left noise dominate style counts

**Status:** FIXED (2026-06-11 — see ROOT-CAUSE-AND-PLAN.md and docs/golden-changelog.md)
**Found:** 2026-06-11, during a real migration gate (Webflow staging page vs local Next.js port)
**Severity:** P1 — two canonicalization rules alone would eliminate 15–26% of all style_changed issues across tested runs, making the style score effectively meaningless until implemented
**Area:** diff engine / scoring

---

## Summary

Two classes of computed-CSS noise dominate `style_changed` counts across every tested run of the number-registration page pair:

**(a) Zero-width border color differences** — issues of the form `border changed from 0px none <colorA> to 0px none <colorB>`. A border with `width: 0px` and `style: none` is completely invisible; comparing its color component is comparing a property that has no visual effect. Webflow emits `rgb(0, 0, 0)` for the border-color on unstyled elements; Next.js Tailwind resets emit `rgb(38, 38, 38)` or brand colors. These produce a dense cluster of spurious errors.

**(b) `text-align: start` vs `text-align: left`** — in any LTR document, `start` resolves to `left` at the layout engine. Webflow emits `left` on its elements; Next.js Tailwind / CSS-Modules emit `start`. These are semantically identical in LTR context and should not generate a diff.

Together, in three separate runs against the same page, these two rules would eliminate between 15% and 26% of all `style_changed` issues — yet none of those eliminated issues represent a real visual divergence.

## Environment

- matchy 0.1.0 (d5f0713)
- Linux; node v24.15.0; Chrome Headless Shell 148.0.7778.96 (pw chromium-headless-shell v1223)
- old=`https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration`
- new=`http://localhost:3001/products/connect/number-registration`
- desktop 1440x1000 + mobile 390x844; profile `content-structure`
- diff-result files: `/tmp/matchy-nr-1/diff-result.json`, `/tmp/matchy-nr-2/diff-result.json`, `/tmp/matchy-nr-10/diff-result.json`

## Reproduction

```bash
# Count zero-width border noise in any run
python3 -c "
import json
with open('/tmp/matchy-nr-1/diff-result.json') as f:
    data = json.load(f)
style = [i for i in data['issues'] if i['type'] == 'style_changed']
zb = [i for i in style if '0px none' in i['message'] and 'border' in i['message']]
ta = [i for i in style if 'text-align' in i['message'] and 'left' in i['message'] and 'start' in i['message']]
print(f'zero-border: {len(zb)}, text-align start/left: {len(ta)}, total style: {len(style)}')
"
```

## Observed

**Zero-width border color noise — cluster `cluster_6be1c72afa38` in `/tmp/matchy-nr-1/diff-result.json`:**

All 172 issues in this cluster have the same shape — both sides carry an invisible border and differ only in the color component:

```
id: issue_0bea78d3b959
message: border changed from 0px none rgb(0, 0, 0) to 0px none rgb(38, 38, 38)
evidence.old.border: 0px none rgb(0, 0, 0)
evidence.new.border: 0px none rgb(38, 38, 38)
evidence.match.score: 1.0

id: issue_1a8b59cc47eb
message: border changed from 0px none rgb(255, 255, 255) to 0px none rgb(80, 93, 111)
evidence.old.border: 0px none rgb(255, 255, 255)
evidence.new.border: 0px none rgb(80, 93, 111)
evidence.match.score: 0.8357

id: issue_24ccdae7dbe0
message: border changed from 0px none rgb(255, 255, 255) to 0px none rgb(0, 0, 0)
evidence.old.border: 0px none rgb(255, 255, 255)
evidence.new.border: 0px none rgb(0, 0, 0)
evidence.match.score: 1.0
```

A representative hero-section sample from the same run shows 5 zero-border issues in the `banner` landmark with pairs like `0px none rgb(17, 17, 17)` vs `0px none rgb(10, 19, 48)`. None of these borders are visible in any rendering.

**`text-align: left` vs `text-align: start` noise — cluster `cluster_186c85ce496f` in `/tmp/matchy-nr-1/diff-result.json`:**

All 108 issues in this cluster are `text-align` changes between `left` and `start`:

```
message: text-align changed from left to start near "FAQs"
message: text-align changed from start to center near "Reduce the risk of your calls being labeled spam"
```

In `/tmp/matchy-nr-10/diff-result.json`, the FAQ section alone contributes 36 `text-align left → start` issues (all `score: 1.0`, i.e. well-paired elements where the only diff is the LTR-equivalent keyword):

```
id: issue_115b7b3a01ac
message: text-align changed from left to start near "FAQs"
score: 1.0   (well-paired — the issue is in the equivalence rule, not the pairing)
```

**Computed impact across three runs (python3 over on-disk files):**

| Run | Total `style_changed` | Zero-border issues | `left`↔`start` issues | Removable | % of total |
|-----|----------------------|-------------------|----------------------|-----------|-----------|
| nr-1 | 1487 | 172 | 54 | 226 | 15.2% |
| nr-2 | 1592 | 200 | 68 | 268 | 16.8% |
| nr-10 | 1056 | 185 | 92 | 277 | 26.2% |

All three style scores are near zero (0.00119, 0.00119, 0.00157). Removing 15–26% of issues that carry no visual signal would materially change the score and reduce noise that obscures real findings.

## Expected

Before emitting a `style_changed` issue, the diff engine should apply computed-value equivalence rules:

1. **Zero-width border/outline rule:** if `border-width` resolves to `0px` on both sides, drop `border-color` and `border-style` from the comparison entirely. The property is invisible regardless of its other components. Apply the same rule to `outline` when `outline-width: 0`.
2. **`text-align` LTR normalization:** resolve `start` → `left` and `end` → `right` before comparing when the resolved writing direction is LTR (the common case). Emit a diff only if the resolved values differ.
3. **`line-height: normal` resolution:** resolve `normal` to the UA computed pixel value (`font-size × 1.2`) before comparing. This is a third noise class, visible in messages like `line-height changed from 22.6094px to normal`, where 22.6094 ≈ 18.84 × 1.2 — effectively the same line-height expressed two ways.

These rules should be implemented in a canonicalization pass that runs on both sides' computed values before the diff comparison, not as post-hoc suppression. The result: issues that survive the pass are guaranteed to reflect a real visual difference.

## Evidence

All numbers in the table above were computed with `python3 -c` over the on-disk files at `/tmp/matchy-nr-{1,2,10}/diff-result.json`. Method: count issues where `type == "style_changed"` AND `"0px none" in message AND "border" in message` (zero-border rule); and `type == "style_changed"` AND `"text-align" in message AND "left" in message AND "start" in message` (LTR normalization rule). No additional filtering applied — these are raw counts.

## Suggested fix direction

Add a `canonicalize_computed_value(property, value, direction)` helper called in the property-diff stage:

```
canonicalize("border", "0px none rgb(0,0,0)", ltr) → "0px"  # width==0, drop color+style
canonicalize("border", "0px none rgb(38,38,38)", ltr) → "0px"  # same → no diff
canonicalize("text-align", "start", ltr) → "left"
canonicalize("text-align", "left", ltr) → "left"  # equal → no diff
canonicalize("line-height", "normal", ltr) → computed_px(font_size * 1.2)
```

Make the rules list configurable in the profile TOML so project-specific edge cases can be added without a code change.
