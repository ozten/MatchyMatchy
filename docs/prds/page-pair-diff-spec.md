# Page Pair Diff — Agent-First Specification (v3)

> **Audience:** an agentic coding tool implementing this project.
> **Status:** authoritative build spec. Where this document conflicts with any prior draft, this document wins.
> **One-line mission:** Given URL A (old) and URL B (new), produce a deterministic, machine-actionable diff that lets an agent fix migration defects with minimal additional investigation.
> **v3 changelog:** CLI renamed to `matchy`; Rust analysis core from day 1 (TS fallback removed); no Docker — curl-to-install with documented runtime requirements; no AI/LLM layer anywhere; capability probes, auth, and `locale_parity_missing` deferred to post-v1; added stable issue IDs, `--baseline` accept-list, deterministic clustering, identity-first matching, egress policy, and a real-pair calibration milestone.

---

## 0. How to read this spec (for the implementing agent)

- The **primary deliverable is the `DiffResult` JSON contract** (Section 7), not the HTML report. The HTML report is one (static) renderer of that contract.
- Build in the **milestone order** in Section 12. Each milestone has acceptance fixtures. Do not advance until fixtures pass.
- This tool is **pure deterministic code**. It contains no LLM or AI integration of any kind and no logic belonging to the agent harness that consumes it. Its job is to surface maximally actionable structured information; what consumes that information is out of scope.
- The tool **never names the source component.** It emits a greppable **anchor set** (Section 5) that maps to repo source; the downstream agent — which has repo access — resolves the component. Do not fabricate component identity.

---

## 1. Goal traceability (these drive priority)

Every feature exists to serve a concrete user goal. The implementing agent must keep this table satisfied; a feature that does not trace to a goal is out of scope for v1.

| # | User goal | Primary subsystem(s) | Milestone |
|---|---|---|---|
| G1 | Which component needs CSS tweaks | Computed-Style Diff + Anchor Locator | M4 |
| G2 | Which content is missing (headings, text, CTAs, forms, images) | Semantic Diff (text/link/image/form) | M3 |
| G3 | Which component order is wrong / swapped | Sequence Diff | M5 |
| G4 | Visually different + missing background gradient | Visual Diff + Computed-Style Diff (background/gradient) | M1/M4 |
| G5 | URL has a trailing slash it shouldn't | URL & Redirect Hygiene | M2 |
| G6 | Locale path wrong case (`es-mx` vs `es_MX`) | Locale Hygiene | M2 |
| G7 | Broken links, images, and assets | Semantic Diff + Network Diff | M3 |
| G8 | Accessibility regressions | A11y Diff | M7 |

---

## 2. Scope

### In scope (v1)
- Single explicit URL pair: `--old <url> --new <url>`.
- Real-browser rendering via Playwright (Chromium), full-page + viewport screenshots.
- Deterministic page stabilization.
- Semantic page-model extraction (rendered, visible content).
- Computed-style extraction for a deterministic candidate set.
- Multi-signal, identity-first element matching across differing DOMs.
- Visual diff, content diff, **computed-style diff**, **sequence/order diff**, link/image/form diff, a11y diff, network/console diff.
- **URL/redirect/canonical hygiene** and **locale-path hygiene** (single-pair checks only).
- Stable JSON `DiffResult` (the contract), plus **static** HTML and Markdown renderers.
- **Migration-loop support:** content-stable issue IDs, `--baseline` accept-list, deterministic issue clustering.
- Parity profiles (Section 9).

### Out of scope — post-v1 (deferred, not deleted)
- **Authentication** for protected targets (basic auth, header/cookie injection, Playwright `storageState`). v1 assumes both URLs are publicly reachable.
- **Capability detection & probes** (nav/menu/search/accordion/tab/carousel detection, interactive click-probes) and the `capability-only` parity profile.
- `locale_parity_missing` (requires knowledge of URLs beyond the single pair).
- Interactive HTML report features (filters, region-jump navigation, re-sorting). v1 ships a static HTML page.
- Docker / pinned-environment images of any kind.
- Any AI/LLM explanation layer. **This is permanently out of scope for this tool**, not merely deferred: the tool is deterministic code end to end.
- A fix-locus (repo vs CMS) classification field.
- Full-site crawling, sitemap discovery, batch CSV mode.
- SEO suite parity (Screaming Frog replacement).
- Proving business-logic / transactional correctness.
- Submitting production forms or performing auth actions.
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
            │  • computed-style diff      │   clustering, report rendering
            │  • url/locale hygiene       │
            │  • scoring + issue gen      │
            │  • DiffResult + renderers   │
            └───────────────┬─────────────┘
                            ▼
              DiffResult JSON  ──▶  static HTML / Markdown / agent consumer
