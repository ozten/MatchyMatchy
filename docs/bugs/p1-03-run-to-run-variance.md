# Issue: Large run-to-run variance in issue counts and scores with no candidate change

**Status:** FIXED (2026-06-11 — see ROOT-CAUSE-AND-PLAN.md and docs/golden-changelog.md)
**Found:** 2026-06-11, during a real migration gate (Webflow staging page vs local Next.js port)
**Severity:** P1 — per-run scores are not reliable indicators of whether a fix helped; count-based or score-based CI gates would flap on an unchanged candidate
**Area:** capture determinism / scoring stability

---

## Summary

Across approximately 29 runs during a single working session — with the same URLs, same
flags, and no changes to the candidate (`new`) page between runs — matchy produced error
counts ranging from 4 to 155 and content scores ranging from 0.013 to 1.0. Even in the
narrower band of runs that used `--no-freeze-time` (eliminating the clock-crash
contribution from p0-01), error counts still ranged from 116 to 128 and content scores
from 0.0130 to 0.0169. The style score moved by 2× between consecutive runs
(`0.00119` → `0.00254`). No candidate change was made between those runs.

Two identifiable contributors drive the variance:

1. **Stabilizer crash from time-freeze** (p0-01): when `lazyLoadPass` fails, the
   baseline capture is corrupted, and the diff reflects phantom content. This produces
   the extreme outliers (content 0.013 with 129+ errors vs content 0.2 with 4 errors).

2. **Rotating notification bar on the source (old) page**: the Webflow staging site
   serves region-specific content variants in a notification bar between page loads.
   Different variants expose different link href sets, causing issue counts to drift
   by ~10–15 errors even on clean (unfrozen) runs.

