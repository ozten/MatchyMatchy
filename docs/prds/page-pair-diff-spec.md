# Page Pair Diff — Agent-First Specification (v2)

> **Audience:** an agentic coding tool implementing this project.
> **Status:** authoritative build spec. Where this document conflicts with any prior draft, this document wins.
> **One-line mission:** Given URL A (old) and URL B (new), produce a deterministic, machine-actionable diff that lets an agent fix migration defects with minimal additional investigation.

---

## 0. How to read this spec (for the implementing agent)

- The **primary deliverable is the `DiffResult` JSON contract** (Section 7), not the HTML report. The HTML report is one renderer of that contract.
- Build in the **milestone order** in Section 12. Each milestone has acceptance fixtures. Do not advance until fixtures pass.
- Prefer **deterministic evidence** over inference everywhere. Any AI/LLM step must cite deterministic artifacts and must be disableable.
- The tool **never names the source component.** It emits a greppable **anchor set** (Section 5) that maps to repo source; the downstream agent — which has repo access — resolves the component. Do not fabricate component identity.

---

## 1. Goal traceability (these drive priority)

Every feature exists to serve a concrete user goal. The implementing agent must keep this table satisfied; a feature that does not trace to a goal is out of scope for v1.

| # | User goal | Primary subsystem(s) | Milestone |
|---|---|---|---|
| G1 | Which component needs CSS tweaks | Computed-Style Diff + Anchor Locator | M4 |
| G2 | Which content is missing | Semantic Diff (text/link/image/form) | M3 |
| G3 | Which component order is wrong / swapped | Sequence Diff | M5 |
| G4 | Visually different + missing background gradient | Computed-Style Diff (background/gradient) | M4 |
| G5 | URL has a trailing slash it shouldn't | URL & Redirect Hygiene | M2 |
| G6 | Locale path wrong case (`es-mx` vs `es_MX`) | Locale Hygiene | M2 |
| G+ | Missing/changed headings, CTAs, forms, a11y, broken assets | Semantic Diff + A11y + Network | M3/M6 |

The four goals the original draft deferred or omitted (G1, G3, G4, G5/G6) are **promoted into v1** here.

---

## 2. Scope

### In scope (v1)
- Single explicit URL pair: `--old <url> --new <url>`.
- Real-browser rendering via Playwright (Chromium), full-page + viewport screenshots.
- Deterministic page stabilization.
- Semantic page-model extraction (rendered, visible content).
- Computed-style extraction for candidate elements.
- Multi-signal element matching across differing DOMs.
- Visual diff, content diff, **computed-style diff**, **sequence/order diff**, link/image/form diff, a11y diff, network/console diff.
- **URL/redirect/canonical hygiene** and **locale-path hygiene**.
- Stable JSON `DiffResult` (the contract), plus HTML and Markdown renderers.
- Parity profiles (Section 9).

### Out of scope (v1)
- Full-site crawling, sitemap discovery, batch CSV mode (post-v1).
- SEO suite parity (Screaming Frog replacement).
- Proving business-logic / transactional correctness.
- Submitting production forms or performing auth actions by default.
- Reliable recovery of minified source-component names (best-effort only; see Section 5).

---

## 3. Architecture

### 3.1 Decision: hybrid TS capture + Rust analysis, joined by a JSON seam

```
            ┌─────────────────────────────┐
  --old/--new ──▶  capture (TypeScript)    │   drives Playwright,
            │  • Chromium, stabilization   │   runs in-page extraction JS,
            │  • screenshots               │   reads getComputedStyle,
            │  • PageModel + ComputedStyle │   captures network/console/a11y
            │  • network/console/a11y tree │
            └───────────────┬─────────────┘
                            │  writes CaptureBundle JSON + PNGs  (the seam)
                            ▼
            ┌─────────────────────────────┐
            │  analyze (Rust)             │   pure, deterministic, fast:
            │  • visual diff              │   image diff, matching,
            │  • element matching         │   sequence/style diff,
            │  • sequence diff            │   hygiene checks, scoring,
            │  • computed-style diff      │   report rendering
            │  • url/locale hygiene       │
            │  • scoring + issue gen      │
            │  • DiffResult + renderers   │
            └───────────────┬─────────────┘
                            ▼
              DiffResult JSON  ──▶  HTML / Markdown / agent consumer
```

