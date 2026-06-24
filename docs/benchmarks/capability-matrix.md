# matchy vs SiteDiff vs Wraith — capability synthesis

A side-by-side of what each tool *claims*, how it *works*, and where matchy provides or lacks the
same capability. Sourced from the [SiteDiff](tool-research-sitediff.md) and
[Wraith](tool-research-wraith.md) research briefings and the matchy spec/README. The numbers
behind the claims come from the two benchmarks ([matchy vs SiteDiff](matchy-vs-sitediff.md),
[matchy vs Wraith](matchy-vs-wraith.md)) run over the shared 21-variant corpus.

## One-paragraph positioning

**SiteDiff** is a breadth-first **HTML-text** differ for migrations: crawl many URLs, fetch raw
HTML over curl, normalize, and show a line-level markup diff. **Wraith** is a **pixel** differ for
responsive visual regression: screenshot two environments at several widths and report the percent
of pixels that changed. **matchy** is a depth-first **single-pair semantic** differ: render both
pages in a real browser and emit *typed, located, fixable* defects across content, style, structure,
assets, URL/transport hygiene, runtime, and accessibility — a superset of the *signals* SiteDiff and
Wraith each expose on one axis, traded against their breadth (SiteDiff's crawler) and simplicity.

## What each tool claims to do

| | SiteDiff | Wraith | matchy |
|---|---|---|---|
| Tagline | "see how a website changes by comparing two similar sites" | responsive visual regression (live vs staging) | "what broke in the migration," as a machine-actionable diff |
| Primary job | migration / CMS-upgrade content QA at scale | catch unintended *visual* regressions across breakpoints | deep per-page old↔new defect report for migrations |
| Unit of work | many paths (crawled) | many paths × many widths | one explicit URL pair |
| Output | per-path HTML diff report + `serve` UI | gallery of old/new/diff thumbnails + % | typed JSON `DiffResult` (+ HTML/MD) |
| Comparison model | same path on two base URLs | same path on two base URLs | two arbitrary URLs (paths may differ) |

## Mechanism

| | SiteDiff | Wraith | matchy |
|---|---|---|---|
| Fetch | HTTP (typhoeus/libcurl) | headless browser screenshot | Playwright/Chromium render |
| JS executed | ❌ no | ✅ (whatever the engine supports) | ✅ yes |
| Compares | normalized HTML text (diffy) | rendered pixels (ImageMagick `compare`) | DOM model + computed style + pixels + transport + axe |
| Noise control | regex `sanitization` + `dom_transform` | `fuzz` (anti-alias) + `threshold` | confidence bands, identity matching, `--hide`/`--mask`/`--baseline` |
| Determinism | text diff (stable) | pixel (stabilized by fuzz) | pure-function analyze layer, byte-deterministic |

## Strengths (each tool's genuine edge)

- **SiteDiff** — breadth: one crawl covers hundreds of pages. Exact markup deltas when content
  changed. No browser → trivial, fast CI. Mature normalization rule library (Drupal presets).
  Actively maintained (v1.2.11, Aug 2024).
- **Wraith** — pixel-truth: the honest arbiter of *visual* equivalence; catches any visible change
  regardless of cause, across multiple breakpoints; correctly ignores render-equivalent DOM
  rewrites. Dead-simple mental model (one % per page per width).
- **matchy** — semantics: every defect is *typed, located (greppable anchor), and remediated*.
  Covers channels the other two can't see (transport, runtime, a11y). No-false-positive identity
  matching. Deterministic, agent/CI-first JSON contract.

## Weaknesses (and whether matchy shares them)

| Weakness | SiteDiff | Wraith | matchy shares it? |
|---|---|---|---|
| Blind to CSS-only visual change | ✅ yes (HTML unchanged) | ❌ no | ❌ matchy catches (style/computed-style) |
| Blind to non-visual defects (links/console/a11y/transport) | partial | ✅ yes | ❌ matchy catches all |
| False-positive on render-equivalent markup rewrite | ✅ yes | ❌ no | ❌ matchy matches & stays quiet |
| No *semantic* label (what/why) | mostly (raw diff) | ✅ yes (only %) | ❌ matchy names the defect |
| Needs a (working) browser | ❌ (curl only) | ✅ — and its engine is **dead on aarch64** | ✅ needs Chromium (modern, works) |
| Only one URL pair (no crawl) | ❌ crawls | ❌ crawls/spiders | ✅ **yes — matchy v1 is single-pair** |
| Dynamic-content false positives | medium (regex) | high | low (matching + hide/mask) |
| Maintenance risk | low (active) | **archived Jan 2026** | low (active) |

## How matchy provides — or doesn't — each tool's signature capability

**SiteDiff's signature: line-level HTML markup diff across a crawled site.**
- *matchy provides:* the *semantic* content delta (missing/changed headings, paragraphs, CTAs,
  forms, link targets) as typed issues — usually more useful than a raw markup diff, and it won't
  false-positive on render-equivalent rewrites.
- *matchy does NOT provide:* (1) **crawling / many-paths-in-one-run** — matchy v1 takes a single
  explicit pair; SiteDiff's crawler is a real, unmatched advantage for broad coverage. (2) the
  **exact textual markup edit** — for added content (e.g. a new banner `<div>`), SiteDiff names the
  inserted markup, whereas matchy may report it only as visual regions (see `v02-banner-added`).
  (3) **curl-only, zero-browser** operation.

**Wraith's signature: multi-breakpoint pixel-diff %.**
- *matchy provides:* its own pixel-diff layer (`visual_region_changed`, `page_height_changed`,
  clustered bounding boxes) across multiple viewports, plus the semantic reason behind the pixels.
- *matchy does NOT provide:* a single, dead-simple *“% of pixels changed”* score sorted worst-first
  for fast human visual triage — Wraith's gallery UX is purpose-built for eyeballing visual drift,
  and some teams prefer that raw, assumption-free signal over typed issues.

## Bottom line

The three tools sit on a breadth↔depth and pixels↔semantics plane. SiteDiff = broad + markup.
Wraith = broad + pixels. matchy = deep + semantic (and ships a pixel layer of its own). For a
migration, SiteDiff/Wraith answer *“did anything change on these many pages (in markup / in
pixels)?”*; matchy answers *“on this page, exactly what broke, where, and how do I fix it?”* — and
catches the non-visual, non-markup defect classes (transport, runtime, a11y) that neither competitor
models at all. The honest gap in matchy's favor-or-not column: it cannot yet crawl, and for
pure-markup or pure-visual triage at scale the older tools remain simpler to point at a whole site.