```

**Rationale.** The browser layer genuinely wants Playwright (auto-waiting, `axe-core/playwright`, stabilization maturity); in-page extraction needs injected JS regardless of host language. The analysis layer is pure, deterministic, correctness-critical work suited to Rust. The JSON seam is also the agent-first artifact boundary and makes each half independently testable.

**The analysis core is Rust from M1.** There is no TypeScript fallback for the analyze layer; the contract is the seam between the two languages, not an excuse to defer the Rust implementation.

**Orchestration across the seam.** `matchy` (the Rust binary) is the sole entry point and the only command a user runs:

1. `matchy` resolves the bundled capture script (`capture.cjs`, shipped alongside the binary).
2. For each page/viewport it spawns `node <path-to>/capture.cjs`, writing a `CaptureConfig` JSON to the child's stdin (URL, viewport, stabilization config, hide/mask selectors, output dir) and reading the resulting `CaptureBundle` path from stdout.
3. Capture failures (non-zero child exit, malformed bundle, schema violation) map to `matchy` exit code `2` with a structured error on stderr.
4. `matchy doctor` verifies the runtime environment — Node version, Playwright version, Chromium availability — and prints concrete remediation steps for anything missing.

All CLI flags that affect capture cross the seam inside `CaptureConfig`; the capture script takes no flags of its own.

### 3.2 Workspace layout

```
matchy/
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
      package.json          # playwright pinned exactly (see §14 runtime requirements)
      esbuild.config.mjs    # bundles to a single capture.cjs shipped with the binary
    analyze/                # Rust — the core
      src/
        bin/matchy.rs       # CLI (clap); spawns capture, orchestrates end-to-end
        contract.rs         # serde structs mirroring the contract
        visual_diff.rs
        matching.rs
        sequence_diff.rs
        style_diff.rs
        hygiene_url.rs
        hygiene_locale.rs
        semantic_diff.rs
        clustering.rs
        scoring.rs
        issue.rs
        baseline.rs         # --baseline accept-list
        report/{html.rs, markdown.rs, json.rs}
      Cargo.toml
  contract/                 # SINGLE SOURCE OF TRUTH for JSON schema
    capture-bundle.schema.json
    diff-result.schema.json
  fixtures/                 # local HTML pairs (Section 13)
  install.sh                # curl-to-install: delivers matchy binary + capture.cjs
  README.md
```

The contract lives in `/contract` as JSON Schema. CI enforces **validation-only conformance**: serialized output from the Rust `serde` structs and the TS `zod` schemas is validated against the JSON Schema over shared fixtures; a mismatch fails CI. Code generation from the schema is optional developer tooling, not build infrastructure.

### 3.3 Determinism vs. stability (cross-cutting — read before implementing either layer)

These are two different properties and must not be conflated.

**Determinism** = same input → same output, exactly. The **analyze layer is deterministic by construction**: it is a pure function `(CaptureBundle_old, CaptureBundle_new) → DiffResult`. The only ways to break this are engineering defects, all of which the implementation MUST avoid:
- Iteration order: never iterate `std::collections::HashMap`/JS `Object` key order for anything that affects output. Use `BTreeMap`, or collect-and-sort by a stable key.
- Tie-breaks: the assignment step (Section 6) and any sort MUST define a **total order**; break ties on a stable key (node `id`), never on insertion order or address.
- Float reductions: sum/aggregate scores in a fixed, sorted order so results don't depend on accumulation order across threads.
- Concurrency: parallelism is allowed but results MUST be reassembled in a deterministic order before scoring/serialization.

Given the above, identical bundles MUST produce a byte-identical `DiffResult` (modulo the `runId`/timestamp fields, which are excluded from golden comparisons).

**The capture layer is deterministic only within bounds it does not fully control**, and the spec is honest about the three leaks:
1. **Rendering environment.** Subpixel rasterization, font hinting, and image decoding vary by OS / GPU / Chromium version. There is no pinned container image: pixel-level visual baselines are therefore **machine-scoped** — valid only on the machine + Chromium build that produced them, with cross-machine comparison unsupported. Capture records an **environment fingerprint** (OS, Chromium build, Playwright version, deviceScaleFactor) in every `CaptureBundle`; analyze emits a warning when comparing bundles with mismatched fingerprints and downgrades pixel-diff confidence accordingly. The semantic, style, structure, and hygiene diffs are the authoritative cross-environment signals; the pixel diff is corroborating evidence.
2. **Page nondeterminism.** Time, randomness, animation, lazy-load races, A/B tests, ads. The Section 4.2 steps suppress the controllable sources (freeze time/RNG/rAF, wait for fonts/images, kill animation, mask volatile regions), but an adversarial live page cannot be driven to 100%. Residual nondeterminism MUST be **masked or surfaced as low-confidence**, never allowed to silently vary the result.
3. **CMS content drift.** The old production CMS keeps receiving edits (new posts, price changes, updated listings) while the migration proceeds — diffs caused by drift are not migration defects and nothing in the new site's source can fix them. Mitigations: capture old and new as close together in time as possible (the tool captures them in the same run); route known-volatile regions to `maskSelectors`; route legitimate, recognized drift to the `--baseline` accept-list (Section 7.4).

**Responsibility boundary, stated mechanically.** Capture records exactly which determinism steps ran, succeeded, failed, or were skipped in `CaptureBundle.determinism`. Analyze treats the bundle as ground truth and MUST mechanically lower the `confidence` of any issue whose evidence overlaps a region or signal that the `determinism` block marks as failed or masked. Capture never decides what is an issue; analyze never re-fetches or re-renders.

Design target, stated plainly: *analysis is fully deterministic; capture is deterministic within a single machine's pinned-by-documentation environment and to the extent the page cooperates, with the uncontrollable residue masked or flagged.*

**Stability is a separate concern.** A hard threshold (e.g. "match if score ≥ 0.7") is perfectly deterministic yet brittle: a 0.699 vs 0.701 score flips an issue on/off, so tiny capture noise changes output. Solve this with **confidence bands / hysteresis**, not by weakening determinism: classify `>= matchFloor` as matched, `< noMatchCeil` as unmatched, and the band between as `uncertain` (emitted with low confidence for agent/human review rather than a coin-flip verdict). Thresholds live in config so a run's verdicts are auditable and reproducible.

---

## 4. Capture layer (TypeScript) — `CaptureBundle`

### 4.1 Browser runner
- Chromium via Playwright. WebKit/Firefox post-v1.
- Per-run isolated `BrowserContext`. Old and new use **identical** viewport, deviceScaleFactor, locale, timezone, user agent, and color scheme.
- Configurable: viewports (default `desktop 1440x1000`, `mobile 390x844`), timeout, retries.
- Never launch Chromium with `--no-sandbox`; run as an unprivileged user.

### 4.2 Determinism guarantees (hard requirements, each with an escape hatch)
Before any capture, inject:
- Animation/transition kill CSS (`animation: none`, `transition: none`, `scroll-behavior: auto`, `caret-color: transparent`).
- `prefers-reduced-motion: reduce` emulation.
- Freeze nondeterminism in page context using **Playwright's `page.clock` API** (install a fixed time, controlled ticking) and an init script that seeds/stubs `Math.random`. rAF is driven by the controlled clock rather than ad-hoc neutralization.
- Wait for: load → networkidle (configurable) → `document.fonts.ready` → all in-viewport `<img>` `decode()`.
- Lazy-load pass: scroll to bottom in steps, then back to top; re-wait for fonts/images.
- Apply configured `hideSelectors` (visibility:hidden) and `maskSelectors` (neutral fill, preserves layout).
- Apply configured `clickBeforeCapture` (e.g. cookie accept), each optional. These are user-specified explicit selectors; the tool never chooses click targets itself in v1.

Each stub has a per-flag escape hatch (`--no-freeze-time`, `--no-stub-random`) because frozen clocks are a known footgun (time-polling lazy-loaders and carousels can spin or stall). If the `networkidle` wait times out with time frozen, capture retries that page **once** without time freezing and flags the bundle low-confidence.

A capture is only valid if the determinism steps completed (or were explicitly skipped); record the status of **every step** — ran / failed / skipped — in `CaptureBundle.determinism`.

### 4.3 Page model extraction (in-browser, post-stabilization)
Extract **rendered, visible** content. Visible = non-empty bbox, not `display:none`/`visibility:hidden`, not fully transparent, within page bounds. Capture screen-reader-only labels separately for a11y.

All extracted page-derived strings are **length-capped** (default 500 chars, configurable) and **control-character-stripped** at extraction time (see Section 7.5 — untrusted data).

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
  "network": { "requests": [ /* {url,status,type,failed} — redacted per §14 */ ] },
  "console": [ /* {level,text} */ ],
  "a11y": { "violations": [ /* axe results */ ] }
}
```

