# Issue: `page_height_changed` reports the aggregate delta but does not attribute it per landmark or section

**Status:** FIXED (2026-06-11 — see ROOT-CAUSE-AND-PLAN.md and docs/golden-changelog.md)
**Found:** 2026-06-11, during a real migration gate (Webflow staging page vs local Next.js port)
**Severity:** P2 — the issue tells you the page shrank by ~400 px but not where; locating the responsible section requires external bbox measurement outside matchy
**Area:** `packages/analyze/src/visual_diff.rs` (page height detector); `packages/analyze/src/contract.rs` (Issue/Locator shape)

---

## Summary

When `page_height_changed` fires, the issue carries the aggregate old and new page heights
and the pixel delta. The `locator.bboxOld` and `locator.bboxNew` are both `null`. No
landmark, section, or container breakdown is provided. On the number-registration page the
desktop viewport shrank from 4211 px to 3792 px (−419 px); determining where those pixels
went required running an external per-section bbox measurement. The real causes — a mid-page
banner section approximately 95 px too tall relative to Webflow, plus footer/chrome
differences — were not surfaced by matchy at all.

## Environment

- matchy 0.1.0 (d5f0713); Linux; node v24.15.0; Chrome Headless Shell 148.0.7778.96
  (pw chromium-headless-shell v1223)
- old=https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration
- new=http://localhost:3001/products/connect/number-registration
- Run directory: `/tmp/matchy-nr-10/`

## Reproduction

```bash
/home/admin/MatchyMatchy/target/release/matchy \
  --old https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration \
  --new http://localhost:3001/products/connect/number-registration \
  --out /tmp/matchy-nr-10 --markdown
```

## Observed

`/tmp/matchy-nr-10/diff-result.json` — two `page_height_changed` issues (one per viewport).
Desktop:

```json
{
  "type": "page_height_changed",
  "message": "Page height changed from 4211 to 3792 px",
  "evidence": {
    "delta": -419,
    "new": { "pageHeight": 3792 },
    "old": { "pageHeight": 4211 }
  },
  "locator": {
    "anchors": {
      "text": null, "role": null, "href": null, "alt": null,
      "ariaLabel": null, "nearestHeading": null,
      "landmark": null, "ordinalInLandmark": null
    },
    "cssSelectorOld": null,
    "cssSelectorNew": null,
    "bboxOld": null,
    "bboxNew": null,
    "seqIndexOld": null,
    "seqIndexNew": null
  }
}
```

All locator fields are null. The `evidence` object contains only the aggregate heights and
delta; no per-section breakdown. Bundle files confirm the raw heights:
`/tmp/matchy-nr-10/desktop/old.bundle.json` → `page.pageHeight: 4211`;
`/tmp/matchy-nr-10/desktop/new.bundle.json` → `page.pageHeight: 3792`.

The mobile viewport reported `Page height changed from 6390 to 6039 px` (−351 px) with the
same null locator structure.

## Expected

When page heights differ beyond the detection threshold, the issue's `evidence` should
include a per-landmark or per-top-level-container breakdown showing the height contribution
from each section on both sides, so the reviewer can see which section(s) account for the
delta. For example:

```json
"evidence": {
  "delta": -419,
  "old": { "pageHeight": 4211 },
  "new": { "pageHeight": 3792 },
  "sectionDeltas": [
    { "landmark": "header", "role": "banner", "oldHeight": 72, "newHeight": 72, "delta": 0 },
    { "landmark": "main › section[2]", "role": "region", "oldHeight": 380, "newHeight": 285, "delta": -95 },
    { "landmark": "footer", "role": "contentinfo", "oldHeight": 310, "newHeight": 286, "delta": -24 }
  ]
}
```

This would immediately point to the banner section and the footer as the two responsible
areas without requiring external bbox measurement.

## Evidence

Bundle files (`old.bundle.json` and `new.bundle.json`) in the `desktop/` subdirectory
both include a `page.pageHeight` field (confirmed: 4211 and 3792 respectively). The
`page.landmarks` array in each bundle (visible in the schema at `contract/capture-bundle.schema.json`)
provides the bounding boxes of top-level landmark elements. Those bbox values are captured
at capture time and available for comparison in the analyze pass — they are just not used
in the `page_height_changed` issue construction.

From `diff-result.json` (run nr-10): `bboxOld: null` and `bboxNew: null` on both
`page_height_changed` issues, confirming no section attribution is computed today.

## Suggested fix direction

- In the `page_height_changed` detector, after computing the aggregate delta, iterate the
  `landmarks` arrays from both bundles, pair them by role/label, compute per-landmark
  height deltas, and embed the top contributors (|delta| > threshold, e.g. 20 px) in the
  `evidence.sectionDeltas` array.
- For landmarks that are present in one bundle but missing in the other (entire section
  added or removed), include them with the partner height set to 0.
- The `locator.bboxOld` / `locator.bboxNew` fields could point to the top contributing
  landmark's bounding box to enable viewport-overlay highlighting in the HTML report.
- If the `landmarks` array is not populated at capture time for the target page, fall back
  to reporting the delta without section attribution (current behavior) to avoid regressing
  any existing output contract.
