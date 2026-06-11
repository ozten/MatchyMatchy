# Issue: Default time-freezing crashes target-page JS and silently corrupts the baseline capture

**Status:** FIXED (2026-06-11 — see ROOT-CAUSE-AND-PLAN.md and docs/golden-changelog.md)
**Found:** 2026-06-11, during a real migration gate (Webflow staging page vs local Next.js port)
**Severity:** P0 — corrupted baseline causes all subsequent diffs to measure the wrong ground truth; fix rounds are wasted optimizing against a phantom reference
**Area:** stabilizer / capture integrity

---

## Summary

When matchy's default clock-freeze (`--freeze-time`, on by default) is active, Swiper.js v11
calls `clock.runFor` during its initialization sequence, which triggers a fake timer that
fires `re.slideTo` before the carousel DOM is ready. This raises an uncaught
`TypeError: Cannot read properties of undefined (reading 'style')` inside the frozen-clock
context. The stabilizer catches the exception, logs it as a non-fatal step failure
(`determinism.lazyLoadPass = "failed"`), and continues the run without retrying without the
clock freeze. No run-level warning appears in `report.md` or `diff-result.json`.

The practical consequence is that the baseline screenshot is captured in a broken carousel
state. On the Number Registration page this produced a baseline with **24 image-kind nodes
instead of the correct 19**: five extra carousel logo images (A-LIGN ISO-27001, IAF, ANAB,
AICPA, globe SVG) appeared in the frozen capture but not in the clean `--no-freeze-time` run.
Because the diff tool measures the candidate against this corrupted reference, every
subsequent fix round was scored against phantom content, and developers spent time chasing
discrepancies that do not exist in either real page.

---

## Environment

- matchy 0.1.0 (d5f0713)
- Linux; node v24.15.0
- Chrome Headless Shell 148.0.7778.96 (playwright chromium-headless-shell v1223)
- `old` URL: `https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration`
- `new` URL: `http://localhost:3001/products/connect/number-registration`
- Viewports: desktop 1440×1000, mobile 390×844
- Profile: `content-structure`
- Run directory: `/tmp/matchy-nr-round9` (runId `2026-06-11T17-57-31Z`)
- Clean-run directory: `/tmp/matchy-nr-10` (runId `2026-06-11T18-12-29Z`, `--no-freeze-time`)

---

## Reproduction

```bash
# Reproduces with default flags (time-freeze on):
matchy run \
  --old "https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration" \
  --new "http://localhost:3001/products/connect/number-registration" \
  --profile content-structure \
  --output /tmp/matchy-repro

# Check for the failure:
cat /tmp/matchy-repro/desktop/old.bundle.json | python3 -c \
  "import json,sys; d=json.load(sys.stdin); print(d['determinism']['lazyLoadPass'], d['determinism']['retriedWithoutTimeFreeze'])"
# Expect: failed false

# Compare image count vs clean run:
matchy run --no-freeze-time \
  --old "https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration" \
  --new "http://localhost:3001/products/connect/number-registration" \
  --profile content-structure \
  --output /tmp/matchy-repro-clean

python3 -c "
import json
for path, label in [('/tmp/matchy-repro/desktop/old.bundle.json','frozen'),
                    ('/tmp/matchy-repro-clean/desktop/old.bundle.json','clean')]:
    d = json.load(open(path))
    imgs = [n for n in d['page']['nodes'] if n.get('kind')=='image']
    print(label, len(imgs), 'image nodes')
"
```

The reproduction rate is nondeterministic — the crash depends on Swiper's timer racing the
frozen clock — but the conditions were hit on multiple independent runs during the session.

---

## Observed

1. **Stabilizer log** (captured in `page.console` of `old.bundle.json`):

   ```
   TypeError: Cannot read properties of undefined (reading 'style')
     at re.setTransition (https://cdn.jsdelivr.net/npm/swiper@11/swiper-bundle.min.js:13:53693)
     at re.slideTo (...:13:26418)
     at ...swiper-bundle.min.js:13:28190
     at ClockController._callFirstTimer (<anonymous>:285:20)
     at ClockController._runTo (<anonymous>:120:33)
     at <anonymous>:168:38
   ```

2. **Bundle fields** (`/tmp/matchy-nr-round9/desktop/old.bundle.json`):

   ```json
   "determinism": {
     "timeFrozen": "ran",
     "lazyLoadPass": "failed",
     "retriedWithoutTimeFreeze": false
   }
   ```

