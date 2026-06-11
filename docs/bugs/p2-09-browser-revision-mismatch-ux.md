# Issue: Browser-revision mismatch prints Playwright banner three times; `doctor` does not check pinned revision or surface `PLAYWRIGHT_BROWSERS_PATH`

**Status:** OPEN
**Found:** 2026-06-11, during a real migration gate (Webflow staging page vs local Next.js port)
**Severity:** P2 — the triple banner is noise that obscures the real error; the doctor check passes without verifying the revision that capture actually needs, leaving the user with no actionable guidance
**Area:** `packages/analyze/src/doctor.rs` — `check_capture_doctor()`; `packages/analyze/src/orchestrate.rs` — capture retry loop

---

## Summary

When `PLAYWRIGHT_BROWSERS_PATH` is not set to the repo-local `.pw-browsers/` cache and the
shared `~/.cache/ms-playwright` contains a different Chromium build than the one pinned by
this repo's `packages/capture` (Playwright 1.60.0 → build 1223), every capture attempt prints
Playwright's "just installed or updated" advisory banner in full. Because matchy runs old
capture, new capture, and a retry, the banner appears three times. The real error —
`[CAPTURE_FAILED] browserType.launch: Executable doesn't exist at ...chromium_headless_shell-1223/...`
— is buried after three screens of advisory text.

`matchy doctor` reports the `chromium` check as OK when any Chromium build is installed
at `PLAYWRIGHT_BROWSERS_PATH` (or the default `~/.cache/ms-playwright`), but does not
verify that the installed build number matches the one pinned by `packages/capture`. A
passing doctor is therefore no guarantee that captures will succeed.

`docs/playwright-setup.md` documents the full setup convention: Playwright 1.60.0 is
pinned in `packages/capture/package.json` → Chromium build 1223; browsers live in a
repo-local `.pw-browsers/` cache selected via `PLAYWRIGHT_BROWSERS_PATH`; the Makefile
exports `PLAYWRIGHT_BROWSERS_PATH := $(CURDIR)/.pw-browsers` for every `make` target.
The shared `~/.cache/ms-playwright` holds `chromium-1217` (installed by a separate
`agent-browser` tool pinning Playwright 1.59.1). Running the bare binary without
`PLAYWRIGHT_BROWSERS_PATH` set causes Playwright inside `capture.cjs` to look in the
shared cache, fail to find build 1223, and print its install advisory.

## Environment

- matchy 0.1.0 (d5f0713); Linux; node v24.15.0; Chrome Headless Shell 148.0.7778.96
  (pw chromium-headless-shell v1223)
- old=https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration
- new=http://localhost:3001/products/connect/number-registration
- `~/.cache/ms-playwright` contains `chromium-1217` (Playwright 1.59.1 global install)
- `PLAYWRIGHT_BROWSERS_PATH` not set in the shell that ran the binary directly

## Reproduction

```bash
# Ensure PLAYWRIGHT_BROWSERS_PATH is unset (does not point to .pw-browsers)
unset PLAYWRIGHT_BROWSERS_PATH

/home/admin/MatchyMatchy/target/release/matchy \
  --old https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration \
  --new http://localhost:3001/products/connect/number-registration \
  --out /tmp/matchy-browser-test
```

## Observed

The Playwright "Looks like Playwright Test or Playwright was just installed or updated. /
Please run the following command to download new browsers: / npx playwright install" advisory
box is printed in full **three times** (old capture, new capture, retry attempt). The
terminal output then shows:

```
Both captures failed for viewport desktop:
  old: [CAPTURE_FAILED] browserType.launch: Executable doesn't exist at
    /home/admin/.cache/ms-playwright/chromium_headless_shell-1223/chrome-linux/chrome
new: [CAPTURE_FAILED] browserType.launch: Executable doesn't exist at
    /home/admin/.cache/ms-playwright/chromium_headless_shell-1223/chrome-linux/chrome
```

Running `matchy doctor` beforehand (also without `PLAYWRIGHT_BROWSERS_PATH` set) reports
`chromium: OK  build chromium-1217` — it finds build 1217 in the shared cache and passes,
without detecting that `packages/capture` requires build 1223.

## Expected

1. The browser-not-found error class should be caught and trigger **one** concise remedy
   message, naming the exact install command for the capture package's pinned version plus
   the `PLAYWRIGHT_BROWSERS_PATH` convention:

   ```
   Chromium build 1223 not found. To install:
     cd packages/capture && PLAYWRIGHT_BROWSERS_PATH=$PWD/../../.pw-browsers \
       npx playwright install chromium
   Then run matchy with:
     export PLAYWRIGHT_BROWSERS_PATH=/path/to/MatchyMatchy/.pw-browsers
   ```

2. `matchy doctor`'s `chromium` check should verify the **pinned build number** (readable
   from the doctor-mode response, which already returns `chromium.version`), not merely
   the presence of any build. A mismatch should report `FAIL` with the same remedy text.

## Evidence

- `docs/playwright-setup.md` (lines 9–13): Playwright 1.60.0 → Chromium build 1223;
  browsers in `.pw-browsers/` via `PLAYWRIGHT_BROWSERS_PATH`.
- `docs/playwright-setup.md` (lines 48–52): Makefile exports
  `PLAYWRIGHT_BROWSERS_PATH := $(CURDIR)/.pw-browsers`; running the bare binary requires
  `export PLAYWRIGHT_BROWSERS_PATH=$PWD/.pw-browsers` manually.
- `docs/playwright-setup.md` (lines 63–70): `AGENT_BROWSER_EXECUTABLE_PATH` points to
  `chromium-1217` in the shared cache; Matchy must not touch that cache.
- `packages/analyze/src/doctor.rs` `check_capture_doctor()` (lines 256–261): the chromium
  check reads `chromium.version` from the doctor-mode JSON and reports it, but the `ok` flag
  comes from `chromium.ok` in the capture response — which reports whether any installed
  Chromium satisfies Playwright's own search, not specifically build 1223.

## Suggested fix direction

- In `check_capture_doctor()`: after parsing the doctor response, compare
  `chromium.version` against the expected build number (hardcoded constant or read from
  `packages/capture/package.json`). Emit FAIL with a targeted remedy if they differ.
- Detect the `Executable doesn't exist` error class in the capture error output and short-
  circuit with one printed remedy block rather than propagating the raw Playwright banner
  once per capture attempt. The retry path should also suppress the repeated banner.
- Surface the `PLAYWRIGHT_BROWSERS_PATH` env var in both the doctor output and the
  browser-not-found remedy, since misuse of the shared cache is the most common root cause.
