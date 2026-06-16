"""
pair_privacy.py — Privacy/PII freeze gate for M9 Tier-3 pair fixtures.

Called by pair-add.py BEFORE any artifact is written into the tracked tree.
Pure logic functions are separated from interaction so tests require no real
stdin/stdout.

Credential/token scan (HARD, fail-closed):
  Scans every URL-bearing field of each bundle dict for secret-named query
  params with a non-sentinel, non-empty value.  A param named by SECRET_NAMES
  whose value is the redaction sentinel '…redacted…' is already safe and
  does NOT trip the gate.  This scan CANNOT be bypassed by assume_yes.

Human-review manifest (interactive, skippable by assume_yes):
  Surfaces external origins, captured-text length+sample, data:-URI count,
  console line count, and PNG file paths for human PII review, then requires
  confirmation (or skips if assume_yes).

Ownership/rights assertion (interactive, skippable by assume_yes):
  Requires the contributor to confirm that the captured content is owned by,
  or redistributable by, them and that it will be committed to a PUBLIC git
  repository permanently.

Size budget (non-fatal warning):
  If total fixture weight (bundle JSONs + PNG files) exceeds the budget,
  emits a WARNING and proceeds.

API:
  scan_credentials(bundle: dict) -> list[dict]
  collect_manifest(old_bundle: dict, new_bundle: dict, png_paths: list[Path]) -> dict
  total_weight_bytes(bundle_paths: list[Path], png_paths: list[Path]) -> int
  run_gate(*, old_bundle_path, new_bundle_path, png_paths, urls,
           assume_yes=False, size_budget_bytes=10*1024*1024,
           confirm=input, out=print) -> None

  class PrivacyGateError(Exception)
"""

from __future__ import annotations

import json
from pathlib import Path
from urllib.parse import parse_qsl, urlsplit


# ---------------------------------------------------------------------------
# Sentinel value — the capture layer's redaction placeholder
# ---------------------------------------------------------------------------

REDACTION_SENTINEL = "…redacted…"  # "…redacted…"

# ---------------------------------------------------------------------------
# Secret parameter names (case-insensitive)
#
# Mirrors capture/src/normalize.ts DEFAULT_REDACT_PARAMS plus obvious extras.
# These are CREDENTIAL/TOKEN shapes — NOT PII (different thing).
# ---------------------------------------------------------------------------

_CAPTURE_DEFAULT_REDACT = {
    "token",
    "sig",
    "signature",
    "key",
    "auth",
    "apikey",
    "access_token",
}

_EXTRA_SECRET_NAMES = {
    "password",
    "passwd",
    "pwd",
    "secret",
    "client_secret",
    "bearer",
    "jwt",
    "session",
    "sessionid",
    "sid",
    "api_key",
}

SECRET_NAMES: frozenset[str] = frozenset(
    name.lower() for name in (_CAPTURE_DEFAULT_REDACT | _EXTRA_SECRET_NAMES)
)


# ---------------------------------------------------------------------------
# Error class
# ---------------------------------------------------------------------------


class PrivacyGateError(Exception):
    """Raised when the privacy gate hard-fails (credential hit or user refusal)."""


# ---------------------------------------------------------------------------
# Pure helpers
# ---------------------------------------------------------------------------


def _is_credential_param(name: str, value: str) -> bool:
    """
    Return True if:
      - param name (case-insensitive) is in SECRET_NAMES, AND
      - the value is non-empty AND not the redaction sentinel.
    A sentinel value means capture.ts already redacted it — that is the safe,
    expected form and must NOT trip the gate.
    """
    if name.lower() not in SECRET_NAMES:
        return False
    if not value:
        return False
    if value == REDACTION_SENTINEL:
        return False
    return True


def _scan_url(url: str, field: str) -> list[dict]:
    """
    Parse query params from a URL string and return credential findings.
    Each finding: {"field": field, "url": url, "param": param_name}
    """
    findings: list[dict] = []
    try:
        parsed = urlsplit(url)
        for name, value in parse_qsl(parsed.query, keep_blank_values=True):
            if _is_credential_param(name, value):
                findings.append({"field": field, "url": url, "param": name})
    except Exception:
        # Malformed URLs: skip (can't redact what we can't parse)
        pass
    return findings