**Rationale.** The browser layer genuinely wants Playwright (auto-waiting, `axe-core/playwright`, stabilization maturity); in-page extraction needs injected JS regardless of host language. The analysis layer is pure, perf-sensitive, correctness-critical work suited to Rust. The JSON seam is also the agent-first artifact boundary and makes each half independently testable.

**Allowed fallback (faster MVP):** implement `analyze` in TypeScript first, behind the *exact same* `CaptureBundle` → `DiffResult` contract, then port the analysis core to Rust later without changing the interface. The contract is the invariant; the language of the core is not.

### 3.2 Workspace layout

```
page-pair-diff/
  packages/
    capture/                # TypeScript (Node), Playwright
      src/
        browser-runner.ts
        stabilizer.ts
        extract/            # in-page extraction (runs in browser context)
          page-model.ts
          computed-style.ts
          a11y.ts
        capture.ts          # orchestrates, emits CaptureBundle
        schema.ts           # zod schemas mirroring the contract
      package.json
    analyze/                # Rust (or TS fallback) — the core
      src/
        bin/ppd.rs          # CLI (clap)
        contract.rs         # serde structs mirroring the contract
        visual_diff.rs
        matching.rs
        sequence_diff.rs
        style_diff.rs
        hygiene_url.rs
        hygiene_locale.rs
        semantic_diff.rs
        scoring.rs
        issue.rs
        report/{html.rs, markdown.rs, json.rs}
      Cargo.toml
  contract/                 # SINGLE SOURCE OF TRUTH for JSON schema
    capture-bundle.schema.json
    diff-result.schema.json
  fixtures/                 # local HTML pairs (Section 13)
  README.md
```

The contract lives in `/contract` as JSON Schema. The TS `zod` schemas and Rust `serde` structs are both generated from / validated against it in CI. Schema drift between the two languages is a build failure.

### 3.3 Determinism vs. stability (cross-cutting — read before implementing either layer)

These are two different properties and must not be conflated.

**Determinism** = same input → same output, exactly. The **analyze layer is deterministic by construction**: it is a pure function `(CaptureBundle_old, CaptureBundle_new) → DiffResult`. The only ways to break this are engineering defects, all of which the implementation MUST avoid:
- Iteration order: never iterate `std::collections::HashMap`/JS `Object` key order for anything that affects output. Use `BTreeMap`, or collect-and-sort by a stable key.
- Tie-breaks: the assignment step (Section 6) and any sort MUST define a **total order**; break ties on a stable key (node `id`), never on insertion order or address.
- Float reductions: sum/aggregate scores in a fixed, sorted order so results don't depend on accumulation order across threads.
- Concurrency: parallelism is allowed but results MUST be reassembled in a deterministic order before scoring/serialization.

Given the above, identical bundles MUST produce a byte-identical `DiffResult` (modulo the `runId`/timestamp fields, which are excluded from golden comparisons).

**The capture layer is deterministic only within bounds it does not fully control**, and the spec is honest about the two leaks:
1. **Rendering environment.** Subpixel rasterization, font hinting, and image decoding vary by OS / GPU / Chromium version. Pixel-level visual diff is therefore stable **only within a pinned environment** — this is the real reason the Docker image pins Chromium + fonts, not just CI convenience. All golden/visual baselines are environment-scoped; comparing across environments is unsupported.
2. **Page nondeterminism.** Time, randomness, animation, lazy-load races, A/B tests, ads. The Section 4.2 steps suppress the controllable sources (freeze time/RNG/rAF, wait for fonts/images, kill animation, mask volatile regions), but an adversarial live page cannot be driven to 100%. Residual nondeterminism MUST be **masked or surfaced as low-confidence**, never allowed to silently vary the result.

Design target, stated plainly: *analysis is fully deterministic; capture is deterministic within a pinned environment and to the extent the page cooperates, with the uncontrollable residue masked or flagged.*

**Stability is a separate concern.** A hard threshold (e.g. "match if score ≥ 0.7") is perfectly deterministic yet brittle: a 0.699 vs 0.701 score flips an issue on/off, so tiny capture noise changes output. Solve this with **confidence bands / hysteresis**, not by weakening determinism: classify `>= matchFloor` as matched, `< noMatchCeil` as unmatched, and the band between as `uncertain` (emitted with low confidence for agent/human review rather than a coin-flip verdict). Thresholds live in config so a run's verdicts are auditable and reproducible.