### 4.4 Computed-style capture (scoped — performance critical)
Do **not** read `getComputedStyle` for every element on the page. The candidate set is **deterministic and fixed at capture time** (the matcher runs later, in analyze, on the other side of a one-way seam — it cannot request more styles after the fact):

> **Candidates = every `SemanticNode`, plus each node's ancestor chain up to (and including) its nearest landmark element**, deduplicated, subject to a configurable budget (default 2,000 elements; on overflow, drop deepest-ancestor entries first and record the truncation in the bundle).

Including the ancestor chain is what brings **container** properties (`flex-direction`, `justify-content`, `gap`, `grid-template-columns`) into the bundle, so layout breakage (G1) is detectable — those properties rarely live on leaf nodes.

Capture a curated, normalized property set per candidate:

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
  "environment": {                      // fingerprint — see §3.3
    "os": "linux", "chromiumBuild": "1223", "playwright": "1.60.0", "dsf": 1
  },
  "determinism": {
    "animationsDisabled": "ran", "timeFrozen": "ran", "randomStubbed": "ran",
    "fontsReady": "ran", "lazyLoadPass": "ran", "clicked": ["button.accept"]
    // every step: "ran" | "failed" | "skipped"
  },
  "page": { /* page model from 4.3 */ },
  "computedStyles": { "node_42": { /* curated props */ } },
  "screenshots": { "fullPage": "desktop/old.png", "viewport": "desktop/old-vp.png" }
}
```
The analyze layer receives **two** bundles (old, new) per viewport.

---

## 5. Anchor Locator (serves G1, G3 — replaces source-component naming)

The tool does **not** try to recover a React/source component name, and does **not** require the team to emit `data-component`. It runs against the vanilla staging preview as-is. Instead, every node carries an **anchor set**: a small fingerprint of *semantic* facts — visible text, hrefs, alt text — that identify the element precisely and are greppable when they appear in repo source.

**Why not XPath / structural selectors as the agent-facing identity.** Absolute positional XPath (`/html/body/div[2]/section[1]/a[1]`) is brittle (any DOM change invalidates it) and, more importantly, encodes *DOM position* — but the agent's task is finding the *source of truth*, not traversing the DOM. In a Next.js/Sanity build, classNames are typically CSS-module hashes or Tailwind utility soup, so class selectors generally don't grep to a component file either. String literals — visible text, `href`, `alt`, `aria-label` — are the most distinctive facts available.

**Honest caveat about the grep targets:** in a CMS-backed site, visible text/hrefs/alt may live in CMS documents rather than repo source. A grep for them may hit JSX directly (template-owned strings) **or may dead-end in the repo** because the string lives in the CMS dataset or a migration script. Either way the anchor set still pins down *which element* is wrong; where the fix lives is the consuming agent's determination, not this tool's.

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

**Token economics (the point of the change):** the agent receives a ~6-field fingerprint plus an implicit "grep for the text or href," not a 200-character path. Smaller *and* more actionable. The tradeoff accepted by dropping `data-component`: the tool cannot *name* the component — and it doesn't need to. It hands over a precise fingerprint; the agent resolves identity from source (or from the CMS dataset).

**Anchor strength (drives `localityBonus`, Section 7.2):**
- **high** (`localityBonus = 1.0`): a distinctive `href`, distinctive `text`, `alt`, or `ariaLabel`
- **medium** (`localityBonus = 0.7`): only `nearestHeading + landmark + ordinalInLandmark`
- **low** (`localityBonus = 0.4`): nothing distinctive (e.g. a bare decorative element) — the issue is marked harder-to-locate rather than given a fake identity

---

## 6. Element matching (the crux) — identity-first, two-stage

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

Matching is **identity-first**: what an element *is* (its text, target, alt, role) determines pairing; where it *sits* is only a tiebreaker. This is a hard requirement, because the sequence differ (Section 8) depends on reordered components still being **paired** — a component moved from page top to page bottom must match its counterpart. Position must never be able to veto a strong identity match.

1. **Block by kind** (links match links, images match images, etc.; `generic` is a fallback bucket).

2. **Stage 1 — identity scoring.** For each candidate pair within a block, compute an identity score from identity signals only — a fixed-constant linear combination, each sub-score normalized to `[0,1]`:

   | Signal | Definition |
   |---|---|
   | `textSim` | token + edit-distance similarity of normalized visible text |
   | `accNameSim` | same, over accessible name |
   | `hrefSim` | `1` if normalized hrefs equal; partial if same path, differing query; else `0` |
   | `altSim` | similarity of image `alt`; plus intrinsic-dimension ratio |
   | `roleSim` | `1` if role/kind equal, else `0` |
   | `nearbySim` | similarity of nearby text / enclosing landmark |

   **Default per-kind identity weight tables** (config-overridable; the *identity* signal dominates each):

   ```
   link/button:  href 0.55  text 0.35  accName 0.10
   image:        alt  0.55  intrinsicDim 0.45
   heading:      text 1.00
   text block:   text 0.85  nearby 0.15
   form/field:   accName 0.55  role 0.30  nearby 0.15
   generic:      text 0.70  role 0.30
   ```

   A pair whose identity score ≥ `identityFloor` (default 0.85) **and** is the unique mutual best within its block (by a `tieMargin`, default 0.05) is **matched immediately, regardless of position**.

3. **Stage 2 — ambiguity resolution.** Remaining candidates (identity ties within `tieMargin` — e.g. three identical "Read more" links — or identity scores below `identityFloor`) are resolved by **constrained assignment** (Hungarian for small blocks; greedy with a similarity floor for large blocks) over a combined score:

   ```
   score(a,b) = 0.7 · identity(a,b) + 0.3 · tiebreak(a,b)
   tiebreak   = pos 0.5 · size 0.3 · nearby 0.2     (posSim = 1 − min(1, |y_old_norm − y_new_norm|); sizeSim = bbox area ratio min/max)
   ```

   The assignment forbids pairs below `noMatchCeil` (so the uncertain band remains reachable) and prefers monotonic order. Ties broken on stable node `id` (see §3.3 determinism).

4. **Classify** using confidence bands, not a single hard cutoff (see §3.3 stability):
   - `score ≥ matchFloor` → `matched`
   - `noMatchCeil ≤ score < matchFloor` → `uncertain` (emitted low-confidence for review, not silently decided)
   - matched but `|seqIndexNew − seqIndexOld|` exceeds a threshold → candidate for **sequence diff** (Section 8)
   - old node below `noMatchCeil` against all candidates → `missing`
   - new node below `noMatchCeil` against all candidates → `added`
   - matched with differing attributes (href, text, alt, computed style) → emit the corresponding attribute-level issue

All weights are **plain constants, not learned and not stochastic** — they encode "what identifies *this kind* of element." Defaults are frozen only after the M6 real-pair calibration milestone (Section 12).

**Auditability requirement:** the per-signal sub-scores, the stage that decided the pairing (`identity` | `assignment`), and the final `score` for each emitted match/non-match MUST be written into the issue evidence (`evidence.match`). This is what makes the tool forensic rather than magic — when it pairs (or refuses to pair) two elements, the reason is inspectable, and because the weights are constants the result is fully reproducible.

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
    "fixableNow": 6,                       // issues with severity ≥ warning AND anchor strength ≥ medium AND structured remediation
    "byType": { "missing_form": 1, "style_changed": 3, "component_reordered": 1, "url_trailing_slash": 1 },
    "clusterCount": 2,
    "topFixes": ["cluster_001","issue_004","issue_002"]  // first N (default 5) by fix value; may reference clusters
  },
  "scores": { "visual":0.88,"content":0.94,"structure":0.80,"style":0.72,"accessibility":0.92,"technical":1.0,"hygiene":0.5 },
  "viewports": [ { "name":"desktop","status":"fail","issues":["issue_001"] } ],
  "issues": [ /* Issue[] */ ],
  "clusters": [ /* Cluster[] — see 7.4 */ ],
  "suppressed": { "count": 3, "ids": ["issue_0a1","issue_b42","issue_c77"] },   // baseline-accepted (see 7.4)
  "determinism": { "old": {/* … */}, "new": {/* … */} },
  "artifacts": { "old":"desktop/old.png","new":"desktop/new.png","diff":"desktop/diff.png" }
}
```