def scan_credentials(bundle: dict) -> list[dict]:
    """
    Scan all URL-bearing fields of a bundle dict for token/secret-shaped
    query parameters with non-sentinel, non-empty values.

    Returns a list of findings.  Empty list means no credential hits.

    Fields scanned (camelCase bundle keys):
      page.network.requests[].url
      page.redirectChain[]
      page.nodes[].src
      page.nodes[].rawHref
      page.linkProbes[].url
      page.linkProbes[].redirectChain[]
      page.linkProbes[].finalUrl

    NOTE: This enforces CREDENTIAL redaction — not PII.  The presence of
    a sentinel value (…redacted…) means capture.ts already handled it safely.
    There is no "redaction-ran" metadata flag in the bundle; this positive
    token-scan IS the enforcement mechanism.
    """
    findings: list[dict] = []
    page = bundle.get("page") or {}

    # page.network.requests[].url
    network = page.get("network") or {}
    for i, req in enumerate(network.get("requests") or []):
        url = req.get("url") or ""
        if url:
            findings.extend(_scan_url(url, f"page.network.requests[{i}].url"))

    # page.redirectChain[] (array of URL strings)
    for i, url in enumerate(page.get("redirectChain") or []):
        if url:
            findings.extend(_scan_url(url, f"page.redirectChain[{i}]"))

    # page.nodes[].src and page.nodes[].rawHref
    for i, node in enumerate(page.get("nodes") or []):
        src = node.get("src") or ""
        if src and not src.startswith("data:"):
            findings.extend(_scan_url(src, f"page.nodes[{i}].src"))
        raw_href = node.get("rawHref") or ""
        if raw_href:
            findings.extend(_scan_url(raw_href, f"page.nodes[{i}].rawHref"))

    # page.linkProbes[].url, .redirectChain[], .finalUrl
    for i, probe in enumerate(page.get("linkProbes") or []):
        probe_url = probe.get("url") or ""
        if probe_url:
            findings.extend(_scan_url(probe_url, f"page.linkProbes[{i}].url"))
        for j, rurl in enumerate(probe.get("redirectChain") or []):
            if rurl:
                findings.extend(
                    _scan_url(rurl, f"page.linkProbes[{i}].redirectChain[{j}]")
                )
        final_url = probe.get("finalUrl") or ""
        if final_url:
            findings.extend(_scan_url(final_url, f"page.linkProbes[{i}].finalUrl"))

    return findings


def _count_data_uris_in_bundle(bundle: dict) -> int:
    """
    Count inline data: URIs in:
      - page.nodes[].src values that start with "data:"
      - computedStyles values (any value starting with "data:")
    Content is NOT scanned — only counted (automated scanning is roadmapped).
    """
    count = 0
    page = bundle.get("page") or {}

    for node in page.get("nodes") or []:
        src = node.get("src") or ""
        if src.startswith("data:"):
            count += 1

    computed_styles = bundle.get("computedStyles") or {}
    for _node_id, props in computed_styles.items():
        if isinstance(props, dict):
            for _prop, val in props.items():
                if isinstance(val, str) and val.startswith("data:"):
                    count += 1

    return count


def _collect_external_origins(bundle: dict) -> list[str]:
    """
    Return sorted list of distinct external origins (scheme://host) seen in
    page.network.requests[].url.  Sorted for determinism.
    """
    origins: set[str] = set()
    page = bundle.get("page") or {}
    network = page.get("network") or {}
    for req in network.get("requests") or []:
        url = req.get("url") or ""
        if url:
            try:
                parsed = urlsplit(url)
                if parsed.scheme and parsed.netloc:
                    origins.add(f"{parsed.scheme}://{parsed.netloc}")
            except Exception:
                pass
    return sorted(origins)