---

## 4. Capture layer (TypeScript) — `CaptureBundle`

### 4.1 Browser runner
- Chromium via Playwright. WebKit/Firefox post-v1.
- Per-run isolated `BrowserContext`. Old and new use **identical** viewport, deviceScaleFactor, locale, timezone, user agent, and color scheme.
- Configurable: viewports (default `desktop 1440x1000`, `mobile 390x844`), timeout, retries.

### 4.2 Determinism guarantees (hard requirements)
Before any capture, inject:
- Animation/transition kill CSS (`animation: none`, `transition: none`, `scroll-behavior: auto`, `caret-color: transparent`).
- `prefers-reduced-motion: reduce` emulation.
- Freeze nondeterminism in page context: stub `Date.now`/`new Date()` to a fixed epoch, seed/stub `Math.random`, neutralize `requestAnimationFrame` timing where it drives visuals.
- Wait for: load → networkidle (configurable) → `document.fonts.ready` → all in-viewport `<img>` `decode()`.
- Lazy-load pass: scroll to bottom in steps, then back to top; re-wait for fonts/images.
- Apply configured `hideSelectors` (visibility:hidden) and `maskSelectors` (neutral fill, preserves layout).
- Apply configured `clickBeforeCapture` (e.g. cookie accept), each optional.

A capture is only valid if the determinism steps completed; record which ran in `CaptureBundle.determinism`.

### 4.3 Page model extraction (in-browser, post-stabilization)
Extract **rendered, visible** content. Visible = non-empty bbox, not `display:none`/`visibility:hidden`, not fully transparent, within page bounds. Capture screen-reader-only labels separately for a11y.

Per page, emit:
```jsonc
{
  "url": "https://...",
  "finalUrl": "https://...",          // after redirects
  "redirectChain": ["https://...","https://..."],
  "statusCode": 200,
  "title": "…",
  "metaDescription": "…",
  "canonical": "https://…",
  "lang": "es-MX",                     // <html lang>
  "pageHeight": 6420,
  "nodes": [ /* SemanticNode[] — the ordered node stream, see 6.1 */ ],
  "landmarks": ["banner","navigation","main","contentinfo"],
  "network": { "requests": [ /* {url,status,type,failed} */ ] },
  "console": [ /* {level,text} */ ],
  "a11y": { "violations": [ /* axe results */ ] }
}
```

### 4.4 Computed-style capture (scoped — performance critical)
Do **not** read `getComputedStyle` for every node. Read it only for **candidate elements**: headings, links, buttons/CTAs, form controls, images, and any element the matcher later flags as "changed region." Capture a curated, normalized property set:

```
color, background-color, background-image, background (shorthand),
border, border-radius, box-shadow,
font-family, font-size, font-weight, line-height, letter-spacing, text-align,
padding(-*), margin(-*),
display, position, opacity,
flex-direction, justify-content, align-items, gap, grid-template-columns
```

For **gradients (G4)**: from `background-image`, detect and parse `linear-gradient`/`radial-gradient`/`conic-gradient` into `{kind, angle, stops[]}`. Presence/absence and stop deltas are first-class.

Normalize colors to a canonical form (e.g. `rgb()`/`rgba()` lowercased) so `#fff` vs `white` vs `rgb(255,255,255)` do not produce false diffs.

### 4.5 `CaptureBundle` (the seam)
```jsonc
{
  "schemaVersion": "1.0",
  "capturedAt": "2026-06-10T…Z",
  "viewport": { "name": "desktop", "width": 1440, "height": 1000, "dsf": 1 },
  "determinism": { "animationsDisabled": true, "fontsReady": true, "lazyLoadPass": true, "clicked": ["button.accept"] },
  "page": { /* page model from 4.3 */ },
  "computedStyles": { "node_42": { /* curated props */ } },
  "screenshots": { "fullPage": "desktop/old.png", "viewport": "desktop/old-vp.png" }
}
```
The analyze layer receives **two** bundles (old, new) per viewport.

---

## 5. Anchor Locator (serves G1, G3 — replaces source-component naming)

The tool does **not** try to recover a React/source component name, and does **not** require the team to emit `data-component`. It runs against the vanilla staging preview as-is. Instead, every node carries an **anchor set**: a small, greppable fingerprint of *semantic* facts that appear verbatim in repo source, so a repo-aware agent finds the code in roughly one `rg`.