### 7.1 `Issue` (every field exists to reduce agent work)
```jsonc
{
  "id": "issue_004",                       // CONTENT-ADDRESSED — see "Issue identity" below
  "type": "style_changed",
  "category": "style",                     // visual|content|structure|style|accessibility|technical|hygiene
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
    "match": { "stage": "identity", "score": 0.91, "signals": { "href": 1.0, "text": 0.86 } },
    "artifacts": { "oldCrop":"desktop/issues/issue_004_old.png", "newCrop":"desktop/issues/issue_004_new.png" }
  },
  "remediation": {                         // structured fix description — deterministic, no inference
    "action": "restore_css_property",
    "findBy": { "grep": ["\"/signup\"", "Get started"], "near": "Build faster" },
    "property": "background-image",
    "from": "none",
    "to": "linear-gradient(90deg, #6d28d9 0%, #2563eb 100%)",
    "note": "New page replaced the gradient with a flat fill. The grep targets may hit repo source or may live in CMS content; the anchors identify the element either way. The tool does not name the component."
  }
}
```

**Issue identity (content-addressed, re-capture-stable).** The `id` is a hash over **exactly** these inputs:

```
hash( type
    + viewport
    + anchors{ text, role, href, alt, ariaLabel, nearestHeading, landmark, ordinalInLandmark }
    + styleProperty )            // the CSS property name, style-category issues only
```

