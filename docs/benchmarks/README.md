# matchy vs SiteDiff vs Wraith — Combined Benchmark Summary

Three old↔new page-comparison tools over one shared corpus: the matchy testbed (golden page + 21 single-change variants, each isolating one known defect with hand-authored ground truth). Full methodology and per-tool analysis: [matchy vs SiteDiff](matchy-vs-sitediff.md), [matchy vs Wraith](matchy-vs-wraith.md), [capability matrix](capability-matrix.md). Source research briefings: [SiteDiff](tool-research-sitediff.md), [Wraith](tool-research-wraith.md).

## What each tool is

| | SiteDiff | Wraith | matchy |
|---|---|---|---|
| Mechanism | HTTP fetch → **HTML-text** diff | screenshot → **pixel** diff (ImageMagick) | browser render → **semantic** diff |
| Sees | raw markup | rendered pixels | DOM + style + pixels + transport + a11y + runtime |
| Output | per-path HTML diff | gallery + % per breakpoint | typed JSON `DiffResult` |
| Breadth | crawls many paths | crawls many paths × widths | single explicit pair (v1) |
| Runs on this aarch64 host | ✅ | ✅ *with manual chromedriver surgery* (PhantomJS path dead) | ✅ |

## Scorecard (16 in-model variants: 14 real regressions + 2 render-identical controls)

| Tool | Regressions detected | False positives on controls |
|---|:--:|:--:|
| SiteDiff | 8 / 14 | 1 / 2 |
| Wraith | 11 / 14 | 0 / 2 |
| **matchy** | **14 / 14** | **0 / 2** |

Plus **5 out-of-model URL/transport variants** that neither same-path tool can express: matchy detects **5/5**, SiteDiff and Wraith **0/5**.

## Per-variant, three-way

| Variant | Defect class | SiteDiff (markup) | Wraith (pixels) | matchy (semantic) |
|---|---|:--:|:--:|:--:|
| `v01-identical` | control | ✅ quiet | ✅ quiet | ✅ quiet |
| `v02-banner-added` | content | ✅ changed | ✅ 35.8% | ✅ visual-only |
| `v03-font-size` | style | ❌ quiet | ✅ 32.7% | ✅ style_changed×725 |
| `v04-font-family` | style | ❌ quiet | ✅ 46.1% | ✅ style_changed×463 |
| `v05-cta-style` | style | ❌ quiet | ✅ 31.2% | ✅ style_changed×15 |
| `v06-gradient-removed` | style | ❌ quiet | ✅ 38.9% | ✅ background_gradient_lost |
| `v07-sections-swapped` | structure | ✅ changed | ✅ 10.7% | ✅ component_swapped |
| `v08-cta-removed` | content | ✅ changed | ✅ 31.6% | ✅ missing_link |
| `v09-h1-changed` | content | ✅ changed | ✅ 0.4% | ✅ changed_h1 |
| `v10-paragraph-removed` | content | ✅ changed | ✅ 27.4% | ✅ missing_text |
| `v11-broken-link` | link | ✅ changed | ❌ quiet | ✅ broken_link, changed_link_target, url_protocol_downgrade |
| `v12-image-404` | asset | ❌ quiet | ✅ 44.2% | ✅ broken_image, network_error |
| `v13-render-equivalent` | control | ❌ changed | ✅ quiet | ✅ quiet |
| `v14-trailing-slash` | url-hygiene | — n/a | — n/a | ✅ url_trailing_slash |
| `v15-locale-underscore` | locale | — n/a | — n/a | ✅ locale_separator_invalid |
| `v16-locale-lowercase` | locale | — n/a | — n/a | ✅ locale_case_invalid |
| `v17-redirect-chain` | redirect | — n/a | — n/a | ✅ url_redirect_chain |
| `v18-status-mismatch` | status | — n/a | — (77%) | ✅ status_code_mismatch |
| `v19-container-gap` | style | ❌ quiet | ✅ 5.3% | ✅ style_changed×2 |
| `v20-console-error` | console | ✅ changed | ❌ quiet | ✅ console_error |
| `v21-a11y-lang` | a11y | ✅ changed | ❌ quiet | ✅ accessibility_regression |