A developer cannot tell from a single run's score whether a code change improved,
degraded, or had no effect on parity. Count-based CI gates (e.g., "fail if errors
> 10") would produce false positives or false negatives depending on which page
variant was served.

---

## Environment

- matchy 0.1.0 (d5f0713)
- Linux; node v24.15.0
- Chrome Headless Shell 148.0.7778.96 (playwright chromium-headless-shell v1223)
- `old` URL: `https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration`
- `new` URL: `http://localhost:3001/products/connect/number-registration`
- Viewports: desktop 1440×1000, mobile 390×844
- Profile: `content-structure`
- Session: ~29 runs, `matchy-nr-1` through `matchy-nr-round9`, all 2026-06-11

---

## Reproduction

Run matchy twice in immediate succession with no candidate change:

```bash
for i in 1 2 3; do
  matchy run \
    --old "https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration" \
    --new "http://localhost:3001/products/connect/number-registration" \
    --profile content-structure \
    --no-freeze-time \
    --output /tmp/variance-repro-$i
done

python3 -c "
import json, glob
for path in sorted(glob.glob('/tmp/variance-repro-*/diff-result.json')):
    d = json.load(open(path))
    errors = sum(1 for i in d['issues'] if i['severity'] == 'error')
    print(path.split('/')[-2],
          'content=%.4f' % d['scores']['content'],
          'hygiene=%.4f' % d['scores']['hygiene'],
          'errors=%d' % errors)
"
```

Expected (if deterministic): identical lines. Actual: counts and scores diverge.

---

## Observed

**All 29 session runs (no candidate changes between runs):**

| Metric | Min | Max | Notes |
|--------|-----|-----|-------|
| Error-severity issues | 0 | 155 | Full range including corrupted captures |
| Error-severity issues (`--no-freeze-time` runs only) | 116 | 128 | 14 runs |
| Content score | 0.0130 | 1.0000 | Full range |
| Content score (`lazyLoadPass: "ran"` only) | 0.0152 | 0.0169 | 14 runs |
| Style score | 0.00119 | 0.00254 | Both clean and frozen runs |
| `lazyLoadPass` values seen | — | — | `failed`, `ran`, `skipped` all observed |

**Pair most relevant to P1 (runs nr-2 vs nr-3, minutes apart, same flags):**

| | `matchy-nr-2` | `matchy-nr-3` (with `--baseline` from nr-2) |
|---|---|---|
| RunId | `2026-06-11T17-12-49Z` | `2026-06-11T17-15-32Z` |
| Error issues | 129 | 4 active + 120 suppressed |
| Content score | 0.0152 | 0.200 |
| Hygiene score | 0.333 | 1.0 |
| Style score | 0.00119 | 0.00119 |

Note: the hygiene and content score jumps here are partly explained by baseline
suppression, but the underlying issue count shift (129 → 124 unsuppressed in the
raw run, before the baseline) is still caused by rotate-content nondeterminism.

**Representative clean-run sample (no baseline, `--no-freeze-time`, consecutive):**

| Run | Errors | Content |
|-----|--------|---------|
| `matchy-nr-r3a` | 120 | 0.01695 |
| `matchy-nr-r3b` | 118 | 0.01639 |
| `matchy-nr-r3c` | 128 | 0.01563 |
| `matchy-nr-r3d` | 116 | 0.01695 |
| `matchy-nr-r4`  | 123 | 0.01613 |
| `matchy-nr-r5`  | 125 | 0.01515 |
| `matchy-nr-r6`  | 127 | 0.01563 |

Spread across these 7 consecutive clean runs: errors 116–128 (range 12),
content 0.01515–0.01695 (range 0.00180).

**`lazyLoadPass` variance:** across all 29 runs the old-capture `lazyLoadPass` field
took values `failed` (14 runs), `ran` (14 runs), `skipped` (1 run) — the crash is not
deterministic and its presence or absence directly governs whether a run lands in the
corrupted-baseline regime.

---

## Expected

- Two consecutive runs with no candidate change and the same configuration should
  produce identical (or near-identical) scores and issue counts.
- Score movement should be a reliable signal that the candidate changed, not that
  the external page served a different content variant.
- A developer should be able to answer "did my last commit improve parity?" from
  a single run result; currently two runs minutes apart can show a 13× difference
  in error count and a 15× difference in content score with no change to the
  candidate.

---

## Evidence

All evidence is on disk and was verified:

| Artifact | Path | Key value |
|---|---|---|
| Full run table | `/tmp/matchy-nr-*/diff-result.json` (29 files) | Tabulated above |
| lazyLoadPass variance | all bundles `determinism.old.lazyLoadPass` | `failed/ran/skipped` across runs |
| nr-2 scores | `/tmp/matchy-nr-2/diff-result.json` | `content: 0.01515, hygiene: 0.333` |
| nr-3 scores | `/tmp/matchy-nr-3/diff-result.json` | `content: 0.200, hygiene: 1.0` |
| Clean-run error range | runs `r3a`–`r6` | 116–128 errors across 7 runs, no candidate change |
| Style score range | all runs | `0.00119` (nr-2) to `0.00254` (r3c) |
| Rotating content hypothesis | `page.nodes` text comparison nr-2 vs nr-3 | same node count (135), text set identical → rotation produces structural differences not captured in text nodes alone; link href sets diverge nondeterministically |

---

## Suggested fix direction

1. **Determinism self-check (`analyze` subcommand).** Capture the `old` URL twice
   in the same run (before diffing against `new`) and self-diff the two old captures.
   Any issues that appear in the self-diff are nondeterminism noise, not real
   old-vs-new differences. Surface a `captureVolatility` score alongside existing
   scores and subtract volatile issue clusters from the main issue count. The
   existing `analyze` subcommand machinery appears to support this; it should be
   invoked automatically or via a `--self-check` flag.

2. **Per-cluster volatility flags.** Track which issue clusters recur across runs
   and which appear/disappear. Flag clusters with low inter-run stability as
   `volatile` in `diff-result.json` so that a CI gate can choose to ignore them
   or route them to a separate review bucket.

3. **Auto-suggest `--mask` for volatile regions.** When the self-check detects that
   a region of the old page differs between two captures of the same URL, emit a
   suggested `--mask` directive covering that region so the developer can suppress
   it in future runs.

4. **Optional median-of-N capture mode.** Allow `--capture-samples N` to capture
   the old (and optionally new) page N times and use the modal or median node set
   for diffing. This smooths out both the rotating-content variance and any residual
   Swiper-timing variance without requiring the user to identify the unstable region
   manually.

5. **Workaround (immediate):** use `--no-freeze-time` to eliminate the clock-crash
   contribution, which collapses the extreme-outlier band (content 0.013–1.0 with
   clock-crash included → 0.015–0.017 without). The residual ±12-error variance from
   rotating page content still requires the self-check or masking to eliminate.