Explicitly **excluded** from the hash: bboxes, CSS selectors, match scores, artifact paths, timestamps, and any other field that jitters between re-captures of a live page. This is what makes the migration loop work: fix `issue_004`, re-run against the live pages, and `issue_004` is verifiably gone while every still-unfixed issue keeps its ID. The `--baseline` accept-list (7.4) depends on this property.

### 7.2 Issue ordering (fix value)
`issues` array is sorted by descending **fix value** = `severityWeight × confidence × localityBonus`, where `localityBonus` is the numeric anchor-strength value from Section 5 (**high = 1.0, medium = 0.7, low = 0.4**) — strong/greppable anchors are cheap to find in source, so they sort above diffuse visual regions with weak anchors. The HTML report may re-sort; the JSON order is the agent's recommended work queue.

### 7.3 Issue taxonomy (stable strings)
```
# visual
visual_region_changed  page_height_changed
# content (G2, G7)
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
# accessibility (G8)
accessibility_regression accessibility_improved
# technical
status_code_mismatch network_error console_error load_error
# hygiene (G5, G6)
url_trailing_slash url_redirect_chain url_protocol_downgrade canonical_mismatch
locale_case_invalid locale_separator_invalid locale_unknown
```

Reserved for post-v1 (do not emit in v1): `missing_capability`, `nonfunctional_capability`, `changed_capability`, `capability_added`, `locale_parity_missing`.

`load_error` = a page failed to produce a usable render at all (browser-level navigation/timeout failure) where the other page succeeded; both pages failing is exit code 2, not a diff.

### 7.4 Migration-loop support: clusters and the baseline accept-list

**Deterministic clustering.** Real defects are often systematic: one global stylesheet or shared-template bug emits hundreds of per-element issues. Analyze groups issues into `clusters` by **fixed rules** (no inference): issues sharing the same `type` **and** the same changed style property, or the same `type` **and** the same landmark, are clustered when the group size ≥ `clusterMin` (default 3, config). Each cluster:

```jsonc
{
  "id": "cluster_001",                  // hash of (type + sharedKey)
  "issueIds": ["issue_004","issue_009","issue_011"],
  "sharedProperty": "font-family",      // or "sharedLandmark": "main"
  "summary": "23 style_changed issues share font-family change in main"
}
```

`agentSummary.clusterCount` counts clusters; `topFixes` may list a cluster ID where fixing the shared root cause clears the whole group. One global defect = one work item, not hundreds.

**Baseline accept-list.** A migration intentionally changes things. `--baseline accepted.json` supplies an array of `{id, note?}` entries keyed on the stable issue IDs of 7.1. Matching issues are **suppressed** from `issues` (and from scoring/status) but counted in `suppressed: {count, ids}` for audit. The intended loop: run → triage → add intentional diffs to the baseline → fix real defects → re-run; the queue shrinks monotonically to zero.

### 7.5 Page-derived strings are untrusted data

Every string in `DiffResult` that originated in page content — visible text, alt/aria values, hrefs, console messages, network URLs — is **untrusted input** from a source the operator does not fully control. Capture length-caps (default 500 chars) and control-character-strips them (Section 4.3); renderers escape them (Section 15). Consumers MUST treat them as data, never as directives. This tool performs no interpretation of them beyond the deterministic comparisons specified here.

---

## 8. Sequence / order diff (serves G3)

