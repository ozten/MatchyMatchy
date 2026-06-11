# Issue: `report.md` issues section is a flat table with per-viewport duplicate rows; triage requires loading `diff-result.json` and grouping manually

**Status:** FIXED (2026-06-11 — see ROOT-CAUSE-AND-PLAN.md and docs/golden-changelog.md)
**Found:** 2026-06-11, during a real migration gate (Webflow staging page vs local Next.js port)
**Severity:** P2 — on a real page the flat table runs to 1700+ rows; identical findings
repeat once per viewport, making the report nearly unusable for triage without external tooling
**Area:** `packages/analyze/src/report/markdown.rs` — `render_markdown()`

---

## Summary

`report.md` renders all issues as a single flat Markdown table in fix-value order (the order
from `diff-result.json`). There is no grouping by page section, landmark, or nearest heading,
and there is no per-viewport deduplication. Issues that appear on both desktop and mobile
viewports are emitted as two separate table rows with identical content except the implicit
viewport column (which is not even shown in the table). On the number-registration page
comparison, the 1705-issue run produced a 1791-line `report.md` where the same FAQ button
removals appear once for desktop and again for mobile with no indication that they are the
same logical finding.

Triage of this run required loading `diff-result.json` externally and grouping by
`locator.anchors.landmark` + `locator.anchors.nearestHeading`, which collapsed 1705 issues
into roughly eight page sections (hero, registration form, benefits, FAQ, footer navigation,
etc.) plus chrome/global issues. That grouping is not reflected anywhere in `report.md`.

## Environment

- matchy 0.1.0 (d5f0713); Linux; node v24.15.0; Chrome Headless Shell 148.0.7778.96
  (pw chromium-headless-shell v1223)
- old=https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration
- new=http://localhost:3001/products/connect/number-registration
- Run directory: `/tmp/matchy-nr-1/`

## Reproduction

```bash
/home/admin/MatchyMatchy/target/release/matchy \
  --old https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration \
  --new http://localhost:3001/products/connect/number-registration \
  --out /tmp/matchy-nr-1 --markdown
```

## Observed

`/tmp/matchy-nr-1/report.md` — 1791 lines. The Issues section is a single table:

```
## Issues

| # | Type | Severity | Goal | Message |
|---|---|---|---|---|
| 1 | url_protocol_downgrade | error | G5 | Per-link protocol downgrade: http://localhost:3001/products/connect/pricing should be https |
| 2 | url_protocol_downgrade | error | G5 | Protocol downgrade: old=https://... new=http://... |
| 3 | url_protocol_downgrade | error | G5 | Protocol downgrade: old=https://... new=http://... |
| 4 | url_protocol_downgrade | error | G5 | Per-link protocol downgrade: http://localhost:3001/products/connect/pricing should be https |
| 5 | broken_link | error | G7 | Broken link: http://localhost:3001/products/connect/pricing returned 404 |
...
| 9 | missing_button | error | G2 | Button removed: 'What will my customers see on their caller ID after I register? |
...
```

The FAQ "Button removed" rows appear for **mobile** (rows 9–16 approx.) and again
identically for **desktop** hundreds of rows later. Neither the landmark (`main`) nor the
nearest heading (`FAQs`) is surfaced in the table. The Summary section lists total counts
by type but no section breakdown:

```
## Summary

- **Fixable now:** 1630
- **Cluster count:** 34
- **Top fixes:** issue_2139612804fc, issue_47fed783ee7f, ...

**By type:**

- missing_button: 14
- missing_link: 70
- style_changed: 1487
...
```

`diff-result.json` shows that the FAQ button issues carry `locator.anchors.landmark = "main"`
and `locator.anchors.nearestHeading = "FAQs"`, so the grouping data exists in the underlying
JSON — it simply is not used in the Markdown renderer.

## Expected

A hierarchical Issues section grouped by `landmark` → `nearestHeading` → issue type, with
viewport badges inline rather than duplicate rows. Example shape:

```
## Issues by section

### main › FAQs (14 issues)

| Type | Viewports | Count | Message |
|---|---|---|---|
| missing_button | desktop mobile | 6 | Button removed: 'What will my customers see...' |
| missing_button | desktop mobile | 6 | Button removed: 'How long does it take...' |
...

### footer › (navigation) (72 issues)
...
```

The Summary should include per-section counts so a reviewer can see at a glance which
section is the largest source of issues before reading the table.

## Evidence

Source: `packages/analyze/src/report/markdown.rs`, `render_markdown()` (lines 102–153).
The renderer iterates `result.issues` in order and emits one row per issue. The `viewport`
field on each `Issue` is not included as a column (only `Type`, `Severity`, `Goal`,
`Message`). The `Locator.anchors.landmark` and `Locator.anchors.nearestHeading` fields are
not read anywhere in the Markdown renderer.

From `/tmp/matchy-nr-1/diff-result.json`: 1705 total issues. Of the 14 `missing_button`
issues, seven distinct questions appear in both `desktop` and `mobile` viewports (confirmed
by grouping `message` field), yielding 14 rows for 7 logical findings. The `locator` for
each carries `"landmark": "main"` and `"nearestHeading": "FAQs"`.

## Suggested fix direction

- In `render_markdown()`, build a `BTreeMap<(landmark, nearestHeading), Vec<&Issue>>`
  grouping before rendering. Emit one `###` subsection per group.
- Within each group, further fold by `(issue_type, message_normalized)`, collecting
  viewport names into a sorted badge list (`desktop`, `mobile`), to eliminate the
  per-viewport duplicate rows.
- Add a per-section count table to the `## Summary` section (section name → issue count).
- The flat table can be retained as an appendix or removed entirely; the grouped view
  should be the primary surface.
