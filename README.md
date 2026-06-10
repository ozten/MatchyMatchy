# matchy

Compare two renderings of the same page — old site vs. new site — and get a deterministic, machine-actionable diff of what broke in the migration.

```bash
matchy --old https://old.example.com/about --new https://new.example.com/about --out ./report
```

Given a URL pair, matchy renders both pages in a real browser, extracts what's actually visible, and produces a structured `DiffResult` JSON (plus static HTML and Markdown reports) describing every defect it finds: missing content, broken links, lost styles, reordered sections, accessibility regressions, URL hygiene problems. It is built to be consumed by coding agents and CI as much as by humans — every issue carries a greppable locator and a structured remediation, not just a screenshot of red pixels.

matchy is **pure deterministic code end to end**. There is no AI/LLM layer, no heuristic learning, and no network egress beyond the pages you point it at. All page content stays on your machine.

## What it detects

| Category | Examples |
|---|---|
| Content | Missing headings, paragraphs, CTAs, forms, images; changed titles and meta descriptions |
| Links & assets | Broken links, broken images, changed link targets, failed network requests |
| Style | Property-level CSS changes, lost or altered background gradients, container layout changes (`flex-direction`, `gap`, …) |
| Structure | Components reordered or swapped — reported as a reorder, not as a false missing+added pair |
| Visual | Pixel-diff regions clustered into bounding boxes, page-height changes |
| URL hygiene | Trailing-slash policy violations, redirect chains, protocol downgrades, canonical mismatches, status-code mismatches |
| Locale hygiene | Wrong-case locale segments (`es-mx`), underscore separators (`es_MX`), unknown locale codes |
| Accessibility | New axe-core violations relative to the old page |

## Installation

```bash
curl -fsSL https://example.com/matchy/install.sh | sh
```

The installer delivers two artifacts: the `matchy` binary and a bundled `capture.cjs`. It does **not** install Node or browsers — those are host requirements:

| Requirement | Version |
|---|---|
| Node.js | ≥ 20 |
| Playwright | the exact version pinned in the release (recorded in every capture) |
| Chromium | the build matching that Playwright version (`npx playwright install chromium`) |

Run `matchy doctor` after installing — it checks every requirement and prints the exact command to fix anything missing.

## Usage

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
```

Exit codes:

- `0` — pass
- `1` — issues at or above the configured `--fail-on` severity
- `2` — tool/runtime error (both pages failed to load, browser crash, capture failure)

Useful flags:

- `--hide <selectors>` / `--mask <selectors>` — hide volatile elements entirely, or paint over them while preserving layout (timestamps, ads, chat widgets).
- `--baseline accepted.json` — suppress issues you've reviewed and accepted (see [The migration loop](#the-migration-loop)).
- `--profile strict-visual | content-structure` — what counts as a failure. The default `content-structure` profile treats content/structure/hygiene problems as failures and pixel-level differences as informational, which is what you want when the redesign is intentional.
- `--fail-on info|warning|error|critical` — CI gate threshold.

A config file mirrors all flags and adds tuning for matching thresholds, stabilization, visual thresholds, redaction, and egress.

## The DiffResult contract

The JSON output is the primary product; the HTML report is just one renderer of it. It opens with a machine-first triage block:

```jsonc
"agentSummary": {
  "fixableNow": 6,
  "byType": { "missing_form": 1, "style_changed": 3, "component_reordered": 1, "url_trailing_slash": 1 },
  "clusterCount": 2,
  "topFixes": ["cluster_001", "issue_004", "issue_002"]
}
```

Every issue includes:

- **An anchor set** — a small fingerprint of semantic facts (visible text, `href`, `alt`, nearest heading, landmark, ordinal) that identifies the element and is directly greppable in your source. matchy never guesses a component name; it hands you facts that locate one.
- **Evidence** — old/new values, cropped screenshots, and the full per-signal match score that explains why two elements were paired (or weren't).
- **A structured remediation** — e.g. `restore_css_property` with the property, the `from`/`to` values, and grep targets.

Issues are sorted by fix value (severity × confidence × how easy the anchor is to find), so the JSON order is a ready-made work queue.

The schema lives in [`/contract`](contract/) as JSON Schema and is the single source of truth; both halves of the tool are validated against it in CI.

## The migration loop

Issue IDs are **content-addressed** over stable fields only (type, viewport, anchors, style property) — never over pixel positions, selectors, or scores. Fix an issue, re-run, and that ID is verifiably gone while every still-unfixed issue keeps its ID across re-captures of live pages.

That makes the intended workflow a shrinking queue:

1. Run matchy against the page pair.
2. Triage: intentional changes go into `accepted.json` (`--baseline`); real defects get fixed.
3. Re-run. Suppressed issues are counted but excluded from scoring; the queue shrinks monotonically to zero.

Systematic defects are clustered automatically: if one global stylesheet bug breaks `font-family` on 200 elements, you get one cluster work item, not 200 issues.

## How it works

```
--old/--new ──▶ capture (TypeScript + Playwright)     renders, stabilizes, screenshots,
                        │                             extracts page model + computed styles
                        ▼  CaptureBundle JSON
                analyze (Rust)                        pure function: matching, all diffs,
                        │                             scoring, clustering, rendering
                        ▼
                DiffResult JSON ──▶ HTML / Markdown / your agent or CI