Inputs: matched pairs from Section 6 plus their `seqIndex` on each page. (The identity-first matcher guarantees that moved components are still *paired* — position can lower a tiebreak score but never veto a strong identity match — so reorders arrive here as matches, not as missing+added.)

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

| Profile | Visual diff | Style diff | Content | Structure | Hygiene | A11y |
|---|---|---|---|---|---|---|
| `strict-visual` | fail | fail | fail | fail | fail | warn |
| `content-structure` (default) | info | warn | fail | fail | fail | warn |

Profiles set default severities per category; explicit per-type severity config overrides them. (The `capability-only` profile is deferred along with the capability differ.)

---

## 10. URL & locale hygiene (serves G5, G6)

### 10.1 URL/redirect/canonical (G5)
Run on the two input URLs and on extracted **same-site** links (see egress policy, 10.3):
- **Trailing slash** vs configured policy (`"trailing": "never" | "always" | "preserve"`). Mismatch → `url_trailing_slash`, remediation = rewrite to policy.
- **Redirect chain**: follow; if `redirectChain.length > 1` or a redirect occurs where none should, emit `url_redirect_chain` with the full chain.
- **Protocol**: any `http://` where the sibling is `https://` → `url_protocol_downgrade`.
- **Canonical**: `<link rel=canonical>` not matching `finalUrl` (mod policy) → `canonical_mismatch`.
- **Status parity**: old responds 2xx but new responds non-2xx (a rendered 404/500 page) → `status_code_mismatch` (severity **critical**). This **short-circuits** content/style/sequence diffing for that pair — the run still emits a valid `DiffResult` containing the one decisive issue rather than noisily diffing real content against an error page. (Both pages failing to load at all is a tool error: exit code 2.)

### 10.2 Locale path (G6)
Detect a locale segment in the path (first or second segment). Validate against BCP-47:
- Region subtag must be **uppercase**, language **lowercase**: `es-MX` valid, `es-mx`/`ES-mx` → `locale_case_invalid`.
- Separator must be hyphen, not underscore: `es_MX` → `locale_separator_invalid`.
- Unknown language/region codes → `locale_unknown`.

Remediation for locale issues carries the corrected segment (`from: "/es_MX/about"`, `to: "/es-MX/about"`).

(`locale_parity_missing` — checking that locale variants exposed by the old site exist on the new — requires URLs beyond the single pair and is post-v1.)

### 10.3 Egress policy (applies to all probes)

The tool issues network requests to URLs it did not receive from the operator (extracted links, redirect targets, asset HEAD checks). Page content is attacker-influenceable, so all probing obeys a hard egress policy:

- **Schemes:** `http`/`https` only. `file:`, `ftp:`, custom schemes are never fetched.
- **Scope:** probes (broken-link HEAD checks, redirect following) target the **same registrable domain** as the old or new input URL by default. `--allow-external-probes` widens this to third-party hosts.
- **Address blocking:** resolved private, link-local, loopback, and cloud-metadata ranges (`10/8`, `172.16/12`, `192.168/16`, `169.254/16`, `127/8`, `::1`, `fd00::/8`) are refused by default — unless the input URLs themselves resolve there (local fixture serving stays possible).
- Per-link policy checks (trailing slash, redirect chains) apply to **same-site links only**; external links get at most an opt-in liveness check.
- Probe concurrency is capped (config `concurrency`) to stay polite.

---

## 11. Visual, semantic, a11y, network diffs

- **Visual diff:** full-page + viewport. Pixel comparison (Rust `image` crate; pixelmatch-style algorithm or `dssim` for perceptual). When page heights differ, comparison runs over the **common height only**; the remainder is reported solely via `page_height_changed` and never counted as changed pixels (naive padding would mark everything below the shorter page as different). Output: diff image, changed-pixel %, **region clustering** (bounding boxes) over the common area, page-height delta. Emission rule: **one `visual_region_changed` issue per clustered region**, emitted only when region area ≥ `minRegionArea` (default 2,500 px², config) and the page-level changed-pixel ratio ≥ `visualThreshold` (default 0.5%, config). Regions are linked to overlapping `SemanticNode`s so a region can name what changed. Under non-`strict-visual` profiles a pure region change is `info` *unless* it overlaps a matched node with a content/style issue, which raises severity. Remember (§3.3): pixel results are machine-scoped corroborating evidence, downgraded on environment-fingerprint mismatch.
- **Semantic diff (G2, G7):** title/meta/canonical/lang, headings + hierarchy, text blocks (grouped, normalized — see normalization rules), links, images (+alt, dimensions, load status), forms (+fields, labels, required, submit). Weight **main-content** over repeated chrome (header/footer nav classified separately).
- **A11y diff (G8):** `axe-core/playwright` on both; diff violation sets → `accessibility_regression` (new) / `accessibility_improved` (fixed); also changed accessible names on important controls, missing landmarks/labels, heading-hierarchy regressions.
- **Network/console:** failed requests, 4xx/5xx assets, CORS, mixed content, uncaught exceptions. New-only failures are issues; failures on both are noted but not scored against the new page.

(The **capability diff** — detection of nav/menu/search/accordion/tab/carousel and interactive click-probes — is deferred to post-v1 in its entirety. When it returns it must run in the capture layer *after* screenshots and page-model extraction so probe interactions cannot pollute captured state, transport its results in a dedicated `CaptureBundle.capabilities` field, and ship written probe-safety criteria.)