def collect_manifest(
    old_bundle: dict,
    new_bundle: dict,
    png_paths: list[Path],
) -> dict:
    """
    Collect the human-review manifest for this pair.

    Returns a dict with:
      origins: list[str]         — distinct external origins from both bundles (sorted)
      textLength: int            — total captured-text character count across both bundles
      textSample: str            — first ~200 chars of concatenated node text
      dataUriCount: int          — total data: URI count across both bundles
      consoleLines: int          — total console message count across both bundles
      pngPaths: list[str]        — string paths of the committed screenshot PNGs

    No content is scanned — that's roadmapped.
    """
    origins_set: set[str] = set()
    for bundle in (old_bundle, new_bundle):
        for o in _collect_external_origins(bundle):
            origins_set.add(o)
    origins = sorted(origins_set)

    # Total captured-text length and sample
    all_texts: list[str] = []
    for bundle in (old_bundle, new_bundle):
        page = bundle.get("page") or {}
        for node in page.get("nodes") or []:
            text = node.get("text")
            if text:
                all_texts.append(text)
    combined_text = " ".join(all_texts)
    text_length = len(combined_text)
    text_sample = combined_text[:200]

    # data: URI count
    data_uri_count = (
        _count_data_uris_in_bundle(old_bundle)
        + _count_data_uris_in_bundle(new_bundle)
    )

    # Console line count
    console_lines = 0
    for bundle in (old_bundle, new_bundle):
        page = bundle.get("page") or {}
        console_lines += len(page.get("console") or [])

    return {
        "origins": origins,
        "textLength": text_length,
        "textSample": text_sample,
        "dataUriCount": data_uri_count,
        "consoleLines": console_lines,
        "pngPaths": [str(p) for p in png_paths],
    }


def total_weight_bytes(bundle_paths: list[Path], png_paths: list[Path]) -> int:
    """
    Sum the file sizes of the given bundle JSON files and PNG files.
    Files that do not exist contribute 0 (not an error here; gate logic handles that).
    """
    total = 0
    for p in bundle_paths:
        try:
            total += p.stat().st_size
        except FileNotFoundError:
            pass
    for p in png_paths:
        try:
            total += p.stat().st_size
        except FileNotFoundError:
            pass
    return total


# ---------------------------------------------------------------------------
# Top-level gate entry point
# ---------------------------------------------------------------------------