**Why not XPath / structural selectors as the agent-facing identity.** Absolute positional XPath (`/html/body/div[2]/section[1]/a[1]`) is brittle (any DOM change invalidates it) and, more importantly, encodes *DOM position* — but the agent's task is grepping *source*, not traversing the DOM. In a Next.js/Sanity build, classNames are typically CSS-module hashes or Tailwind utility soup, so class selectors generally don't grep to a component file either. String literals — visible text, `href`, `alt`, `aria-label` — do appear in JSX and grep in one shot. So the anchor set, not a path, is what the agent acts on.

**Anchor set (the agent-facing locator):**
```jsonc
"anchors": {
  "text": "Donate today",          // strongest grep target when present
  "role": "link",
  "href": "/donate",               // grep target for links/CTAs
  "alt": null,                     // grep target for images
  "ariaLabel": null,
  "nearestHeading": "Support our work",
  "landmark": "main",              // banner|navigation|main|contentinfo|…
  "ordinalInLandmark": 2           // disambiguates repeats of the same text
}
```

**Tool-internal locator (re-find + crop only — never the thing handed to the agent):** the tool still needs *something* to re-find the element across runs and crop screenshots. Keep a short, **relative** CSS selector (scoped to the nearest landmark, not absolute) plus `bbox` and `seqIndex`. These live in the locator object but are flagged internal; the agent works from `anchors`.

**Token economics (the point of the change):** the agent receives a ~6-field fingerprint plus an implicit "grep for the text or href," not a 200-character path. Smaller *and* more actionable. The tradeoff accepted by dropping `data-component`: the tool cannot *name* the component — and it doesn't need to. It hands over a precise fingerprint; the agent resolves identity from source.

**Anchor quality / confidence.** Rank anchor strength so `localityBonus` (Section 7.2) can reward cheap-to-find issues: `href` or distinctive `text` ⇒ high; `alt`/`ariaLabel` ⇒ high; only `nearestHeading + landmark + ordinal` ⇒ medium; nothing distinctive (e.g. a bare decorative element) ⇒ low, and the issue is marked harder-to-locate rather than given a fake identity.

---

## 6. Element matching (the crux) — concrete algorithm

### 6.1 Semantic node stream
Each page is reduced to an **ordered list of `SemanticNode`** (document order over visible, meaningful elements). A `SemanticNode`:
```jsonc
{
  "id": "node_42",
  "kind": "heading|text|link|button|image|form|field|landmark|generic",
  "role": "link",
  "text": "Donate today",
  "accName": "Donate today",
  "href": "/donate",
  "imageAlt": null,
  "bbox": [120,720,180,48],
  "seqIndex": 31,
  "anchors": { "text": "Donate today", "role": "link", "href": "/donate", "nearestHeading": "Support our work", "landmark": "main", "ordinalInLandmark": 2 },
  "cssSelector": "main section.cta a"   // internal: relative, re-find/crop only
}
```

### 6.2 Matching
1. **Block by kind** (links match links, images match images, etc.; `generic` is a fallback bucket).
2. For each candidate pair within a block, compute a **weighted similarity score** — a fixed-constant linear combination of per-signal sub-scores, each normalized to `[0,1]`:

   ```
   score(a,b) = Σ wᵢ · signalᵢ(a,b)      where Σ wᵢ = 1
   ```

   The weights are **plain constants, not learned and not stochastic** — they encode "what identifies *this kind* of element," so they differ per block. Sub-scores:

   | Signal | Definition |
   |---|---|
   | `textSim` | token + edit-distance similarity of normalized visible text |
   | `accNameSim` | same, over accessible name |
   | `hrefSim` | `1` if normalized hrefs equal; partial if same path, differing query; else `0` |
   | `altSim` | similarity of image `alt`; plus intrinsic-dimension ratio |
   | `roleSim` | `1` if role/kind equal, else `0` |
   | `posSim` | `1 − min(1, |y_old_norm − y_new_norm|)` (normalized y in `[0,1]`) |
   | `sizeSim` | bbox area ratio `min/max` |
   | `nearbySim` | similarity of nearby text / enclosing landmark |

   **Default per-kind weight tables** (config-overridable; the *identity* signal dominates each):

   ```
   link/button:  href 0.45  text 0.30  pos 0.15  size 0.10
   image:        alt  0.40  sizeSim(intrinsic) 0.30  pos 0.20  size 0.10
   heading:      text 0.75  pos 0.15  size 0.10
   text block:   text 0.70  pos 0.20  nearby 0.10
   form/field:   accName 0.40  role 0.25  nearby 0.20  pos 0.15
   generic:      text 0.40  pos 0.30  size 0.20  role 0.10
   ```

