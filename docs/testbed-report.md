# Testbed report — page-pair-diff

Built 2026-06-10. Golden page: <https://www.hiya.com/products/connect/branded-call>.

## Golden (`testbed/golden/`, port 3000)

- 99 files, ~4.5 MB, fully self-contained: all CSS/JS/images and three font families (Catamaran,
  Nunito Sans, Eina01) vendored locally; zero render-time third-party fetches.
- 26 determinism strips recorded in `testbed/golden/CAPTURE-NOTES.md` (GTM, reCAPTCHA v2/v3,
  Weglot, WebFont loader removed; publish-timestamp comment removed; copyright year pinned to
  2026; one CDN-403 asset stubbed with a zero-byte placeholder — not visible in any rendered
  section).
- Material inventory: H1 "Reach more customers with Hiya's Branded Call", 13 `<section>` elements
  in two sibling groups, 161 links, 103 images, 75 `linear-gradient` rules in the stylesheet.
- **Known gap:** the page has **no static `<form>`** (HubSpot injects forms at runtime), so spec
  §13.1.5 (`missing_form`, critical) cannot be exercised by this golden. v08 was adapted to a
  missing-CTA fixture (still G2). **Recommendation:** add a small synthetic fixture pair later for
  `missing_form` / `missing_form_field` / `missing_submit` coverage, or use a second real page
  that embeds a static form.
- The golden is FROZEN. All variants are single-change copies of it.

## Variants

| # | dir | port | change (one deliberate edit) | goal(s) | required expected issues | key forbidden |
|---|-----|------|------------------------------|---------|--------------------------|---------------|
| v01 | v01-identical | 3001 | none — byte-identical control | — | none; `status: pass`, `maxIssues: 0` | any issue at all |
| v02 | v02-banner-added | 3002 | promo banner div (text-only) inserted before the hero section | visual | `visual_region_changed` @ "20% off" | `missing_*`, mis-paired hero text |
| v03 | v03-font-size | 3003 | six `:root` type-scale variables: h1/h2 −20%, paragraph-body −2px | G1 | `style_changed(font-size)` on H1 | `missing_h1`, `changed_h1` |
| v04 | v04-font-family | 3004 | `--_typography---fonts--secondary-font`: "Nunito Sans" → Georgia | G1 | `style_changed(font-family)` on body paragraph, from "Nunito Sans" to "Georgia" | font-family change on H1 (Eina untouched) |
| v05 | v05-cta-style | 3005 | `.button_content`: background-color → `#d97706`, padding `1em 2em` → `1em 1em` (class shared by 6 buttons — declared) | G1 | `style_changed(background-color)` on "See pricing and sign up" | `missing_button`, `changed_cta` on it |
| v06 | v06-gradient-removed | 3006 | hero page-section rule (`.page-section:where(.w-variant-56a9f9bb-…)`, class verified in static HTML): gradient → `none` | G4 | `background_gradient_lost` with from `linear-gradient…` / to `none`, anchored at hero | generic `style_changed(background-image)` double-count, `missing_*` |
| v07 | v07-sections-swapped | 3007 | adjacent feature sections "Display your company's name…" ↔ "Branded Call performance analytics" swapped in place | G3 | single `component_swapped` | `missing_*` for either section, `component_reordered` |
| v08 | v08-cta-removed | 3008 | hero secondary CTA "Get a Demo" (`get-a-demo`) removed — **adapted from §13.1.5 missing-form; page has no static form** | G2 | `missing_button`\|`missing_link` anchored to the HERO (ordinal-disambiguation test: a twin anchor survives in the Problem section) | missing on sibling primary CTA; `changed_link_target` on surviving twin |
| v09 | v09-h1-changed | 3009 | H1 text "Reach more customers with" → "Connect with more customers using" | G2 | `changed_h1` with old/new evidence | `missing_h1`, `missing_text` |
| v10 | v10-paragraph-removed | 3010 | main-content Problem-section paragraph "Spam, robocalls, and scams…" removed | G2 | `missing_text` anchored under that heading | `changed_text` on it (mis-pairing trap) |
| v11 | v11-broken-link | 3011 | main-content "Free Call Inspection" href: absolute hiya.com → `/free-call-inspection` (404s locally) | G2, G7 | `broken_link` | `missing_link` (identity must pair); `changed_link_target` accepted as legitimate co-detection |
| v12 | v12-image-404 | 3012 | analytics-dashboard PNG deleted from assets; `<img>` left in DOM (alt empty) | G7, technical | `broken_image` \| `network_error` anchored to "performance analytics" section | `missing_image` |
| v13 | v13-render-equivalent | 3013 | §13.2 verbatim: hero primary CTA wrapped in `display:contents` div + `role="button"`; class/href/text byte-identical | matcher negative | none; `status: pass`, `maxIssues: 0` (default profile) | `missing_*`/changed on the CTA, any visual or style issue |
| v14 | v14-trailing-slash | 3014 | no content change; served at `/products/connect/branded-call/` (bare path 301s to slash form). urlUnderTest in manifest | G5 | `url_trailing_slash` (assumes `--trailing never`) | all content/style/visual issues, `locale_*`, `url_redirect_chain` |
| v15 | v15-locale-underscore | 3015 | no content change; served at `/es_MX/products/connect/branded-call` (200, no redirect) | G6 | `locale_separator_invalid` with `/es_MX/…` → `/es-MX/…` remediation | `locale_case_invalid`, `locale_unknown`, `url_trailing_slash`, content issues |

Every variant carries `manifest.json` (single change, exact edit, declared knock-ons, diff-vs-golden
summary) and a hand-authored `expected-issues.json` (intent tier). Knock-on effects accepted in the
expectations are exactly those declared in the manifests.

## Harness

- `testbed/run-all.py start|stop|check` — starts golden + all 15 variants (PID-tracked,
  idempotent), stops them, or runs the full check.
- `make testbed-up` / `make testbed-down` / `make testbed-check`.
- `check` validates every manifest against `testbed/schemas/manifest.schema.json` and every
  expectation against `testbed/schemas/expected-issues.schema.json` (jsonschema 4.10.3), enforces
  port uniqueness and name/dir consistency, and HTTP-checks each server at its manifest
  `urlUnderTest` (redirects count as failure).
- **Current status: `make testbed-check` → `TESTBED CHECK: PASS (16/16 ok)`, exit 0, no orphan
  servers.**

## Audit trail

Initial authorship of all 15 expectation files was audited by the golden-auditor: 13 APPROVE,
2 REJECT (v03: matcher pinned to a paragraph governed by an untouched CSS variable chain;
v10: U+2019 vs ASCII apostrophe in an anchor substring). Both corrected and re-audited. Full
rationale and standing conditions (v14 `--trailing never` pin; M6 revisit of hygiene status bands)
in `docs/golden-changelog.md`.