def run_gate(
    *,
    old_bundle_path: Path,
    new_bundle_path: Path,
    png_paths: list[Path],
    urls: list[str],
    assume_yes: bool = False,
    size_budget_bytes: int = 10 * 1024 * 1024,
    confirm=input,
    out=print,
) -> None:
    """
    Run the full privacy gate over the pair's TEMP artifacts BEFORE freeze.

    Raises PrivacyGateError on:
      - Any credential/token hit in either bundle (UNCONDITIONAL — cannot be
        bypassed by assume_yes).
      - User answers "n" / "no" at the human-review manifest prompt.
      - User answers "n" / "no" at the ownership/rights assertion prompt.

    Emits a WARNING (via out) but does NOT raise when total fixture weight
    exceeds size_budget_bytes.

    Parameters
    ----------
    old_bundle_path : Path
        Path to the OLD bundle JSON in the temp directory.
    new_bundle_path : Path
        Path to the NEW bundle JSON in the temp directory.
    png_paths : list[Path]
        Paths to all PNG files that will be committed (full-page + viewport shots).
    urls : list[str]
        The requested old/new URLs (for display only).
    assume_yes : bool
        If True, skips the interactive manifest + ownership prompts (trusted/
        already-reviewed re-freeze only).  Does NOT skip the credential scan.
    size_budget_bytes : int
        Per-fixture size budget in bytes.  Default: 10 MB.
    confirm : callable
        Injected prompt function — defaults to built-in input().  Override in
        tests to avoid real stdin.
    out : callable
        Injected output function — defaults to built-in print().  Override in
        tests to capture warnings.
    """
    # --- 1. Load bundles ---
    try:
        old_bundle = json.loads(old_bundle_path.read_text(encoding="utf-8"))
    except Exception as exc:
        raise PrivacyGateError(f"Cannot load old bundle: {exc}") from exc

    try:
        new_bundle = json.loads(new_bundle_path.read_text(encoding="utf-8"))
    except Exception as exc:
        raise PrivacyGateError(f"Cannot load new bundle: {exc}") from exc

    # --- 2. Credential scan (HARD — not bypassable by assume_yes) ---
    all_findings: list[dict] = []
    for label, bundle in [("old", old_bundle), ("new", new_bundle)]:
        for finding in scan_credentials(bundle):
            all_findings.append({**finding, "bundle": label})

    if all_findings:
        lines = [
            "CREDENTIAL GATE FAILURE — secret-named query parameters found in bundle URLs.",
            "",
            "This gate enforces CREDENTIAL redaction via a positive token-scan.",
            "The capture layer (capture.ts redactUrl) should have replaced these with",
            f"the sentinel '{REDACTION_SENTINEL}' already.  Finding non-sentinel values",
            "means redaction did NOT fire on these URLs — this is a bug, not a missing",
            "configuration step.  The gate is UNCONDITIONAL; --yes cannot bypass it.",
            "",
            "Findings:",
        ]
        for f in all_findings:
            lines.append(
                f"  [{f['bundle']}] field={f['field']}  param={f['param']}  url={f['url'][:120]}"
            )
        raise PrivacyGateError("\n".join(lines))

    # --- 3. Human-review manifest (interactive, skippable by assume_yes) ---
    manifest = collect_manifest(old_bundle, new_bundle, png_paths)

    if not assume_yes:
        out("")
        out("=" * 70)
        out("HUMAN PII REVIEW REQUIRED")
        out("=" * 70)
        out(f"URLs being frozen: {urls}")
        out("")
        out("External origins seen in network requests:")
        if manifest["origins"]:
            for o in manifest["origins"]:
                out(f"  {o}")
        else:
            out("  (none)")
        out("")
        out(f"Total captured-text length : {manifest['textLength']} chars")
        out(f"Text sample (first 200 chars):")
        out(f"  {manifest['textSample']!r}")
        out("")
        out(f"Inline data: URI count     : {manifest['dataUriCount']}")
        out("  (content NOT scanned — automated scanning is roadmapped)")
        out(f"Browser console line count : {manifest['consoleLines']}")
        out("")
        out("Screenshot PNGs to be committed (may show visible PII):")
        if manifest["pngPaths"]:
            for p in manifest["pngPaths"]:
                out(f"  {p}")
        else:
            out("  (none)")
        out("")
        out(
            "Review the above.  PII in visible page content / screenshots is a "
            "PERMANENT commit to a PUBLIC git repository."
        )
        answer = confirm("Does the captured content contain NO personal data? [y/N] ").strip().lower()
        if answer not in ("y", "yes"):
            raise PrivacyGateError(
                "Human PII review refused — nothing will be frozen.  "
                "Remove PII from captured content before retrying."
            )

    # --- 4. Ownership / redistribution-rights assertion (interactive, skippable) ---
    if not assume_yes:
        out("")
        out("=" * 70)
        out("OWNERSHIP / REDISTRIBUTION-RIGHTS ASSERTION")
        out("=" * 70)
        out("")
        out(
            "By proceeding, you assert that ALL captured page content at the "
            "URLs listed above is OWNED BY, or REDISTRIBUTABLE BY, you (the "
            "contributor), and that you consent to committing this content "
            "PERMANENTLY to a PUBLIC git repository where it will be visible "
            "to the public, archived, and indexed."
        )
        out("")
        out(
            "If the captured page belongs to a third party, you must have "
            "explicit redistribution rights (e.g. CC license, API terms that "
            "permit this, or written permission) before proceeding."
        )
        out("")
        answer = confirm(
            "I confirm I own or have redistribution rights to this content [y/N] "
        ).strip().lower()
        if answer not in ("y", "yes"):
            raise PrivacyGateError(
                "Ownership/redistribution assertion refused — nothing will be frozen.  "
                "Ensure you own or have rights to the captured content before retrying."
            )

    # --- 5. Size budget (non-fatal warning) ---
    weight = total_weight_bytes(
        [old_bundle_path, new_bundle_path],
        png_paths,
    )
    if weight > size_budget_bytes:
        budget_mb = size_budget_bytes / (1024 * 1024)
        actual_mb = weight / (1024 * 1024)
        out(
            f"WARNING: total fixture weight {actual_mb:.1f} MB exceeds budget "
            f"{budget_mb:.1f} MB.  Consider using --maxTextLength, "
            f"--probeLinks false, or a smaller viewport to reduce size."
        )