3. **Constrained assignment**: solve as an assignment problem (Hungarian for small blocks; greedy with a similarity floor for large blocks), forbidding matches below the configurable `matchFloor` and preferring monotonic order. Ties broken on stable node `id` (see §3.3 determinism).
4. **Classify** using confidence bands, not a single hard cutoff (see §3.3 stability):
   - `score ≥ matchFloor` → `matched`
   - `noMatchCeil ≤ score < matchFloor` → `uncertain` (emitted low-confidence for review, not silently decided)
   - matched but `|seqIndexNew − seqIndexOld|` exceeds a threshold → candidate for **sequence diff** (Section 8)
   - old node below `noMatchCeil` against all candidates → `missing`
   - new node below `noMatchCeil` against all candidates → `added`
   - matched with differing attributes (href, text, alt, computed style) → emit the corresponding attribute-level issue

**Auditability requirement:** the per-signal sub-scores and the final `score` for each emitted match/non-match MUST be written into the issue evidence (`evidence.match`). This is what makes the tool forensic rather than magic — when it pairs (or refuses to pair) two elements, the reason is inspectable, and because the weights are constants the result is fully reproducible.

The matcher's output (matched pairs + unmatched sets) feeds every downstream diff so that "missing" and "added" are not double-counted as a "swap" (that is the sequence differ's job).

---

## 7. The contract: `DiffResult` (PRIMARY DELIVERABLE)

This is the product. Optimize it for an agent that will fix the issues.

```jsonc
{
  "schemaVersion": "1.0",
  "toolVersion": "0.1.0",
  "runId": "2026-06-10T14-00-00Z",
  "oldUrl": "https://old.example.com/about",
  "newUrl": "https://new.example.com/about",
  "parityProfile": "content-structure",
  "status": "fail",                        // pass | warn | fail | error
  "agentSummary": {                        // machine-first triage block, FIRST in file
    "fixableNow": 6,
    "byType": { "missing_form": 1, "style_changed": 3, "component_reordered": 1, "url_trailing_slash": 1 },
    "topFixes": ["issue_001","issue_004","issue_002"]  // ordered by fix value
  },
  "scores": { "visual":0.88,"content":0.94,"structure":0.80,"style":0.72,"capability":0.75,"accessibility":0.92,"technical":1.0,"hygiene":0.5 },
  "viewports": [ { "name":"desktop","status":"fail","issues":["issue_001"] } ],
  "issues": [ /* Issue[] */ ],
  "determinism": { "old": {/* … */}, "new": {/* … */} },
  "artifacts": { "old":"desktop/old.png","new":"desktop/new.png","diff":"desktop/diff.png" }
}
```

### 7.1 `Issue` (every field exists to reduce agent work)
```jsonc
{
  "id": "issue_004",                       // CONTENT-ADDRESSED: hash(type + locator + evidence)
  "type": "style_changed",
  "category": "style",                     // visual|content|structure|style|capability|accessibility|technical|hygiene
  "severity": "warning",                   // info|warning|error|critical
  "confidence": 0.86,
  "viewport": "desktop",
  "locale": "es-MX",
  "goal": "G4",                            // which user goal this serves, when applicable
  "message": "Hero CTA lost its background gradient.",
  "locator": {
    "anchors": {
      "text": "Get started", "role": "link", "href": "/signup",
      "nearestHeading": "Build faster", "landmark": "main", "ordinalInLandmark": 1
    },
    "cssSelectorOld": "section.hero a.btn",   // internal: relative, re-find/crop only
    "cssSelectorNew": "main section a.cta",   // internal
    "bboxOld": [120,720,180,48],
    "bboxNew": [120,760,160,48],
    "seqIndexOld": 6, "seqIndexNew": 6
  },
  "evidence": {
    "old": { "background-image": "linear-gradient(90deg, #6d28d9 0%, #2563eb 100%)" },
    "new": { "background-image": "none", "background-color": "rgb(109,40,217)" },
    "match": { "score": 0.91, "signals": { "href": 1.0, "text": 0.86, "pos": 0.97, "size": 0.9 } },
    "artifacts": { "oldCrop":"desktop/issues/issue_004_old.png", "newCrop":"desktop/issues/issue_004_new.png" }
  },
  "remediation": {                         // the agent acts on THIS
    "action": "restore_css_property",
    "findBy": { "grep": ["\"/signup\"", "Get started"], "near": "Build faster" },
    "property": "background-image",
    "from": "none",
    "to": "linear-gradient(90deg, #6d28d9 0%, #2563eb 100%)",
    "note": "New page replaced the gradient with a flat fill. Locate the CTA in source by grepping the href or label; the tool does not name the component."
  }
}
```