**Text normalization:** collapse whitespace, trim, NBSP→space, optional smart-quote/punctuation/case folding. Never normalize away dates, names, prices, phone numbers, emails, legal text, product claims, CTA wording.

---

## 12. Milestones (build order — reordered to deliver user goals early)

**M1 — Capture + visual diff skeleton.** TS capture produces `CaptureBundle` (screenshots + stabilization + environment fingerprint). Analyze (Rust) produces `old.png/new.png/diff.png`, page-height delta, and a `DiffResult` with `visual_region_changed`. `matchy` orchestrates end-to-end; `matchy doctor` verifies the environment. *DoD:* two URLs → artifacts + valid `DiffResult` JSON validated against schema; environment fingerprint recorded in bundles; runtime requirements documented in the README; `matchy doctor` reports this machine healthy.

**M2 — URL & locale hygiene (G5, G6).** Implement Section 10 fully, including `status_code_mismatch` short-circuit and the 10.3 egress policy. Cheap, high-signal, validates the contract end-to-end. *DoD:* fixtures for trailing slash, redirect chain, `es_MX`, `es-mx`, and old-200/new-404 produce the correct hygiene issues with correct remediation.

**M3 — Semantic extraction + content diff (G2, G7).** Node stream, identity-first matching (Section 6), content/link/image/form issues. *DoD:* content fixtures produce expected issue lists; matcher pairs render-equivalent DOM changes (Section 13.2) with no false missing/added.

**M4 — Computed-style diff + anchor locator (G1, G4).** Deterministic candidate-set computed-style capture (including ancestor chains), anchor-set locators (Section 5), `style_changed` / `background_gradient_lost` with `remediation` carrying grep targets. *DoD:* gradient-removal fixture yields `background_gradient_lost` with from/to; a CSS color/spacing change yields `style_changed` with property-level from/to and a greppable anchor (no component name claimed); a container `flex-direction`/`gap` change is detected.

**M5 — Sequence diff (G3).** Section 8 on matched pairs. *DoD:* a swapped-sections fixture yields a single `component_swapped` (not missing+added), including when the swapped sections are far apart vertically.

**M6 — Real-pair calibration (gate).** Run the tool against **at least one real old/new page pair from the actual migration**. Tune matcher weights, `identityFloor`, `matchFloor`/`noMatchCeil`, and visual thresholds against observed false positives/negatives; record the findings and chosen values in the repo. **Default weights and thresholds are frozen only after this gate.** *DoD:* a written calibration note in the repo; the real pair produces a triaged `DiffResult` the team agrees reflects reality (no unexplained missing/added floods).

**M7 — A11y + network diffs (G8).** axe diff, network/console issue emission. *DoD:* new 404 asset and new console error reported; seeded a11y regression detected.

**M8 — Reporters, profiles, migration loop.** Static HTML (side-by-side screenshots + fix-ordered issue list — no interactive features) and Markdown renderers; parity profiles wired to severities; `--baseline` accept-list; deterministic clustering; CI exit codes + `--fail-on`. *DoD:* one run renders static HTML/Markdown/JSON; profile switch changes pass/fail as specified; a baselined issue is suppressed and counted; a seeded global-style defect produces one cluster.

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
7. Broken PDF link (G7) → `broken_link`.
8. **Gradient removed** (G4) → `background_gradient_lost` with from/to.
9. **CSS spacing/color change** (G1) → `style_changed` with property delta + greppable anchor.
10. **Two sections swapped** (G3) → single `component_swapped`.
11. Render-equivalent DOM change (Section 13.2) → no issue (or info under `strict-visual`).
12. New 404 asset → `network_error`; new console error → `console_error`.
13. Page-height mismatch + masked timestamp → height delta reported; masked region produces no diff; differing heights produce no false pixel-change flood below the common height.
14. Old 200 / new 404 (rendered error page) → single `status_code_mismatch` (critical); content diff short-circuited.
15. **Global stylesheet defect** (e.g. font-family changed on every text block) → one cluster referencing all member issues.
16. **Intentional restyle + seeded defects (false-positive budget).** A pair with a deliberate visual redesign and DOM rewrite, into which N known defects are seeded (one missing section, one broken link, one gradient loss, one swap). *Acceptance:* under the default profile, **all N seeded defects are found and zero false `missing`/`added` issues are emitted.** This fixture is the noise-floor regression test for the matcher.

### 13.2 Render-equivalent DOM change (must NOT flag)
Old `<a class="btn primary" href="/donate">Donate</a>` vs
New `<div class="cta"><a href="/donate" role="button">Donate</a></div>`
→ matcher pairs them (same role/href/text — identity-first); no `missing`/`added`; at most an `info` implementation note when implementation comparison is explicitly enabled.