```

A few properties worth knowing:

- **Deterministic analysis.** The analyze layer is a pure function: identical capture bundles produce a byte-identical `DiffResult`. Verdicts use confidence bands rather than knife-edge thresholds, so a 0.001 score wobble can't flip an issue on and off between runs.
- **Identity-first matching.** Elements are paired by what they *are* (text, href, alt, role), not where they sit. A section moved from the top of the page to the bottom is reported as a reorder — not as one missing component and one added one. Render-equivalent DOM rewrites (`<a class="btn">` → `<div><a role="button">`) match cleanly and produce no false positives.
- **Honest capture.** Before screenshotting, matchy kills animations, freezes time and `Math.random`, waits for fonts and images, and runs a lazy-load pass. What it can't control, it records: every bundle carries a determinism report and an environment fingerprint, and analysis lowers confidence on any evidence touched by a failed or masked stabilization step. Pixel baselines are machine-scoped; the semantic, style, structure, and hygiene diffs are the authoritative cross-environment signals.

## Privacy and network behavior

- No page content — screenshots, DOM, text — ever leaves your machine.
- `Authorization`, `Cookie`, and `Set-Cookie` headers are never recorded; known token-bearing query parameters are redacted from every bundle, log, and report.
- Probing (broken-link checks, redirect following) is restricted to `http`/`https` on the same registrable domain as your input URLs, with private/link-local/metadata IP ranges refused, unless you explicitly opt out with `--allow-external-probes`.
- matchy never submits forms, performs auth actions, or clicks anything you didn't explicitly configure via `clickBeforeCapture`.

## Limitations (v1)

- Single explicit URL pair only — no crawling, sitemaps, or batch mode.
- Both URLs must be publicly reachable; authenticated targets are not yet supported.
- Chromium only (WebKit/Firefox planned).
- Pixel-level baselines are valid only on the machine and Chromium build that produced them.
- matchy locates elements precisely but does not name your source components — resolving an anchor set to a file is the consumer's job.

## Development

The full specification lives at [`docs/prds/page-pair-diff-spec.md`](docs/prds/page-pair-diff-spec.md). The codebase is a two-language workspace: `packages/capture` (TypeScript, Playwright) and `packages/analyze` (Rust, the core and the `matchy` binary), joined by the `CaptureBundle` JSON seam defined in `/contract`. Local HTML fixture pairs in `/fixtures` back the golden test suites.

### Runtime requirements

The `matchy` binary delegates page capture to `capture.cjs` (Node.js + Playwright). These must be present on the host:

| Requirement | Version | Notes |
|---|---|---|
| Node.js | ≥ 20 (tested on 24.x) | runs `capture.cjs` |
| Playwright | pinned exactly 1.60.0 | bundled into `capture.cjs`; version recorded in every capture bundle |
| Chromium | build matching pinned Playwright | install with `npx playwright install chromium` |
| Rust | ≥ 1.85 (build only) | not required to run a pre-built binary |

Run `matchy doctor` after installing — it checks each requirement and prints the exact command to fix anything missing.

### Make targets

| Target | What it does |
|---|---|
| `make build` | `cargo build --release` + `packages/capture` npm install and bundle |
| `make verify` | Full CI gate: build, unit tests, testbed servers, M1 fixture checks, golden comparisons, determinism spot-check |
| `make fixture VARIANT=vNN` | Run `check-fixture.py` for one variant (e.g. `VARIANT=v02-banner-added`) |
| `make testbed-up` | Start all testbed HTTP servers (golden + variants) |
| `make testbed-down` | Stop all testbed servers |
| `make testbed-check` | Verify all servers respond HTTP 200 and manifests validate |

## License

[MIT](LICENSE) © ozten
