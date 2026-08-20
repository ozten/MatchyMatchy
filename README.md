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

## How matchy compares

matchy was benchmarked head-to-head against the two established old↔new page-comparison tools, over a
shared corpus of 21 single-change page pairs (this repo's own testbed). Across the 16 same-path
variants — 14 real regressions plus 2 render-identical controls:

| Tool | Mechanism | Regressions detected | False positives | Also detects URL/transport defects |
|---|---|:--:|:--:|:--:|
| [SiteDiff](https://github.com/evolvingweb/sitediff) | HTTP fetch → HTML-text diff | 8 / 14 | 1 / 2 | 0 / 5 |
| [Wraith](https://github.com/bbc/wraith) | screenshot → pixel diff | 11 / 14 | 0 / 2 | 0 / 5 |
| **matchy** | browser render → semantic diff | **14 / 14** | **0 / 2** | **5 / 5** |

SiteDiff is blind to anything that doesn't change the HTML source (CSS-only regressions, a deleted
image whose `src` is unchanged) and false-positives on render-equivalent DOM rewrites; Wraith is blind
to anything invisible (a broken link that still renders, a console error, a missing `lang`); both miss
the entire URL/transport class. The benchmarks are written to be fair — each report has a "where the
other tool is better" section (SiteDiff's crawler, Wraith's pixel-truth gallery).

**Full reports:** [How matchy compares — overview](docs/benchmarks/README.md) ·
[matchy vs SiteDiff](docs/benchmarks/matchy-vs-sitediff.md) ·
[matchy vs Wraith](docs/benchmarks/matchy-vs-wraith.md) ·
[capability matrix](docs/benchmarks/capability-matrix.md)

## Installation

```bash
curl -fsSL https://raw.githubusercontent.com/ozten/MatchyMatchy/main/scripts/install.sh | bash
```

The installer delivers two artifacts: the `matchy` binary and a bundled `capture.cjs`. It does **not** install Node or browsers — those are host requirements:

| Requirement | Version |
|---|---|
| Node.js | ≥ 20 |
| Playwright | **exactly `1.60.0`** — install **globally** and pinned so `capture.cjs` can resolve it: `npm install -g playwright@1.60.0` |
| Chromium | the build that Playwright `1.60.0` pins (currently build `1223`) — pulled automatically by `npx playwright install chromium` |
| System libraries | the shared libs Chromium links against (`libatk-1.0.so.0`, `libnss3`, `libgbm`, …) — **without these the browser downloads but won't launch** |

Pin the Playwright version exactly. The Chromium build is **not** chosen by matchy — it is derived from whatever Playwright version is installed, so a mismatched Playwright pulls a mismatched Chromium and `matchy doctor` will fail. Install pinned, then let Playwright fetch its matching browser, then install the system libraries it needs to launch:

```bash
npm install -g playwright@1.60.0
npx playwright install chromium

# system libraries — pick the line for your distro:
sudo npx playwright install-deps chromium        # Debian / Ubuntu
sudo dnf install -y nss nspr atk at-spi2-atk at-spi2-core cups-libs \
  libdrm libxkbcommon libXcomposite libXdamage libXext libXfixes \
  libXrandr libgbm mesa-libgbm libX11 libxcb pango cairo alsa-lib   # RHEL / Amazon Linux
```

> **Heads up — the most common failure.** If `matchy doctor` reports *"Chromium build 1223 not found"* but the build is clearly downloaded, the real cause is almost always **missing system libraries**: doctor verifies Chromium by *launching* it, and a launch failure surfaces as "not found." Confirm with `ldd ~/.cache/ms-playwright/chromium-1223/chrome-linux64/chrome | grep "not found"` (empty output = all libs present). `playwright install-deps` only supports Debian/Ubuntu; on Amazon Linux / RHEL install the equivalents with `dnf` as above, then re-run for any straggler with `sudo dnf provides '*/libNAME.so.0'`.

Run `matchy doctor` after installing — it checks every requirement and prints the exact command to fix anything missing.

### Build from source

Prefer to build it yourself? Clone the repo and run `make build`. You need a Rust toolchain (≥ 1.85) and Node.js (≥ 20); the build compiles the `matchy` binary and bundles the capture layer.

```bash
git clone https://github.com/ozten/MatchyMatchy && cd MatchyMatchy
source "$HOME/.cargo/env"   # if cargo isn't already on your PATH
make build                  # cargo build --release + packages/capture bundle
```

This produces:

- the release binary at `target/release/matchy`
- the bundled `capture.cjs` at `packages/capture/dist/capture.cjs`

`matchy` shells out to `capture.cjs` at runtime, so keep the two together when you install them — copy both into the same directory (e.g. `~/.local/bin`), exactly as the install script does. To build only the binary, run `cargo build --release` (skips the capture bundle). Then run `matchy doctor` to confirm Node, Playwright, and Chromium are present.

### Where `capture.cjs` goes

`matchy` runs the capture layer by shelling out to `capture.cjs`, so it has to locate that file at runtime. **The install script does this for you** — it places `capture.cjs` directly beside the `matchy` binary (both land in `/usr/local/bin`, or `~/.local/bin` when that isn't writable), which is the first location `matchy` looks. If you install manually or relocate the binary, keep the two together or point `matchy` at the file explicitly.

Resolution order, first match wins:

| Order | Location | When it applies |
|---|---|---|
| 1 | `$MATCHY_CAPTURE_PATH` | Set to an absolute path to keep `capture.cjs` anywhere you like (override / escape hatch) |
| 2 | Sibling of the binary — `<dir-of-matchy>/capture.cjs` | The default; what the install script sets up |
| 3 | `<ancestor>/packages/capture/dist/capture.cjs`, walking up from the binary | Running a binary built inside a repo checkout |
| 4 | `<cwd>/packages/capture/dist/capture.cjs` | Running from inside a repo checkout |

In short:

- **Installed from a release** → leave `capture.cjs` next to `matchy` — the install script already does this.
- **Custom location** → `export MATCHY_CAPTURE_PATH=/path/to/capture.cjs`.
- **Running from a repo checkout** → nothing to place; it resolves to `packages/capture/dist/capture.cjs`.

`matchy doctor` confirms the file is resolvable and prints the fix if it isn't.

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
- `--self-check` — capture the old URL a second time and diff it against itself, writing the old-vs-old result to `self-check.json`. Any issues found there are capture volatility, not real differences: if the probe finds drift, a `volatile_capture` warning (with an issue count and breakdown by type) is added to the main result's `warnings[]`; if the probe itself fails for one or more viewports, a `self_check_failed` warning is added (both can appear when only some viewports fail). Either way, self-check never changes the exit code.

A config file mirrors all flags and adds tuning for matching thresholds, stabilization, visual thresholds, redaction, and egress.

### Severity mapping

`--severity-map <path>` points at a JSON file that overrides how issues are scored, on top of the built-in defaults:

```json
{
  "types": { "pseudo_element_missing": "error" },
  "properties": { "letter-spacing": "info", "line-height": "info" }
}
```

- `types` keys are wire issue-type names (the same strings that appear in `issues[].type`, e.g. `style_changed`, `missing_link`).
- `properties` keys are CSS property names, and apply to property-carrying style issues (`style_changed` and the gradient types) on any style channel — keyed on the issue's own `remediation.property`, not its type.
- Values are one of `info` / `warning` / `error` / `critical`.
- An unrecognized type or property key, or a malformed file, is a hard error: `matchy` exits `2` and names the bad key on stderr.

**Resolution order**, most general to most specific:

1. **Profile category default** — the `--profile`'s category → severity table (e.g. `style` is `warning` under `content-structure`, `error` under `strict-visual`), including four fixed overrides regardless of profile: `accessibility_improved` → info, `console_error` → warning, `load_error` / `status_code_mismatch` → critical, `missing_form` → critical.
2. **Built-in overrides** — shipped opinionated defaults that fire before any user map: `clickable_area_regressed` is always `error` (never silently demoted to info by the visual category), and `letter-spacing` / `line-height` style diffs are demoted to `info` (these two properties dominate the flood of low-signal, sub-pixel/leading style noise a real port produces).
3. **Your `--severity-map`** — overrides both of the above. Within both layer 2 and layer 3, a `properties` match beats a `types` match for the same issue (more specific wins).
4. **Deny-list (always wins)** — `load_error`, `status_code_mismatch`, and `missing_form` can never be demoted below `critical`, even by your map. An attempted demotion is silently *ignored* (never applied) and reported as a `severity_map_denied` warning in `warnings[]`, naming the type and the attempted severity.

The resolved overrides your map actually contributed (denied entries excluded) are echoed back on the result as `severityMap`, so two runs compared with different maps are never silently incomparable:

```jsonc
"severityMap": {
  "source": "file",
  "overrides": {
    "types": { "pseudo_element_missing": "error" },
    "properties": { "letter-spacing": "info", "line-height": "info" }
  }
}
```

`severityMap` is `null` when `--severity-map` isn't passed.

Info-severity issues are excluded from that category's `scores.*` value (see [The DiffResult contract](#the-diffresult-contract)), so demoting a noisy property or type with `--severity-map` legitimately raises the corresponding score — that's the intended lever for tuning signal-to-noise without touching the underlying detectors.

### Gating on issues

`agentSummary.byType` and `agentSummary.bySeverity` are counts over the exact same kept set: after `--baseline` suppression and `--scope` partitioning have both been applied. Suppressed issues (in `suppressed.ids`) and out-of-scope issues (in `outOfScope.ids`) are excluded from both maps. Both maps are always present — an empty object `{}` when nothing survives, never absent — so a gate can read them directly without re-deriving counts from `issues[]`:

```bash
matchy --old https://old.example.com --new https://new.example.com \
       --out ./report --scope main --baseline ledger.json --json

jq -e '(.agentSummary.bySeverity.error // 0) == 0 and (.agentSummary.bySeverity.critical // 0) == 0' \
   ./report/diff-result.json
```

That asserts "no unaccepted error-or-worse issues remain in the `main` landmark" without walking `issues[]`. The same guarantee makes `byType` usable the same way, e.g. `jq -e '(.agentSummary.byType.missing_form // 0) == 0'` to gate on one issue type specifically.

### Other commands

- **`matchy doctor`** — verify Node.js, Playwright, and Chromium are present and print the exact fix for anything missing.
- **`matchy analyze --old-bundle <path> --new-bundle <path> --out <dir>`** — re-run analysis offline from two previously-saved `CaptureBundle` JSON files, with no browser, network, or Playwright. Produces a byte-deterministic `DiffResult`. Honors the global `--profile`, `--baseline`, `--severity-map`, `--scope`, and `--fail-on` flags and the same `0`/`1`/`2` exit codes as a full run (`--viewport` is irrelevant — the bundle carries its own).
- **`matchy explain --old-bundle <path> --new-bundle <path> --anchor "text=…"`** — read-only triage probe. Locates one element across the two bundles — by `--anchor "<key>=<value>"` (key ∈ `text`/`role`/`href`/`nearestHeading`), `--node <id>`, or `--selector "<css>"` — and prints its per-side computed-style + bbox values, diff-only by default (or restricted with `--props color,gap,…`). Use it to fact-check why an issue was or wasn't flagged. Hermetic: no browser or network.

The full CLI reference (flags, exit codes, screenshot resolution) lives in [`docs/prds/page-pair-diff-spec.md`](docs/prds/page-pair-diff-spec.md) §14.

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
| Playwright | pinned exactly 1.60.0 | bundled into `capture.cjs`; version recorded in every capture bundle. Install globally + pinned: `npm install -g playwright@1.60.0` |
| Chromium | build matching pinned Playwright (build `1223`) | derived from the Playwright version, not chosen by matchy; install with `npx playwright install chromium` |
| System libraries | Chromium's shared-lib deps (`libatk-1.0.so.0`, `libnss3`, `libgbm`, …) | required to *launch* Chromium. Debian/Ubuntu: `sudo npx playwright install-deps chromium`. Amazon Linux/RHEL: `sudo dnf install` the equivalents (see [Install](#install)). A launch failure here is reported by doctor as "build not found." |
| Rust | ≥ 1.85 (build only) | not required to run a pre-built binary |

Run `matchy doctor` after installing — it checks each requirement and prints the exact command to fix anything missing.

### Make targets

| Target | What it does |
|---|---|
| `make build` | `cargo build --release` + `packages/capture` npm install and bundle |
| `make verify` | Full CI gate: build, unit tests, testbed servers, M1 fixture checks, M8 acceptance, **Tier-3 real-pair gate**, golden comparisons, determinism spot-check |
| `make fixture VARIANT=vNN` | Run `check-fixture.py` for one variant (e.g. `VARIANT=v02-banner-added`) |
| `make pair CASE=pNN-…` | Replay + assert one Tier-3 real-pair fixture hermetically (`check-pair.py`) |
| `make pair-add CASE=… URL_OLD=… URL_NEW=… [PROFILE= VIEWPORT= HIDE= MASK=]` | Capture a real old/new URL pair, run the privacy gate, freeze it, and scaffold a fixture |
| `make pair-refresh CASE=…` | Re-capture an existing pair from its recorded flags (golden-discipline event) |
| `make testbed-up` | Start all testbed HTTP servers (golden + variants) |
| `make testbed-down` | Stop all testbed servers |
| `make testbed-check` | Verify all servers respond HTTP 200 and manifests validate |

### Regression fixtures: three tiers

The testbed has three tiers of regression fixtures, all gated by `make verify`:

- **Tier 1 — synthetic variants** (`testbed/variants/`): one deliberate single-change permutation per variant, served locally. Precise feature tests with hand-authored `expected-issues.json` intent.
- **Tier 2 — calibration pairs** (gitignored, run-once): real URLs captured during M6 calibration; not a permanent gate.
- **Tier 3 — real-pair regression fixtures** (`testbed/pairs/`): any real old/new URL pair where matchy misses a defect or floods noise, frozen into a deterministic, hermetic, CI-gated test. This is the convenience tier for the *"I hit a real example, add it to the bench, fix the tool"* loop.

**The Tier-3 loop:**

```
make pair-add CASE=pNN-slug URL_OLD=<old> URL_NEW=<new> [HIDE=…]   # capture once (only network step)
  → privacy gate (credential token-scan fail-closed + human PII/ownership review)
  → freeze bundles + screenshots under testbed/pairs/<case>/<viewport>/
  → scaffold pair.json (expectedState defaults "red") + an EMPTY expected-issues.json stub

matchy explain --old-bundle … --new-bundle … --anchor "text=…"     # triage: classify the diff offline
  → hand-author expected-issues.json (what matchy SHOULD emit); set demonstrates / expectedState / knownDrift

make pair CASE=pNN-slug                                            # assert hermetically (no servers/network)
make verify                                                        # the Tier-3 gate runs every committed pair
```

Fixtures replay from **frozen bundles** via `matchy analyze` — no Chromium, Playwright, testbed servers, or network — so they run in minimal CI. Every run re-checks the SHA-256 integrity of both bundles (mismatch is a hard error). A fixture is **`expectedState: "red"`** when it locks a defect matchy cannot yet handle: `check-pair.py` reports it as a gate-safe **XFAIL** (exit 0) so a *"commit red, fix later"* TDD entry does not break `main`. When the fix lands, flip `expectedState` to `"green"`. Committed captures are redaction-clean (credential params are scrubbed and the freeze gate fails closed otherwise); see [Privacy and network behavior](#privacy-and-network-behavior). The seed fixture `p01-hiya-number-registration` locks a real broken-link regression caught in a Webflow-staging → localhost rebuild.

## License

[MIT](LICENSE) © ozten