### 7.2 Issue ordering (fix value)
`issues` array is sorted by descending **fix value** = `severityWeight × confidence × localityBonus`, where `localityBonus` rewards issues whose `anchors` are strong/greppable (a distinctive `text` or `href` is cheap to find in source) over diffuse visual regions with weak anchors. The HTML report may re-sort; the JSON order is the agent's recommended work queue.

### 7.3 Issue taxonomy (stable strings)
```
# visual
visual_region_changed  page_height_changed
# content (G2)
missing_title changed_title missing_meta_description changed_meta_description
missing_h1 changed_h1 heading_structure_changed
missing_text changed_text duplicate_text
missing_link changed_link_target broken_link changed_link_text
missing_image broken_image changed_alt_text missing_alt_text changed_image_dimensions
missing_form changed_form missing_form_field changed_required_field missing_submit changed_cta missing_button
# structure (G3)
component_reordered component_swapped
# style (G1, G4)
style_changed background_gradient_lost background_gradient_changed
# capability
missing_capability nonfunctional_capability changed_capability capability_added
# accessibility
accessibility_regression accessibility_improved
# technical
network_error console_error load_error
# hygiene (G5, G6)
url_trailing_slash url_redirect_chain url_protocol_downgrade canonical_mismatch
locale_case_invalid locale_separator_invalid locale_unknown locale_parity_missing
```

---

## 8. Sequence / order diff (serves G3)

Inputs: matched pairs from Section 6 plus their `seqIndex` on each page.

1. Build the sequence of matched component identities in old order and in new order.
2. Compute the edit script / longest common subsequence over that sequence.
3. For a matched pair whose relative order changed: emit `component_reordered`. When two components exchange positions (A↔B), collapse to a single `component_swapped` with both locators.
4. Do **not** emit `missing`/`added` for these — they are reorders, not deletions. The matcher must have already paired them.

`remediation.action` for reorders = `reorder_components`, with `target`, `before`/`after` anchors, and the expected order list.

---

## 9. Parity profiles

A migration that intentionally restyles will trip every pixel diff; profiles control what counts as a failure.

```jsonc
"parityProfile": "content-structure"
```

| Profile | Visual diff | Style diff | Content | Structure | Capability | Hygiene | A11y |
|---|---|---|---|---|---|---|---|
| `strict-visual` | fail | fail | fail | fail | fail | fail | warn |
| `content-structure` (default) | info | warn | fail | fail | fail | fail | warn |
| `capability-only` | info | info | warn | warn | fail | fail | warn |

Profiles set default severities per category; explicit per-type severity config overrides them.

---

## 10. URL & locale hygiene (serves G5, G6)

### 10.1 URL/redirect/canonical (G5)
Run on the input URLs and on every extracted link:
- **Trailing slash** vs configured policy (`"trailing": "never" | "always" | "preserve"`). Mismatch → `url_trailing_slash`, remediation = rewrite to policy.
- **Redirect chain**: follow; if `redirectChain.length > 1` or a redirect occurs where none should, emit `url_redirect_chain` with the full chain.
- **Protocol**: any `http://` where the sibling is `https://` → `url_protocol_downgrade`.
- **Canonical**: `<link rel=canonical>` not matching `finalUrl` (mod policy) → `canonical_mismatch`.

### 10.2 Locale path (G6)
Detect a locale segment in the path (first or second segment). Validate against BCP-47:
- Region subtag must be **uppercase**, language **lowercase**: `es-MX` valid, `es-mx`/`ES-mx` → `locale_case_invalid`.
- Separator must be hyphen, not underscore: `es_MX` → `locale_separator_invalid`.
- Unknown language/region codes → `locale_unknown`.
- Optional cross-locale parity: if old exposes locale variants the new omits → `locale_parity_missing`.