3. **Image node count divergence** (desktop viewport, old/baseline capture):
   - Frozen run (`matchy-nr-round9`): **24 image-kind nodes**
   - Clean `--no-freeze-time` run (`matchy-nr-10`): **19 image-kind nodes**
   - Extra nodes in frozen capture: `A-LIGN_ISO-27001.webp`, `logo-iaf.svg`,
     `logo-anab.svg`, `logo-aicpa.svg`, `globe.svg` — all carousel logo images
     that should be in an off-screen or uninitialized carousel slide

4. **No warning in output artifacts**: `report.md` contains no mention of the
   stabilizer failure; `diff-result.json` has no `captureIntegrity`, `degraded`, or
   warning field referencing `lazyLoadPass: "failed"`.

5. **Downstream score impact**: the diff of the corrupted baseline against a
   legitimate candidate reported `style: 0.00154` (vs the correct band). Fix
   attempts in rounds 1–8 of the session were scored against a baseline containing
   5 phantom images that the candidate correctly does not have.

---

## Expected

- When any stabilizer step throws an error originating from page JS interacting with
  injected time-freeze infrastructure (i.e., the stack trace contains
  `ClockController` or `clock.runFor`), matchy should auto-retry the capture without
  the clock freeze and record `retriedWithoutTimeFreeze: true`.
- If the retry is skipped or unavailable, the run should be marked **degraded** and
  `report.md` / `diff-result.json` should surface a prominent capture-integrity
  warning naming the failed step and its consequence.
- A capture-integrity self-check (inventory of heading count and landmark count
  before vs after stabilization) should detect gross content loss and abort or warn
  before the diff proceeds.

---

## Evidence

All evidence is on disk and was verified:

| Artifact | Path | Key value |
|---|---|---|
| Frozen bundle determinism | `/tmp/matchy-nr-round9/desktop/old.bundle.json` | `lazyLoadPass: "failed"`, `retriedWithoutTimeFreeze: false` |
| Frozen image count | same | 24 image-kind nodes |
| Clean bundle determinism | `/tmp/matchy-nr-10/desktop/old.bundle.json` | `lazyLoadPass: "ran"` |
| Clean image count | same | 19 image-kind nodes |
| Swiper TypeError | `page.console` in frozen bundle | full stack trace above |
| No warning in report | `/tmp/matchy-nr-round9/report.md` | grep `lazyLoad\|stabiliz\|freeze\|warn\|corrupt\|integrity` → 0 hits |
| No warning in diff-result | `/tmp/matchy-nr-round9/diff-result.json` | keys: `schemaVersion toolVersion runId … determinism artifacts` — no `captureIntegrity` or `degraded` |

Extra images present in frozen but absent in clean (verified by set difference on
`node.src` values):

```
6a03b33751ddb17efd3cd001_A-LIGN_ISO-27001.webp
6a03b33751ddb17efd3cd5c8_logo-iaf.svg
6a03b33751ddb17efd3cd5c9_logo-anab.svg
6a03b33751ddb17efd3cd5ca_logo-aicpa.svg
6a03b33751ddb17efd3cd5cd_globe.svg
```

---

## Suggested fix direction

1. **Auto-retry without clock freeze on ClockController-originated step exceptions.**
   When `lazyLoadPass` (or any stabilizer step) throws and the stack trace implicates
   `ClockController` / `clock.runFor`, re-run the entire capture with
   `--no-freeze-time` and set `retriedWithoutTimeFreeze: true` in the bundle. Log the
   retry at `info` level.

2. **Promote `lazyLoadPass: "failed"` to a run-level warning.** Any step that reaches
   `"failed"` rather than `"ran"` or `"skipped"` should produce a visible warning
   block in `report.md` and a `warnings` array entry in `diff-result.json`, regardless
   of whether the run continues.

3. **Capture-integrity check.** Before and after stabilization, compare counts of
   `h1`–`h3` heading nodes and named landmark regions. A delta beyond a small
   threshold (e.g., any heading disappears, or image count changes by more than 20%)
   should halt the run with an explicit integrity-failure error rather than silently
   proceeding to diff.

4. **Workaround (immediate):** pass `--no-freeze-time` when profiling pages that
   use Swiper.js v11 or similar timer-driven carousels. This produces a clean capture
   (`lazyLoadPass: "ran"`, 19 image nodes) and eliminates the phantom-content
   diff noise.
