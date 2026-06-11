# Issue: flaky `network_error` on v08 from unvendored `srcset` variants

**Status:** RESOLVED 2026-06-11 — fix plan executed; `make verify` green; see resolution below.
**Found:** 2026-06-11, during pre-release `make verify`
**Severity:** Release blocker (testbed defect, not a product defect)
**Area:** testbed fixtures / golden suite

---

## Summary

`make verify` intermittently fails the `v08-cta-removed` golden comparison: the fresh
run emits a `network_error` (`issue_7908880566e5`) that the committed golden and the
variant's intent do not contain. The `network_error` is a **404 on a responsive-image
`srcset` size variant that was never vendored into the testbed**. It is **flaky**, not
constant, because browser `srcset` candidate selection is timing-sensitive.

The analyze code is correct — a new-only 404 *is* a real defect signal. The bug is in
the **testbed**: incomplete asset vendoring violates our invariant that images are
vendored locally and that a variant introduces only its one declared change.

## Symptom

Fresh `v08-cta-removed` output:

```
issues: missing_link (expected)  +  network_error (spurious)
network_error evidence:
  new: { status: 404, failed: false, type: image,
         url: .../Case Studies_BCLC_hero image-p-500.webp }
  old: null
```

## Evidence it is intermittent (not constant)

Five back-to-back fresh captures of v08 produced **0 `network_error` every time**; the
failing run was observed once under different load timing. The `-p-500` candidate is
requested only on some runs.

```
run 1: 0 network_error
run 2: 0 network_error
run 3: 0 network_error
run 4: 0 network_error
run 5: 0 network_error
```

## Root cause

1. The testbed vendored only the **base** images, never their Webflow `srcset` size
   variants. **All 4 referenced `-p-NNN` variants are missing from disk** and 404:

   ```
   67caf62e..._A-LIGN_ISO-27001-p-500.webp
   691ca7f9..._Case Studies_BCLC_hero image-p-500.webp
   691ca7f9..._Case Studies_BCLC_hero image-p-800.webp
   691ca7f9..._Case Studies_BCLC_hero image-p-1080.webp
   ```

2. v08 deletes the hero's secondary CTA. Its declared knock-on — "hero button-group
   collapses to a single button" — shifts the hero layout, which can move the hero
   image across a `srcset` breakpoint. When that happens the browser requests the
   `-p-500` candidate instead of the base image.

3. `srcset` selection during page load is **timing-sensitive**, so the `-p-500`
   request fires only on some captures → intermittent 404 → intermittent `network_error`.

## Why it was not discovered earlier

- `network_error` detection landed in **M7** (`cf574b4`). v08's golden was recorded in
  **M3** (`9c0a589`) and last touched in **M6** (`bd795bd`) — both *before* the detector
  existed, so the golden has no baseline for this request.
- The flake is low-probability, so the occasional full `make verify` between M7 and now
  usually passed.
- Early-milestone goldens were not re-verified after M7/M8 added new detectors.

A detector added after the golden was frozen, firing intermittently → invisible until
an unlucky full-verify run.

## Decision: do NOT add test retries

Retrying `make verify` until it goes green was considered and **rejected**:

- It masks the cause; the 404 asset remains genuinely missing.
- Worse, it hides **capture nondeterminism** — the exact property matchy exists to
  guarantee (spec §3.3 / §15: identical bundles → byte-identical output). A
  retry-until-green loop selects the runs that agree with the golden and discards the
  ones that don't.
- It is unnecessary: the flake has a known, fixable root cause.

We also will **not** re-record v08's golden to bless the `network_error`. Per the golden
discipline (CLAUDE.md), silencing a false positive caused by a testbed defect by editing
the expectation is forbidden.

## Fix plan

1. **Vendor the 4 missing `srcset` variants** into
   `testbed/golden/site/assets/images/`, fetched from the original Webflow source.
   Fallback if the source is unreachable: copy the corresponding base image to the
   variant filename — the network check only inspects HTTP status, and the variants
   render at the same CSS size, so visual impact is nil. (Delegate to `fixture-builder`.)
   - Once present, both the base and `-p-NNN` requests return **200**, so `network_error`
     (which fires only on `status >= 400`) can never trigger on any run, with zero retries.

2. **Verify no remaining 404s** across the golden server and every variant server
   (`testbed/run-all.py` + a probe of each referenced asset).

3. **Re-run the full golden suite**, not just v08. The earlier `make verify` stopped at
   the first failure, so other early-milestone goldens may also be stale vs M7/M8 output.
   Triage each mismatch as real-bug-vs-legit-rerecord — never blanket-bless. Any
   intentional re-record gets a `docs/golden-changelog.md` entry + `golden-auditor`
   APPROVE.

4. **Then** cut `v0.1.0` (tag `v*` → release workflow).

## Acceptance criteria

- [x] All 4 `-p-NNN` variants present on disk and return 200 on every variant server.
      (Present in all 22 site dirs; serve 200 wherever the page actually requests them — root path
      for normal variants, prefix path for the locale variants v15/v16; v18 returns 404 for *all*
      paths by design, which is its own tested violation.)
- [x] `v08-cta-removed` fresh output = `{ missing_link }` only, matching the committed
      golden, across 5+ consecutive runs. (Verified 5/5: `network_error` = 0 every run.)
- [x] `make verify` exits 0 (full suite, all 21 variants + M8 + goldens + determinism).
- [x] No golden expectation weakened. (Only v04 needed re-recording; the golden-auditor APPROVED it
      after confirming the delta is purely alt-text-artifact removal with zero font-family/G1 loss.)

## Resolution (2026-06-11)

Fix-plan step 1 executed: the four `-p-NNN.webp` variants were vendored as byte copies of their base
images into all 22 site `assets/images/` dirs (88 files). Encoding nuance: the BCLC base is stored
on disk with a literal `%20` in its name, but the server URL-decodes request paths, so the three BCLC
variants were created with **real spaces** in their filenames (the form the decoded request resolves
to). With every `srcset` candidate now returning 200, the new-only 404 that produced `network_error`
can no longer occur — the flake is eliminated structurally, not probabilistically.

Re-running the full golden suite (fix-plan step 3), **only `v04-font-family`** required a re-record:
its golden had been recorded while the broken hero rendered alt text, which v04's font-family swap
contaminated into 28 spurious `visual_region_changed` + 1 `page_height_changed` artifacts. The
re-record (golden-auditor APPROVE) removes exactly those artifacts with no change to the 454
font-family `style_changed` detections. All other 20 goldens were byte-identical, so the repair did
not silently mutate them. Full record: `docs/golden-changelog.md` (2026-06-11 srcset entry).

The non-blocking follow-up below (capture requesting different `srcset` candidates run-to-run) is now
unobservable in output because all candidates serve identical base bytes; it remains a real
capture-determinism question worth tracking but no longer affects the testbed.

## Follow-up (non-blocking)

Capture still requests different `srcset` candidates run-to-run. Once the assets 200
this is unobservable in output, but the underlying capture-stability question (should
capture normalize/pin `srcset`-driven requests, or should the network diff tolerate
`srcset` variance?) is a real determinism gap worth tracking for visual/network diffs
on other pages.
