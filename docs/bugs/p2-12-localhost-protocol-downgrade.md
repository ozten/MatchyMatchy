# Issue: `url_protocol_downgrade` is error-severity for `http://localhost` candidates; suggested remediation rewrites to `https://localhost` which is invalid for a dev server

**Status:** FIXED (2026-06-11 — see ROOT-CAUSE-AND-PLAN.md and docs/golden-changelog.md)
**Found:** 2026-06-11, during a real migration gate (Webflow staging page vs local Next.js port)
**Severity:** P2 — every dev/CI comparison against a localhost `new` URL raises four permanent error-severity false positives, degrades the report score, and requires a suppressions entry to silence; the suggested fix is semantically wrong for a local server
**Area:** `packages/analyze/src/hygiene.rs` — `check_protocol_downgrade_page()` and `check_per_link_protocol_downgrade()`

---

## Summary

The protocol-downgrade checks fire whenever the new page's URL or its same-site links use
`http://` while the old page used `https://`. The check is correct for production comparisons.
When the new URL is `http://localhost:3001/...` (a local Next.js dev server), it is
structurally impossible for the new side to use HTTPS without a self-signed certificate and
Playwright trust configuration. Every run therefore emits four `error`-severity
`url_protocol_downgrade` issues that are true but environmentally expected. The
`remediation.to` field in each issue recommends rewriting to
`https://localhost:3001/products/connect/...`, which is not a valid solution for a plain
`next dev` server. Suppressing these four issues requires a baseline entry on every project
that uses a localhost new URL.

There is no `--env dev` flag or localhost exemption documented anywhere in the codebase.

## Environment

- matchy 0.1.0 (d5f0713); Linux; node v24.15.0; Chrome Headless Shell 148.0.7778.96
  (pw chromium-headless-shell v1223)
- old=https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration
- new=http://localhost:3001/products/connect/number-registration

## Reproduction

```bash
/home/admin/MatchyMatchy/target/release/matchy \
  --old https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration \
  --new http://localhost:3001/products/connect/number-registration \
  --out /tmp/matchy-nr-1 --markdown
```

## Observed

Every run produces four `url_protocol_downgrade` issues at `error` severity. From
`/tmp/matchy-nr-1/diff-result.json`:

```json
{
  "type": "url_protocol_downgrade",
  "severity": "error",
  "goal": "G5",
  "message": "Per-link protocol downgrade: http://localhost:3001/products/connect/pricing should be https",
  "remediation": {
    "action": "rewrite_url",
    "findBy": { "grep": ["localhost:3001/products/connect/pricing"] },
    "from": "http://localhost:3001/products/connect/pricing",
    "to": "https://localhost:3001/products/connect/pricing",
    "note": "Link uses HTTP where old page had HTTPS for the same path. Update to HTTPS."
  }
}
```

```json
{
  "type": "url_protocol_downgrade",
  "severity": "error",
  "goal": "G5",
  "message": "Protocol downgrade: old=https://hiya-com-temp-43830deaa570809d11aa88b1f.webflow.io/products/connect/number-registration new=http://localhost:3001/products/connect/number-registration",
  "remediation": {
    "action": "rewrite_url",
    "findBy": { "grep": ["localhost:3001/products/connect/number-registration"] },
    "from": "http://localhost:3001/products/connect/number-registration",
    "to": "https://localhost:3001/products/connect/number-registration",
    "note": "New page uses HTTP where old page used HTTPS. Update to HTTPS."
  }
}
```

Both page-level issues (desktop and mobile) and both per-link issues appear in every run
regardless of what else changed. The `hygiene` score is penalised accordingly.
`https://localhost:3001/...` is not served by a standard `next dev` process; the suggested
fix is not actionable.

## Expected

For `http://localhost`, `http://127.0.0.1`, or `http://*.local` new URLs, the protocol-
downgrade check should either:

- Emit at `info` severity rather than `error`, with a note that the downgrade is expected
  for a local development server and does not need to be fixed; or
- Be suppressible via a new `--env dev` flag that downgrades all localhost-origin hygiene
  issues to `info`.

The `remediation.to` field for localhost candidates should not suggest `https://localhost`
(which requires Playwright to trust a self-signed cert). It should instead note that HTTPS
is not applicable for plain local dev servers and that the issue will not appear once the
new URL is a real HTTPS host.

## Evidence

Source: `packages/analyze/src/hygiene.rs`.

`check_protocol_downgrade_page()` (lines 462–533): fires when
`new_scheme == "http" && old_scheme == "https"`. No host exemption for `localhost` or
`127.0.0.1`. The `remediation.to` is built by a simple `replacen("http://", "https://", 1)`
on the new URL (line 475), producing `https://localhost:3001/...` for a local server.

`check_per_link_protocol_downgrade()` (lines 540–676): same severity, same `replacen`
logic (line 618).

The unit tests in `hygiene.rs` include a test confirming the check fires on
`http://example.com` vs `https://example.com` (line 1592–1599) but there are no tests
checking localhost behavior. There is no `localhost`-exemption branch anywhere in the
function.

From the real run evidence (`/tmp/matchy-nr-1/diff-result.json`): four issues, all
`severity: "error"`, each with `remediation.to` pointing to an `https://localhost:3001/...`
URL. The same four issues appear in every subsequent run (`/tmp/matchy-nr-2/`,
`/tmp/matchy-nr-10/`, etc.) unchanged.

## Suggested fix direction

- In `check_protocol_downgrade_page()` and `check_per_link_protocol_downgrade()`, after
  determining that the new URL uses `http://`, extract the host. If the host is `localhost`,
  `127.0.0.1`, `::1`, or any `.local` TLD, downgrade the emitted severity to `info` and
  replace the `remediation.note` with a message explaining that HTTP is expected for local
  dev servers.
- Alternatively, add an `--env dev` flag to `CaptureConfig` / the CLI. When set, all
  hygiene issues involving the new page's own origin (localhost or 127.0.0.1) that would
  be `error` severity for a production page are downgraded to `info`.
- Keep the check at `error` severity when the new host is a real (non-loopback)
  domain using HTTP — those are genuine production defects.
- Do not remove the check entirely; a future CI environment may run against an HTTPS-
  terminated staging server (`https://staging.example.com`) in which case the check
  must still fire.