*Controls (`v01`, `v13`): ✅ = correctly stayed quiet. `v13` is a render-equivalent DOM rewrite — SiteDiff false-positives on it; Wraith and matchy correctly ignore it. Out-of-model rows show a pixel %, in parentheses, only where a human might incidentally notice (e.g. the 404 page).*

## The three blind spots, side by side

- **SiteDiff misses what doesn't change the HTML source.** All 5 CSS-only regressions and the deleted image (whose `<img src>` is unchanged) are invisible to it — and it false-positives on a render-equivalent rewrite.
- **Wraith misses what doesn't change pixels.** A link whose target broke but renders identically, a console error, and a missing `lang` attribute are all 0.000% to it. Its `fuzz` knob (needed against cross-environment noise) also suppressed a real gradient regression entirely (36%→0%).
- **Both miss the entire URL/transport class** — trailing-slash policy, locale casing, redirect chains, status-code parity — because they compare same-path content, and these defects *are* the URL/transport.
- **matchy** surfaced every regression in the corpus with a typed, located, fixable issue, and stayed quiet on both controls.

## But matchy is not strictly better — where the older tools win

- **SiteDiff crawls.** One config covers hundreds of pages; matchy v1 takes a single explicit pair. For broad migration coverage, SiteDiff's crawler is a real advantage matchy lacks.
- **SiteDiff shows the exact markup edit.** For added content (`v02`'s new banner `<div>`) it names the inserted markup; matchy reports only visual regions there.
- **Wraith's pixel-truth + gallery** is the simplest, most assumption-free way to eyeball visual drift across breakpoints, sorted worst-first — and it needs zero knowledge of page structure.
- Both compete on **breadth**; matchy competes on **depth**.

## Bottom line

On a like-for-like *single-page* migration check, **matchy strictly dominates on detection and false positives** (14/14 + 5/5, zero FP) because it reasons over signals — DOM, computed style, transport, runtime, accessibility — that a markup-text diff or a pixel diff each only partially see. SiteDiff and Wraith remain valuable for what they were built for: **breadth** (crawl a whole site for markup or visual drift) and **simplicity**. The sharpest way to say it: SiteDiff and Wraith tell you *that* something changed on many pages; matchy tells you *what broke on this page, where, and how to fix it* — including the defects that never touch the markup or the pixels.

## Reproducing

The **corpus is this repo's own testbed** — the golden page plus the 21 single-change variants under
[`testbed/variants/`](../../testbed/variants), each with hand-authored `expected-issues.json` ground
truth. matchy's side of every table is its `DiffResult` on those variants (the committed goldens;
reproducible with `make fixture VARIANT=…`).

The competitor harness — the SiteDiff/Wraith runners, the shared corpus generator, the
mechanism-faithful Wraith screenshotter, and the real-Wraith setup — lives in a companion workspace,
`MatchyMatchBenchmark/`, kept out of this repo so it doesn't pull SiteDiff/Wraith Ruby tooling into
the build. Each runner is self-contained and deterministic:

```bash
cd ~/MatchyMatchy && make testbed-up           # start golden + 21 variant servers
cd ~/MatchyMatchBenchmark
python3 corpus/build_corpus.py                  # regenerate the shared ground truth
python3 sitediff-bench/run.py                   # -> SiteDiff-vs-Matchy report
python3 wraith-bench/run.py                     # -> Wraith-vs-Matchy report (mechanism-faithful)
python3 reports/build_summary.py                # -> this combined summary
```

Real Wraith was also run on aarch64 to validate the mechanism reproduction; the exact chromedriver
recipe is in the [matchy vs Wraith](matchy-vs-wraith.md#real-wraith-validation-the-actual-wraith-gem-on-aarch64)
appendix.

