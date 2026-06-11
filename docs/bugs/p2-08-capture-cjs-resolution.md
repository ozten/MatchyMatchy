# Issue: `capture.cjs` not found when the binary runs outside its repo

**Status:** OPEN
**Found:** 2026-06-11, during a real migration gate (Webflow staging page vs local Next.js port)
**Severity:** P2 — any invocation of the installed binary from a directory other than the repo root fails immediately before capturing a single page
**Area:** `packages/analyze/src/orchestrate.rs` — `resolve_capture_script()`

---

## Summary

`resolve_capture_script()` (orchestrate.rs) tries three locations for `capture.cjs` in a
fixed order. The middle candidate — a sibling of the current executable — is never populated
by the Cargo build layout, and the fallback CWD-relative path only works when the binary is
invoked from the repo root. There is no ancestor walk. Running the installed binary from any
other directory produces an immediate fatal error even though `capture.cjs` exists on disk.

The README's install-script section (line 37) documents installing Playwright globally so
that `capture.cjs` can resolve it, and the design spec (M1.md §5.5) documents the three-step
resolution order and mentions `MATCHY_CAPTURE_PATH` as the override. `matchy doctor`'s
build-capture remediation text is good. The workaround is functional but not discoverable
from the error message alone.

## Environment

- matchy 0.1.0 (d5f0713); Linux; node v24.15.0; Chrome Headless Shell 148.0.7778.96
  (pw chromium-headless-shell v1223)
- old=https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration
- new=http://localhost:3001/products/connect/number-registration

## Reproduction

```bash
# capture.cjs is present at:
ls /home/admin/MatchyMatchy/packages/capture/dist/capture.cjs   # exists

# run the binary from a different directory:
cd /tmp
/home/admin/MatchyMatchy/target/release/matchy \
  --old https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration \
  --new http://localhost:3001/products/connect/number-registration \
  --out /tmp/matchy-out
```

## Observed

```
error: capture.cjs not found. Set MATCHY_CAPTURE_PATH or ensure
packages/capture/dist/capture.cjs exists.
Searched:
  MATCHY_CAPTURE_PATH (not set or not found)
  sibling of binary
  /tmp/packages/capture/dist/capture.cjs
```

(The third candidate resolves against CWD `/tmp`, not the binary's location.)

## Expected

Either the binary finds `capture.cjs` by walking ancestors of its own path, or the error
message names a concrete absolute path to set in `MATCHY_CAPTURE_PATH` and explains that
the workaround is to export that variable rather than to cd to the repo root.

## Evidence

Source: `packages/analyze/src/orchestrate.rs`, `resolve_capture_script()` (lines 18–48).
The three candidates, as implemented:

1. `MATCHY_CAPTURE_PATH` env var (line 20–25).
2. `current_exe().parent().join("capture.cjs")` — i.e. `target/release/capture.cjs` (lines 27–34).
   This path is never created by `cargo build`; the Cargo layout puts only the binary in
   `target/release/`, not any JS artifacts.
3. `current_dir().join("packages/capture/dist/capture.cjs")` — CWD-relative (lines 37–41).
   Works only when CWD is the repo root.

No ancestor walk of `current_exe()` is attempted. The Makefile sets `PLAYWRIGHT_BROWSERS_PATH`
automatically for every `make` target, but does not export `MATCHY_CAPTURE_PATH`, so bare
invocations of the binary outside `make` fall through to the CWD check.

The `bail!` message (lines 43–48) correctly names `MATCHY_CAPTURE_PATH` as a remedy but
does not supply the absolute path the user needs to set.

## Suggested fix direction

Two complementary fixes:

1. **Ancestor walk:** after the `current_exe()` sibling check fails, walk up the ancestor
   chain of the binary's directory looking for `packages/capture/dist/capture.cjs`. This
   covers the common case of `target/release/matchy` inside the repo tree without any env
   variable.

2. **Improve the error message:** when the bail fires, print the absolute path that
   `MATCHY_CAPTURE_PATH` needs to be set to (i.e. the path that would be correct if it
   existed), so the user can export it directly. Example:
   `export MATCHY_CAPTURE_PATH=/home/admin/MatchyMatchy/packages/capture/dist/capture.cjs`

A build-time embed of the absolute path (via `env!` macro + `build.rs`) is a third option
for release artifacts, but the ancestor walk covers the development case cleanly without
requiring a build script.
