# Issue: flaky `network_error` on v08 from unvendored `srcset` variants

**Status:** Open — blocks `v0.1.0` release
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

- [ ] All 4 `-p-NNN` variants present on disk and return 200 on every variant server.
- [ ] `v08-cta-removed` fresh output = `{ missing_link }` only, matching the committed
      golden, across 5+ consecutive runs.
- [ ] `make verify` exits 0 (full suite, all variants).
- [ ] No golden expectation weakened to achieve the above.

## Follow-up (non-blocking)

Capture still requests different `srcset` candidates run-to-run. Once the assets 200
this is unobservable in output, but the underlying capture-stability question (should
capture normalize/pin `srcset`-driven requests, or should the network diff tolerate
`srcset` variance?) is a real determinism gap worth tracking for visual/network diffs
on other pages.