Remediation for locale issues carries the corrected segment (`from: "/es_MX/about"`, `to: "/es-MX/about"`).

---

## 11. Visual, semantic, capability, a11y, network diffs

- **Visual diff:** full-page + viewport. Pixel comparison (Rust `image` crate; pixelmatch-style algorithm or `dssim` for perceptual). Output: diff image, changed-pixel %, **region clustering** (bounding boxes), page-height delta. Regions are linked to overlapping `SemanticNode`s so a region can name what changed. Under non-`strict-visual` profiles a pure region change is `info` *unless* it overlaps a matched node with a content/style/capability issue, which raises severity.
- **Semantic diff (G2):** title/meta/canonical/lang, headings + hierarchy, text blocks (grouped, normalized — see normalization rules), links, images (+alt, dimensions, load status), forms (+fields, labels, required, submit). Weight **main-content** over repeated chrome (header/footer nav classified separately).
- **Capability diff:** detect capabilities (nav, mobile menu, search, newsletter, contact form, accordion, tabs, carousel, video embed, download links, language switcher) from rendered signals; for each on old, check equivalent on new; run **shallow safe probes** (click menu/accordion/tab and assert DOM/visibility change; verify download URLs via HEAD; verify iframe presence). Emit `missing_capability` / `nonfunctional_capability` / `changed_capability`.
- **A11y diff:** `axe-core/playwright` on both; diff violation sets → `accessibility_regression` (new) / `accessibility_improved` (fixed); also changed accessible names on important controls, missing landmarks/labels, heading-hierarchy regressions.
- **Network/console:** failed requests, 4xx/5xx assets, CORS, mixed content, uncaught exceptions. New-only failures are issues; failures on both are noted but not scored against the new page.

**Text normalization:** collapse whitespace, trim, NBSP→space, optional smart-quote/punctuation/case folding. Never normalize away dates, names, prices, phone numbers, emails, legal text, product claims, CTA wording.

---

## 12. Milestones (build order — reordered to deliver user goals early)

**M1 — Capture + visual diff skeleton.** TS capture produces `CaptureBundle` (screenshots + stabilization). Analyze produces `old.png/new.png/diff.png`, page-height delta, and a `DiffResult` with `visual_region_changed`. *DoD:* two URLs → artifacts + valid `DiffResult` JSON validated against schema.

**M2 — URL & locale hygiene (G5, G6).** Implement Section 10 fully. Cheap, high-signal, validates the contract end-to-end. *DoD:* fixtures for trailing slash, redirect chain, `es_MX`, `es-mx` produce the correct hygiene issues with correct remediation.

**M3 — Semantic extraction + content diff (G2).** Node stream, matching (Section 6), content/link/image/form issues. *DoD:* content fixtures produce expected issue lists; matcher pairs render-equivalent DOM changes (Section 13.2) with no false missing/added.

**M4 — Computed-style diff + anchor locator (G1, G4).** Scoped computed-style capture, anchor-set locators (Section 5), `style_changed` / `background_gradient_lost` with `remediation` carrying grep targets. *DoD:* gradient-removal fixture yields `background_gradient_lost` with from/to; a CSS color/spacing change yields `style_changed` with property-level from/to and a greppable anchor (no component name claimed).

**M5 — Sequence diff (G3).** Section 8 on matched pairs. *DoD:* a swapped-sections fixture yields a single `component_swapped` (not missing+added).

**M6 — Capability probes + a11y + network.** Section 11 probes, axe diff, network/console. *DoD:* broken-mobile-menu and missing-accordion-behavior fixtures detected; new 404 asset and new console error reported.

**M7 — Reporters + parity profiles.** HTML (side-by-side, region jump, filters, fix-ordered list) and Markdown renderers; profiles wired to severities; CI exit codes + `--fail-on`. *DoD:* one run renders HTML/Markdown/JSON; profile switch changes pass/fail as specified.

**M8 (optional) — Agentic explanation layer.** Consume `DiffResult` (never raw screenshots alone), cluster + explain + propose fixes, citing artifacts; fully disableable; deterministic layer remains authoritative.

---

