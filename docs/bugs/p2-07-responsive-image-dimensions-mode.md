# Issue: changed_image_dimensions flags intentional responsive downscaling; add a srcset/DPR-aware mode

**Status:** FIXED (2026-06-11 — see ROOT-CAUSE-AND-PLAN.md and docs/golden-changelog.md)
**Found:** 2026-06-11, during a real migration gate (Webflow staging page vs local Next.js port)
**Severity:** P2 — current strict mode produces actionable true-positive findings for strict-parity migrations; the noise class only becomes a problem when evaluating responsive-first frontends where same-asset downscaling is intentional
**Area:** diff engine / scoring (enhancement)

---

## Summary

matchy compares `naturalWidth` × `naturalHeight` of paired images. For a strict migration from Webflow (which always serves full-resolution source images) to a Next.js `next/image` pipeline (which generates CDN-optimised variants at lower resolutions), the candidate consistently serves downscaled variants. matchy correctly flags these as `changed_image_dimensions` at error severity.

During this migration, those findings were **acted on** — the team verified that images were being served at smaller natural dimensions than the Webflow originals and used this as a prompt to audit the image pipeline. Strict natural-width comparison was the right behaviour for that use case.

However, for ongoing development on a responsive frontend, `next/image` and similar systems intentionally serve `naturalWidth` variants tuned to viewport width and DPR. A 1600×1067 image on a 1440px desktop viewport may legitimately be served at 1200×800 or 800×534 depending on DPR and the `sizes` attribute. Flagging all such downscales as errors produces noise that is proportional to the number of images on the page, not to real parity regressions.

The request is for an opt-in `responsive` mode that compares rendered (CSS box) dimensions instead of natural dimensions, or that passes downscales that maintain aspect ratio and remain ≥ the rendered box size, while still flagging upscales, aspect-ratio changes, and missing images.

## Environment

- matchy 0.1.0 (d5f0713)
- Linux; node v24.15.0; Chrome Headless Shell 148.0.7778.96 (pw chromium-headless-shell v1223)
- old=`https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration`
- new=`http://localhost:3001/products/connect/number-registration`
- desktop 1440x1000 + mobile 390x844; profile `content-structure`
- diff-result file: `/tmp/matchy-nr-1/diff-result.json`

## Reproduction

Run matchy against the number-registration page pair. Filter `diff-result.json` for `type == "changed_image_dimensions"`. Observe that all downscaling cases maintain the original aspect ratio exactly (1600:1067 = 800:534 = 1200:800 to within floating-point).

## Observed

From `/tmp/matchy-nr-1/diff-result.json`, 10 `changed_image_dimensions` issues, all at `severity: error`. Three representative CDN-downscaled cases:

**Issue 1**
```
id: issue_4a3bca81e759
message: Image dimensions changed from 1600x1067 to 800x534
evidence.old: naturalWidth=1600, naturalHeight=1067
evidence.new: naturalWidth=800,  naturalHeight=534
evidence.match.band: matched   evidence.match.score: 0.7328
locator.anchors.nearestHeading: FAQs
```
Ratio check: 1600/1067 = 1.4994; 800/534 = 1.4981 — same aspect ratio, 50% downscale.

**Issue 2**
```
id: issue_a555bc3212bf
message: Image dimensions changed from 1600x1067 to 1200x800
evidence.old: naturalWidth=1600, naturalHeight=1067
evidence.new: naturalWidth=1200, naturalHeight=800
evidence.match.band: matched   evidence.match.score: 0.8444
locator.anchors.nearestHeading: Reduce the risk of your calls being labeled spam
```
Ratio: 1600/1067 = 1.4994; 1200/800 = 1.500 — same aspect ratio, 75% downscale.

**Issue 3**
```
id: issue_a76f2256dd17
message: Image dimensions changed from 1600x1000 to 1200x750
evidence.old: naturalWidth=1600, naturalHeight=1000
evidence.new: naturalWidth=1200, naturalHeight=750
evidence.match.band: matched   evidence.match.score: 0.8459
locator.anchors.nearestHeading: Reduce the risk of your calls being labeled spam
```
Ratio: 1600/1000 = 1.600; 1200/750 = 1.600 — identical aspect ratio, 75% downscale.

All three match `next/image`'s standard output sizes (800, 1200 wide) for a 1× desktop viewport. The CDN variant was selected by the browser based on the `sizes` attribute and the current DPR; it is the intended image for this context.

Also present in the same run: two issues with non-proportional dimension changes (`390x499 → 96x123`, `1440x1843 → 96x123`) that appear to be genuine broken-image or wrong-asset regressions rather than responsive downscaling — these should continue to be flagged in all modes.

## Expected

Two modes, selectable per profile:

**`image_dimensions_mode = "strict"` (default, current behavior)**
Flag any difference in `naturalWidth` or `naturalHeight`. No change to current behavior.

**`image_dimensions_mode = "responsive"` (opt-in)**
For each paired image, check the following sequence:
1. If new `naturalWidth` > old `naturalWidth` or new `naturalHeight` > old `naturalHeight` → **flag** (upscale; potentially quality regression or wrong asset).
2. If aspect ratio changes by more than a configurable tolerance (suggested: 2%) → **flag** (aspect ratio mismatch; likely wrong asset).
3. If new `naturalWidth` < rendered CSS width on the new page → **flag** (image is smaller than it needs to be; will be upscaled by the browser).
4. If new `naturalWidth` ≥ rendered CSS width AND aspect ratio preserved AND direction is downscale → **pass** (intentional responsive variant, suppress or downgrade to `info`).
5. If the new-side image is missing or returns a non-200 status → **flag** regardless of mode (already handled by `broken_image` / `missing_image` detectors).

The `responsive` mode should require that the diff engine also capture rendered CSS box dimensions (`getBoundingClientRect().width/height`) for each image element and store them in `evidence.old.renderedWidth` / `evidence.new.renderedWidth`. This is new data the current schema does not capture.

## Evidence

10 `changed_image_dimensions` issues in `/tmp/matchy-nr-1/diff-result.json`, all `severity: error`. Of those:
- 8 are same-aspect-ratio downscales (ratios verified by division above) — candidates for `responsive` mode suppression
- 2 are disproportionate dimension changes (`390x499 → 96x123`, `1440x1843 → 96x123`) — genuine regressions that should still be flagged in both modes

The 8 suppressible issues were **acted on during this migration** and the team confirmed the `next/image` pipeline was correctly generating variants. Strict mode was appropriate for that verification pass. The enhancement is forward-looking: once the pipeline is confirmed correct, a team using matchy for ongoing CI checks would want `responsive` mode to avoid re-flagging the same intentional downscales on every run.

## Suggested fix direction

- Add `image_dimensions_mode` to the profile TOML (`strict` | `responsive`).
- In `responsive` mode, compute and store `renderedWidth`/`renderedHeight` from `getBoundingClientRect()` in the page-capture phase alongside the existing `naturalWidth`/`naturalHeight` capture.
- Implement the 5-step gate above in the `changed_image_dimensions` detector.
- Add a tolerance parameter `aspect_ratio_tolerance_pct` (default: 2.0) to the profile for the aspect-ratio check.
- Document that `strict` mode is the recommended default for initial migration gates; `responsive` mode is for steady-state CI on responsive-first frontends.