### 13.3 Unit / integration / golden
- **Unit (Rust):** text normalization, link/URL normalization, locale parser, color normalization, gradient parser, similarity scoring, identity/assignment matching, sequence/LCS, severity + scoring, content-addressed issue IDs (including re-capture stability: jittered bbox/score inputs produce the same ID), clustering rules, baseline suppression, egress policy (scheme/scope/IP-range refusal), schema (de)serialization.
- **Integration (TS):** Playwright load, stabilization determinism (including `page.clock` and the retry-without-freeze path), screenshot capture, page-model + computed-style extraction (candidate set + budget/truncation), `CaptureBundle` schema validity.
- **Golden — two distinct suites:**
  - **Analysis goldens (byte-exact):** committed `CaptureBundle` fixture files → `DiffResult` compared **byte-identically** (modulo `runId`/timestamps). This is what verifies the Section 15 determinism invariant.
  - **End-to-end goldens (tolerant):** fixture HTML pairs → capture → `DiffResult` compared with float tolerances on scores (capture introduces machine-scoped variation; see §3.3).

---

## 14. CLI & config

```bash
matchy --old https://old.example.com/about \
       --new https://new.example.com/about \
       --out ./report \
       --profile content-structure \
       --viewport desktop=1440x1000 --viewport mobile=390x844 \
       --hide ".chat-widget,.cookie-banner" --mask ".timestamp" \
       --baseline accepted.json \
       --trailing never \
       --fail-on error \
       --json --html --markdown

matchy doctor        # verify node/playwright/chromium and print remediation steps
```

Exit codes: `0` pass; `1` failed configured threshold (`--fail-on`, evaluated on post-profile severities); `2` tool/runtime error (page load failure on both pages, browser crash, capture failure, schema violation).

### Runtime requirements (documented, not bundled)

The curl installer (`install.sh`) delivers exactly two artifacts: the `matchy` binary and the bundled `capture.cjs`. It never installs Node or browsers. The host must already provide, on `PATH`:

| Requirement | Version | Notes |
|---|---|---|
| Node.js | ≥ 20 (tested on 24.x) | runs `capture.cjs` |
| Playwright | pinned exactly in `packages/capture/package.json` (1.60.x at time of writing) | a **host dependency**, not bundled: `playwright-core` resolves its driver/`browsers.json` via `__dirname`, so it cannot be flattened into `capture.cjs`. Install globally (`npm install -g playwright`); `matchy` spawns `node capture.cjs` with `NODE_PATH=$(npm root -g)` so the `require("playwright")` resolves. Version recorded in the environment fingerprint. |
| Chromium | the build matching the pinned Playwright (`npx playwright install chromium`) | `matchy doctor` checks for it |
| (build only) Rust | ≥ 1.85 | building from source |

`matchy doctor` is the support tool for all of this: it checks each requirement and prints the exact command to fix what's missing.

### Config file

Mirrors flags and adds `matching` (`identityFloor`, `tieMargin`, `matchFloor`, `noMatchCeil`, per-kind weight overrides), `stabilization`, `thresholds` (`minRegionArea`, `visualThreshold`, `clusterMin`), `interactions` (`clickBeforeCapture`), `redact`, `concurrency`, `egress` (`allowExternalProbes`).

### Privacy, redaction, locality

- **All page content stays local, period.** The tool never transmits screenshots, DOM, or page content to any third party. There is no AI/LLM integration to enable.
- **Redaction defaults (always on):** `Authorization`, `Cookie`, and `Set-Cookie` headers are never recorded. Known token-bearing query parameters (`token`, `sig`, `signature`, `key`, `auth`, `apikey`, `access_token`, …, extendable via `redact` config) are replaced with `…redacted…` in `network.requests[].url` and `redirectChain` before they are written into any bundle, log, or report.

---

## 15. Non-negotiable invariants (checklist for the agent)

- [ ] `DiffResult` validates against `/contract/diff-result.schema.json`; serialized Rust and TS output are both validated against the schema in CI over shared fixtures; a mismatch fails the build.
- [ ] Every issue has a `locator` with an agent-facing **anchor set**, and where actionable a structured `remediation` with grep targets. The tool never names a source component.
- [ ] Issue IDs are content-addressed over the **stable subset** defined in §7.1 (type + viewport + anchors + style property) — never over bboxes, selectors, scores, or paths — so they survive re-captures and the same defect keeps the same ID across the fix→re-run loop.
- [ ] **Analysis is byte-deterministic:** no map-iteration-order dependence, total-ordered tie-breaks, fixed-order float reductions; identical bundles → identical `DiffResult` (modulo timestamps). Verified by the byte-exact analysis golden suite (§13.3).
- [ ] Matching is **identity-first** with confidence bands (`identityFloor`, `matchFloor`/`noMatchCeil`), not a single hard cutoff; position never vetoes a strong identity match; per-signal sub-scores and deciding stage are written to `evidence.match`.
- [ ] Capture determinism is **machine-scoped** (no Docker): environment fingerprint recorded in every bundle; pixel baselines never compared across mismatched fingerprints; uncontrollable page nondeterminism is masked or flagged low-confidence, never silently varied.
- [ ] All network probing obeys the §10.3 egress policy: http/https only, same-registrable-domain by default, private/link-local/metadata ranges refused.
- [ ] HTML/Markdown renderers treat **all page-derived strings as untrusted**: HTML-escaped on output, no raw interpolation into markup or attributes; the generated HTML report ships a restrictive CSP and no inline event handlers.
- [ ] Each of G1–G8 has at least one passing fixture, and the M6 real-pair calibration gate has been completed, before v1 is "done."
- [ ] No production form submission, auth action, or interactive probing in v1.
- [ ] The tool contains **no AI/LLM integration and no agent-harness logic** — it is deterministic code end to end; all page content stays local.