## 13. Test strategy & fixtures

Local HTML fixture pairs served from `/fixtures` (no network). Each maps to goals and expected issues; golden `DiffResult` JSON committed and diffed in CI.

### 13.1 Fixture matrix
1. Identical pages → `status: pass`, zero issues.
2. Trailing slash mismatch (G5) → `url_trailing_slash`.
3. `es_MX` underscore (G6) → `locale_separator_invalid`.
4. `es-mx` lowercase region (G6) → `locale_case_invalid`.
5. Missing newsletter form (G2) → `missing_form` (critical).
6. Missing paragraph / changed H1 (G2) → `missing_text` / `changed_h1`.
7. Broken PDF link (G2) → `broken_link`.
8. **Gradient removed** (G4) → `background_gradient_lost` with from/to.
9. **CSS spacing/color change** (G1) → `style_changed` with property delta + component hint.
10. **Two sections swapped** (G3) → single `component_swapped`.
11. Render-equivalent DOM change (Section 13.2) → no issue (or info under `strict-visual`).
12. Broken mobile menu → `nonfunctional_capability`.
13. Missing accordion behavior → `nonfunctional_capability`.
14. New 404 asset → `network_error`; new console error → `console_error`.
15. Page-height mismatch + masked timestamp → height delta reported; masked region produces no diff.

### 13.2 Render-equivalent DOM change (must NOT flag)
Old `<a class="btn primary" href="/donate">Donate</a>` vs
New `<div class="cta"><a href="/donate" role="button">Donate</a></div>`
→ matcher pairs them (same role/href/text/position); no `missing`/`added`; at most an `info` implementation note when implementation comparison is explicitly enabled.

### 13.3 Unit / integration / golden
- **Unit (Rust):** text normalization, link/URL normalization, locale parser, color normalization, gradient parser, similarity scoring, matching/assignment, sequence/LCS, severity + scoring, content-addressed issue IDs, schema (de)serialization.
- **Integration (TS):** Playwright load, stabilization determinism, screenshot capture, page-model + computed-style extraction, `CaptureBundle` schema validity.
- **Golden:** fixtures → `DiffResult` compared to committed goldens (with float tolerances on scores).

---

## 14. CLI & config

```bash
ppd --old https://old.example.com/about \
    --new https://new.example.com/about \
    --out ./report \
    --profile content-structure \
    --viewport desktop=1440x1000 --viewport mobile=390x844 \
    --hide ".chat-widget,.cookie-banner" --mask ".timestamp" \
    --trailing never \
    --fail-on error \
    --json --html --markdown
```

Exit codes: `0` pass; `1` failed configured threshold (`--fail-on`); `2` tool/runtime error (page load failure, browser crash, schema violation).

Config file mirrors flags and adds `matching` (`matchFloor`, `noMatchCeil`, per-kind weight overrides), `stabilization`, `thresholds`, `interactions`, `redact`, `concurrency`. All page content stays local; never transmit screenshots/DOM/content to third parties unless an AI layer is explicitly enabled and configured. Redact cookies/auth headers/secrets from logs and artifacts.

---

## 15. Non-negotiable invariants (checklist for the agent)

- [ ] `DiffResult` validates against `/contract/diff-result.schema.json`; TS and Rust types both derive from it; drift fails CI.
- [ ] Every issue has a `locator` with an agent-facing **anchor set**, and where actionable a structured `remediation` with grep targets. The tool never names a source component.
- [ ] Issue IDs are content-addressed and stable across runs given the same inputs.
- [ ] **Analysis is byte-deterministic:** no map-iteration-order dependence, total-ordered tie-breaks, fixed-order float reductions; identical bundles → identical `DiffResult` (modulo timestamps). Verified by golden tests.
- [ ] Matching uses **confidence bands** (`matchFloor`/`noMatchCeil`), not a single hard cutoff; per-signal sub-scores are written to `evidence.match`.
- [ ] Capture determinism is environment-scoped: Chromium + fonts pinned in Docker; visual baselines are not compared across environments. Uncontrollable page nondeterminism is masked or flagged low-confidence, never silently varied.
- [ ] Each of G1–G6 has at least one passing fixture before v1 is "done."
- [ ] No production form submission / auth action without explicit opt-in flags.
- [ ] AI layer (if present) cites deterministic artifacts and is disableable; deterministic evidence is authoritative.
