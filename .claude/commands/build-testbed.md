---
description: Build the page-pair-diff testbed (golden + permutation variants + expected outputs)
argument-hint: <golden-page-url>
---

Build the local testbed for page-pair-diff, golden page URL: $ARGUMENTS

Read `CLAUDE.md` and `docs/prds/page-pair-diff-spec.md` §12–13 first. Then orchestrate — delegate all
mechanical work per the routing policy; you personally do only planning, expected-output
authoring, and review.

## Phase 1 — Golden (delegate to fixture-builder)
Capture $ARGUMENTS into `testbed/golden/` per the fixture-builder's golden-capture procedure,
served at :3000. Review its CAPTURE-NOTES.md: confirm the determinism strips are sound and the
page still contains material to mutate (a form, a gradient or distinctive styling, ≥2 swappable
sections, links, images, headings). If it doesn't, tell me and propose a better target page
before continuing.

## Phase 2 — Variant plan (you, then delegate builds)
Produce a variant plan table covering at minimum, mapped to spec §13.1 and goals G1–G6:

| # | port | change | goal(s) |
|---|------|--------|---------|
| v01 | 3001 | identical control | — |
| v02 | 3002 | promo banner element added | visual/content |
| v03 | 3003 | font-size changed on body+headings | G1 |
| v04 | 3004 | font-family swapped | G1 |
| v05 | 3005 | spacing/color tweak on a CTA | G1 |
| v06 | 3006 | background gradient removed from hero/CTA | G4 |
| v07 | 3007 | two sections swapped | G3 |
| v08 | 3008 | form removed | G2 |
| v09 | 3009 | H1 text changed | G2 |
| v10 | 3010 | paragraph removed | G2 |
| v11 | 3011 | link target broken (404) | G2 |
| v12 | 3012 | image asset 404s | technical |
| v13 | 3013 | render-equivalent DOM change (spec §13.2) | matcher negative test |
| v14 | 3014 | same site under trailing-slash-violating URL | G5 |
| v15 | 3015 | same site under /es_MX/ path | G6 |

Adapt names/selectors to what the golden actually contains; one deliberate change each.
Dispatch fixture-builder per variant (batch where sensible), then review each manifest +
diff-against-golden summary yourself.

## Phase 3 — Expected outputs (you write these; do not delegate)
For each variant author `expected-issues.json`:
```jsonc
{
  "required": [
    { "type": "background_gradient_lost", "goal": "G4",
      "anchors": { "textContains": "Get started" },
      "evidence": { "property": "background-image", "fromContains": "linear-gradient", "to": "none|flat" } }
  ],
  "forbidden": [ { "type": "missing_link" }, { "type": "added_*_for_render_equivalent" } ],
  "status": "fail",
  "notes": "intent + any accepted knock-on issues from the manifest"
}
```
Match against the spec §7.3 taxonomy exactly. Specify the minimum that pins the intent; do not
over-specify pixel counts or scores. v01 must require `status: pass` and zero issues; v13 must
forbid missing/added for the wrapped CTA.

## Phase 4 — Harness + verification
Have code-implementer write `testbed/run-all.py` (start/stop/--check all servers) and a JSON-Schema
validator for manifests + expected-issues files, wired as `make testbed-up` / `make testbed-check`.
Then dispatch test-runner on `make testbed-check`. Finally write `docs/testbed-report.md`
summarizing every variant (port, change, goal, expected issues) and show me the report.
