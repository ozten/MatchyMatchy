"""
Tests for testbed/pair_privacy.py  (Unit U6).

Runnable as:
  python3 testbed/tests/test_pair_privacy.py      (standalone, exits non-zero on failure)
  pytest testbed/tests/test_pair_privacy.py       (pytest discovery)

All tests use only the Python stdlib (plus pair_privacy itself).
No real stdin/stdout is needed — confirm and out are injected.
"""

from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path

# ---------------------------------------------------------------------------
# Import the module under test
# ---------------------------------------------------------------------------

_TESTS_DIR = Path(__file__).resolve().parent
_TESTBED_DIR = _TESTS_DIR.parent
sys.path.insert(0, str(_TESTBED_DIR))

from pair_privacy import (  # noqa: E402
    PrivacyGateError,
    REDACTION_SENTINEL,
    SECRET_NAMES,
    collect_manifest,
    run_gate,
    scan_credentials,
    total_weight_bytes,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _bundle(
    *,
    network_urls: list[str] | None = None,
    redirect_chain: list[str] | None = None,
    nodes: list[dict] | None = None,
    link_probes: list[dict] | None = None,
    computed_styles: dict | None = None,
    console: list[dict] | None = None,
) -> dict:
    """Build a minimal synthetic bundle dict."""
    return {
        "page": {
            "url": "https://example.com/",
            "finalUrl": "https://example.com/",
            "redirectChain": redirect_chain or [],
            "statusCode": 200,
            "title": "Example",
            "metaDescription": "",
            "canonical": None,
            "lang": "en",
            "pageHeight": 1000,
            "nodes": nodes or [],
            "landmarks": [],
            "network": {
                "requests": [{"url": u, "status": 200, "type": "document", "failed": False}
                              for u in (network_urls or [])]
            },
            "console": console or [],
            "a11y": {"violations": []},
            "linkProbes": link_probes or [],
        },
        "computedStyles": computed_styles or {},
    }


def _write_bundle(path: Path, bundle: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(bundle), encoding="utf-8")


def _yes_confirm(_prompt: str) -> str:
    return "y"


def _no_confirm(_prompt: str) -> str:
    return "n"


def _silent_out(_msg: str) -> None:
    pass


# ---------------------------------------------------------------------------
# Test results collector
# ---------------------------------------------------------------------------

_RESULTS: list[tuple[str, bool, str]] = []


def _test(name: str, fn) -> None:
    try:
        fn()
        _RESULTS.append((name, True, ""))
        print(f"  PASS  {name}")
    except AssertionError as exc:
        _RESULTS.append((name, False, str(exc)))
        print(f"  FAIL  {name}: {exc}")
    except Exception as exc:
        _RESULTS.append((name, False, f"UNEXPECTED ERROR: {type(exc).__name__}: {exc}"))
        print(f"  FAIL  {name}: UNEXPECTED ERROR: {type(exc).__name__}: {exc}")


# ===========================================================================
# CHARACTERIZATION TESTS — token-bearing bundle fails first
# (build the token-bearing fixture and assert failure BEFORE other tests)
# ===========================================================================


# ---------------------------------------------------------------------------
# C1. access_token in network.requests[].url — scan_credentials finds it
# ---------------------------------------------------------------------------

def test_credential_in_network_url_scan():
    """?access_token=abc123 in network.requests[].url is reported by scan_credentials."""
    bundle = _bundle(network_urls=["https://cdn.example.com/file?access_token=abc123"])
    findings = scan_credentials(bundle)
    assert len(findings) >= 1, f"Expected credential hit, got: {findings}"
    assert any(f["param"] == "access_token" for f in findings), f"Findings: {findings}"
    assert any("network.requests" in f["field"] for f in findings), f"Findings: {findings}"


def test_credential_in_network_url_run_gate():
    """run_gate raises PrivacyGateError for access_token in network URL."""
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        bundle = _bundle(network_urls=["https://cdn.example.com/file?access_token=abc123"])
        old_path = td / "old.bundle.json"
        new_path = td / "new.bundle.json"
        _write_bundle(old_path, bundle)
        _write_bundle(new_path, _bundle())  # clean new bundle

        try:
            run_gate(
                old_bundle_path=old_path,
                new_bundle_path=new_path,
                png_paths=[],
                urls=["https://example.com/old", "https://example.com/new"],
                assume_yes=True,  # even assume_yes must not bypass credential scan
                confirm=_yes_confirm,
                out=_silent_out,
            )
            assert False, "Expected PrivacyGateError to be raised"
        except PrivacyGateError as exc:
            msg = str(exc)
            assert "CREDENTIAL" in msg or "credential" in msg.lower(), (
                f"Error message should mention credential redaction, got: {msg}"
            )


# ---------------------------------------------------------------------------
# C2. access_token in page.nodes[].src — scan_credentials finds it
# ---------------------------------------------------------------------------

def test_credential_in_node_src_scan():
    """?access_token=abc123 in page.nodes[].src is reported by scan_credentials."""
    nodes = [{"src": "https://img.example.com/photo?access_token=abc123", "text": None}]
    bundle = _bundle(nodes=nodes)
    findings = scan_credentials(bundle)
    assert len(findings) >= 1, f"Expected credential hit, got: {findings}"
    assert any(f["param"] == "access_token" for f in findings), f"Findings: {findings}"
    assert any("nodes" in f["field"] and ".src" in f["field"] for f in findings), (
        f"Expected field to reference nodes[].src, got: {findings}"
    )


def test_credential_in_node_src_run_gate():
    """run_gate raises PrivacyGateError for access_token in node.src."""
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        nodes = [{"src": "https://img.example.com/photo?access_token=abc123", "text": None}]
        bundle = _bundle(nodes=nodes)
        old_path = td / "old.bundle.json"
        new_path = td / "new.bundle.json"
        _write_bundle(old_path, bundle)
        _write_bundle(new_path, _bundle())

        try:
            run_gate(
                old_bundle_path=old_path,
                new_bundle_path=new_path,
                png_paths=[],
                urls=["https://example.com/old", "https://example.com/new"],
                assume_yes=True,
                confirm=_yes_confirm,
                out=_silent_out,
            )
            assert False, "Expected PrivacyGateError to be raised"
        except PrivacyGateError as exc:
            assert "CREDENTIAL" in str(exc) or "credential" in str(exc).lower()


# ---------------------------------------------------------------------------
# C3. access_token in linkProbes[].url — scan_credentials finds it
# ---------------------------------------------------------------------------

def test_credential_in_link_probe_url_scan():
    """?access_token=abc123 in linkProbes[].url is reported by scan_credentials."""
    probes = [
        {
            "url": "https://example.com/protected?access_token=abc123",
            "redirectChain": [],
            "finalUrl": "https://example.com/protected",
        }
    ]
    bundle = _bundle(link_probes=probes)
    findings = scan_credentials(bundle)
    assert len(findings) >= 1, f"Expected credential hit, got: {findings}"
    assert any(f["param"] == "access_token" for f in findings), f"Findings: {findings}"
    assert any("linkProbes" in f["field"] and ".url" in f["field"] for f in findings), (
        f"Expected field to reference linkProbes[].url, got: {findings}"
    )


def test_credential_in_link_probe_url_run_gate():
    """run_gate raises PrivacyGateError for access_token in linkProbes[].url."""
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        probes = [
            {
                "url": "https://example.com/protected?access_token=abc123",
                "redirectChain": [],
                "finalUrl": "https://example.com/protected",
            }
        ]
        bundle = _bundle(link_probes=probes)
        old_path = td / "old.bundle.json"
        new_path = td / "new.bundle.json"
        _write_bundle(old_path, bundle)
        _write_bundle(new_path, _bundle())

        try:
            run_gate(
                old_bundle_path=old_path,
                new_bundle_path=new_path,
                png_paths=[],
                urls=["https://example.com/old", "https://example.com/new"],
                assume_yes=True,
                confirm=_yes_confirm,
                out=_silent_out,
            )
            assert False, "Expected PrivacyGateError to be raised"
        except PrivacyGateError as exc:
            assert "CREDENTIAL" in str(exc) or "credential" in str(exc).lower()


# ===========================================================================
# SENTINEL TESTS — already-redacted values must NOT trip the gate
# ===========================================================================


def test_sentinel_in_network_url_no_hit():
    """token=…redacted… (the sentinel) in network URL is NOT a credential hit."""
    bundle = _bundle(network_urls=[f"https://cdn.example.com/file?token={REDACTION_SENTINEL}"])
    findings = scan_credentials(bundle)
    assert findings == [], f"Sentinel value should not be a hit, got: {findings}"


def test_sentinel_in_node_src_no_hit():
    """access_token=…redacted… in node.src is NOT a credential hit."""
    nodes = [{"src": f"https://img.example.com/photo?access_token={REDACTION_SENTINEL}", "text": None}]
    bundle = _bundle(nodes=nodes)
    findings = scan_credentials(bundle)
    assert findings == [], f"Sentinel value should not be a hit, got: {findings}"


def test_sentinel_in_link_probe_no_hit():
    """sig=…redacted… in linkProbes[].url is NOT a credential hit."""
    probes = [
        {
            "url": f"https://example.com/file?sig={REDACTION_SENTINEL}",
            "redirectChain": [],
            "finalUrl": "https://example.com/file",
        }
    ]
    bundle = _bundle(link_probes=probes)
    findings = scan_credentials(bundle)
    assert findings == [], f"Sentinel value should not be a hit, got: {findings}"


# ===========================================================================
# ASSUME_YES CANNOT BYPASS CREDENTIAL SCAN
# ===========================================================================


def test_assume_yes_does_not_bypass_credential_scan():
    """assume_yes=True with a token-bearing bundle STILL raises PrivacyGateError."""
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        # Token in the NEW bundle
        bundle = _bundle(network_urls=["https://api.example.com/data?session=mysecrettoken"])
        old_path = td / "old.bundle.json"
        new_path = td / "new.bundle.json"
        _write_bundle(old_path, _bundle())   # clean old
        _write_bundle(new_path, bundle)       # token in new

        raised = False
        try:
            run_gate(
                old_bundle_path=old_path,
                new_bundle_path=new_path,
                png_paths=[],
                urls=["https://example.com/old", "https://example.com/new"],
                assume_yes=True,  # MUST still fail
                confirm=_yes_confirm,
                out=_silent_out,
            )
        except PrivacyGateError:
            raised = True

        assert raised, (
            "assume_yes=True must NOT bypass the credential scan — "
            "PrivacyGateError should have been raised."
        )


# ===========================================================================
# CLEAN BUNDLE — gate passes; manifest is populated correctly
# ===========================================================================


def test_clean_bundle_gate_passes():
    """
    Clean bundle (no secret params) → run_gate does NOT raise.
    collect_manifest reports origins, text length+sample, data:-URI count,
    console line count, and PNG paths.
    """
    # Build a bundle with observable content
    nodes = [
        {"text": "Hello world", "src": None, "rawHref": None},
        {"text": "Second paragraph", "src": "https://img.example.com/photo.jpg", "rawHref": None},
    ]
    console = [
        {"level": "log", "text": "Loaded"},
        {"level": "warn", "text": "Slow resource"},
    ]
    network_urls = [
        "https://cdn.example.com/styles.css",
        "https://cdn.example.com/script.js",
        "https://other.example.org/api",
    ]
    bundle = _bundle(
        network_urls=network_urls,
        nodes=nodes,
        console=console,
    )

    # collect_manifest
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        fake_png = td / "old.png"
        fake_png.write_bytes(b"\x89PNG")
        manifest = collect_manifest(bundle, bundle, [fake_png])

    assert len(manifest["origins"]) >= 2, f"Expected external origins, got: {manifest['origins']}"
    assert "https://cdn.example.com" in manifest["origins"]
    assert manifest["textLength"] > 0, "textLength should be > 0"
    assert len(manifest["textSample"]) <= 200
    assert "Hello world" in manifest["textSample"] or manifest["textLength"] > 200
    assert manifest["dataUriCount"] == 0
    assert manifest["consoleLines"] == 4  # 2 per bundle × 2 bundles
    assert str(fake_png) in manifest["pngPaths"]

    # run_gate does not raise
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        old_path = td / "old.bundle.json"
        new_path = td / "new.bundle.json"
        _write_bundle(old_path, bundle)
        _write_bundle(new_path, bundle)

        warnings_emitted: list[str] = []
        run_gate(
            old_bundle_path=old_path,
            new_bundle_path=new_path,
            png_paths=[],
            urls=["https://example.com/old", "https://example.com/new"],
            assume_yes=False,
            confirm=_yes_confirm,
            out=lambda msg: warnings_emitted.append(msg),
        )
        # No PrivacyGateError raised; that's the assertion (reaching here is the pass).


# ===========================================================================
# ASSUME_YES + CLEAN BUNDLE — confirm is NOT called
# ===========================================================================


def test_assume_yes_clean_bundle_no_confirm_call():
    """
    assume_yes=True with a clean bundle + a fake confirm that would say 'n'
    does NOT call confirm and does NOT raise.
    """
    confirm_called = [False]

    def bad_confirm(_prompt: str) -> str:
        confirm_called[0] = True
        return "n"

    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        bundle = _bundle()
        old_path = td / "old.bundle.json"
        new_path = td / "new.bundle.json"
        _write_bundle(old_path, bundle)
        _write_bundle(new_path, bundle)

        run_gate(
            old_bundle_path=old_path,
            new_bundle_path=new_path,
            png_paths=[],
            urls=["https://example.com/old", "https://example.com/new"],
            assume_yes=True,
            confirm=bad_confirm,
            out=_silent_out,
        )

    assert not confirm_called[0], (
        "assume_yes=True must skip the interactive prompts — confirm must not be called."
    )


# ===========================================================================
# ASSUME_YES=FALSE + "n" AT OWNERSHIP PROMPT → PrivacyGateError
# ===========================================================================


def test_no_at_ownership_prompt_raises():
    """
    assume_yes=False + a fake confirm returning 'n' at the ownership prompt
    raises PrivacyGateError (refused).
    """
    # We need to answer "y" for the PII review and "n" for the ownership assertion.
    call_count = [0]

    def mixed_confirm(_prompt: str) -> str:
        call_count[0] += 1
        if call_count[0] == 1:
            return "y"   # PII review: approve
        return "n"        # ownership: refuse

    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        bundle = _bundle()
        old_path = td / "old.bundle.json"
        new_path = td / "new.bundle.json"
        _write_bundle(old_path, bundle)
        _write_bundle(new_path, bundle)

        try:
            run_gate(
                old_bundle_path=old_path,
                new_bundle_path=new_path,
                png_paths=[],
                urls=["https://example.com/old", "https://example.com/new"],
                assume_yes=False,
                confirm=mixed_confirm,
                out=_silent_out,
            )
            assert False, "Expected PrivacyGateError (ownership refused)"
        except PrivacyGateError as exc:
            msg = str(exc).lower()
            assert "ownership" in msg or "redistribution" in msg or "refused" in msg, (
                f"Error should mention ownership/redistribution or refusal, got: {exc}"
            )


# ===========================================================================
# ASSUME_YES=FALSE + "n" AT PII REVIEW PROMPT → PrivacyGateError
# ===========================================================================


def test_no_at_pii_review_prompt_raises():
    """
    assume_yes=False + 'n' at the PII review prompt raises PrivacyGateError.
    """
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        bundle = _bundle()
        old_path = td / "old.bundle.json"
        new_path = td / "new.bundle.json"
        _write_bundle(old_path, bundle)
        _write_bundle(new_path, bundle)

        try:
            run_gate(
                old_bundle_path=old_path,
                new_bundle_path=new_path,
                png_paths=[],
                urls=["https://example.com/old", "https://example.com/new"],
                assume_yes=False,
                confirm=_no_confirm,
                out=_silent_out,
            )
            assert False, "Expected PrivacyGateError (PII review refused)"
        except PrivacyGateError as exc:
            msg = str(exc).lower()
            assert "pii" in msg or "personal" in msg or "refused" in msg, (
                f"Error should mention PII or refusal, got: {exc}"
            )


# ===========================================================================
# SIZE BUDGET — non-fatal warning, gate still completes
# ===========================================================================


def test_size_budget_warning_emitted_non_fatal():
    """
    Total weight over the budget → a WARNING is emitted (captured via out)
    and run_gate still completes for an otherwise-clean bundle (non-fatal).
    """
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        bundle = _bundle()

        old_path = td / "old.bundle.json"
        new_path = td / "new.bundle.json"
        # Make the bundle files large (> budget)
        large_content = json.dumps(bundle) + " " * (5 * 1024 * 1024)
        old_path.write_text(large_content, encoding="utf-8")
        new_path.write_text(large_content, encoding="utf-8")

        # Also create a large fake PNG
        fake_png = td / "old.png"
        fake_png.write_bytes(b"\x89PNG" + b"\x00" * (2 * 1024 * 1024))

        warnings_emitted: list[str] = []

        run_gate(
            old_bundle_path=old_path,
            new_bundle_path=new_path,
            png_paths=[fake_png],
            urls=["https://example.com/old", "https://example.com/new"],
            assume_yes=True,
            confirm=_yes_confirm,
            out=lambda msg: warnings_emitted.append(msg),
            size_budget_bytes=1 * 1024 * 1024,  # 1 MB budget — will be exceeded
        )

        # Check a warning was emitted
        combined = " ".join(warnings_emitted)
        assert "WARNING" in combined or "warning" in combined.lower(), (
            f"Expected a size warning to be emitted, got: {warnings_emitted}"
        )
        # No exception raised — non-fatal


# ===========================================================================
# DATA: URI COUNTING
# ===========================================================================


def test_data_uri_counting_in_manifest():
    """
    data: URIs in node.src and in computedStyles values are counted correctly.
    Content is NOT scanned — only counted.
    """
    nodes = [
        {"src": "data:image/png;base64,abc123", "text": None},
        {"src": "https://example.com/real.png", "text": "Normal image"},
        {"src": "data:image/svg+xml,<svg/>", "text": None},
    ]
    computed_styles = {
        "node_1": {
            "background-image": "data:image/png;base64,xyz",
            "color": "red",
        },
        "node_2": {
            "content": "data:text/plain,hello",
        },
    }
    bundle = _bundle(nodes=nodes, computed_styles=computed_styles)
    # old and new both get this bundle — so counts are doubled
    manifest = collect_manifest(bundle, bundle, [])

    # old: 2 nodes + 2 computedStyles = 4; new: same = 4; total = 8
    assert manifest["dataUriCount"] == 8, (
        f"Expected 8 data: URIs (4 per bundle × 2 bundles), got {manifest['dataUriCount']}"
    )


def test_data_uri_in_node_src_not_credential_scanned():
    """
    data: URIs in node.src are NOT credential-scanned (scan_credentials skips them).
    This ensures we don't try to urlparse a data: URI as a credential source.
    """
    nodes = [
        {
            "src": "data:image/png;base64,access_token=fake",  # contains secret-looking text but is data:
            "text": None,
        }
    ]
    bundle = _bundle(nodes=nodes)
    findings = scan_credentials(bundle)
    assert findings == [], (
        f"data: URIs in node.src must not be credential-scanned, got: {findings}"
    )


# ===========================================================================
# VARIOUS SECRET NAMES ARE IN SECRET_NAMES
# ===========================================================================


def test_secret_names_set_contains_expected_members():
    """SECRET_NAMES contains both DEFAULT_REDACT_PARAMS and extra secret names."""
    required = {
        # From capture DEFAULT_REDACT_PARAMS
        "token", "sig", "signature", "key", "auth", "apikey", "access_token",
        # Extra
        "password", "passwd", "pwd", "secret", "client_secret",
        "bearer", "jwt", "session", "sessionid", "sid", "api_key",
    }
    for name in required:
        assert name in SECRET_NAMES, f"'{name}' missing from SECRET_NAMES"


def test_various_secret_params_detected():
    """Each secret name in the SECRET_NAMES set trips the credential scan."""
    secret_names_to_test = [
        "token", "sig", "signature", "key", "auth", "apikey", "access_token",
        "password", "passwd", "pwd", "secret", "client_secret",
        "bearer", "jwt", "session", "sessionid", "sid", "api_key",
    ]
    for name in secret_names_to_test:
        bundle = _bundle(network_urls=[f"https://example.com/api?{name}=realvalue"])
        findings = scan_credentials(bundle)
        assert len(findings) >= 1, (
            f"Secret param '{name}=realvalue' should trigger a credential hit, got: {findings}"
        )
        assert any(f["param"] == name for f in findings), (
            f"Finding should name param='{name}', got: {findings}"
        )


def test_case_insensitive_param_names():
    """Secret param name matching is case-insensitive (Token, TOKEN, token all hit)."""
    for variant in ("Token", "TOKEN", "token"):
        bundle = _bundle(network_urls=[f"https://example.com/api?{variant}=abc"])
        findings = scan_credentials(bundle)
        assert findings, (
            f"Param '{variant}=abc' should trigger a credential hit (case-insensitive)"
        )


# ===========================================================================
# total_weight_bytes
# ===========================================================================


def test_total_weight_bytes():
    """total_weight_bytes returns correct sum of file sizes."""
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        f1 = td / "a.json"
        f2 = td / "b.json"
        f3 = td / "c.png"
        f1.write_bytes(b"hello")    # 5 bytes
        f2.write_bytes(b"world!")   # 6 bytes
        f3.write_bytes(b"PNG" * 4)  # 12 bytes

        total = total_weight_bytes([f1, f2], [f3])
        assert total == 23, f"Expected 23 bytes, got {total}"


def test_total_weight_bytes_missing_file_zero():
    """Missing files contribute 0 to the total (not an error)."""
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        existing = td / "real.json"
        existing.write_bytes(b"data")  # 4 bytes
        missing = td / "ghost.png"

        total = total_weight_bytes([existing], [missing])
        assert total == 4, f"Expected 4 bytes (missing file = 0), got {total}"


# ===========================================================================
# REDIRECT CHAIN AND LINK PROBE FIELDS
# ===========================================================================


def test_credential_in_redirect_chain():
    """?token=secret in page.redirectChain[] is detected."""
    bundle = _bundle(redirect_chain=["https://example.com/redirect?token=secret123"])
    findings = scan_credentials(bundle)
    assert findings, f"Expected credential hit in redirectChain, got: {findings}"
    assert any("redirectChain" in f["field"] for f in findings), f"Findings: {findings}"


def test_credential_in_link_probe_redirect_chain():
    """?key=secret in linkProbes[].redirectChain[] is detected."""
    probes = [
        {
            "url": "https://example.com/link",
            "redirectChain": ["https://example.com/link?key=s3cr3t"],
            "finalUrl": "https://example.com/link",
        }
    ]
    bundle = _bundle(link_probes=probes)
    findings = scan_credentials(bundle)
    assert findings, f"Expected credential hit in linkProbes redirectChain, got: {findings}"
    assert any("linkProbes" in f["field"] and "redirectChain" in f["field"] for f in findings)


def test_credential_in_link_probe_final_url():
    """?signature=abc in linkProbes[].finalUrl is detected."""
    probes = [
        {
            "url": "https://example.com/signed",
            "redirectChain": [],
            "finalUrl": "https://example.com/signed?signature=abc123",
        }
    ]
    bundle = _bundle(link_probes=probes)
    findings = scan_credentials(bundle)
    assert findings, f"Expected credential hit in linkProbes finalUrl, got: {findings}"
    assert any("linkProbes" in f["field"] and "finalUrl" in f["field"] for f in findings)


def test_credential_in_node_raw_href():
    """?apikey=secret in page.nodes[].rawHref is detected."""
    nodes = [{"rawHref": "https://example.com/page?apikey=mys3cret", "text": "Link"}]
    bundle = _bundle(nodes=nodes)
    findings = scan_credentials(bundle)
    assert findings, f"Expected credential hit in node rawHref, got: {findings}"
    assert any("rawHref" in f["field"] for f in findings), f"Findings: {findings}"


# ===========================================================================
# Standalone runner
# ===========================================================================

if __name__ == "__main__":
    print("Running pair_privacy.py tests")
    print(f"  Module: {_TESTBED_DIR / 'pair_privacy.py'}")
    print(f"  REDACTION_SENTINEL: {REDACTION_SENTINEL!r}")
    print(f"  SECRET_NAMES count: {len(SECRET_NAMES)}")
    print()

    tests = [
        # Characterization — credential-bearing bundle fails first
        ("test_credential_in_network_url_scan", test_credential_in_network_url_scan),
        ("test_credential_in_network_url_run_gate", test_credential_in_network_url_run_gate),
        ("test_credential_in_node_src_scan", test_credential_in_node_src_scan),
        ("test_credential_in_node_src_run_gate", test_credential_in_node_src_run_gate),
        ("test_credential_in_link_probe_url_scan", test_credential_in_link_probe_url_scan),
        ("test_credential_in_link_probe_url_run_gate", test_credential_in_link_probe_url_run_gate),
        # Sentinel
        ("test_sentinel_in_network_url_no_hit", test_sentinel_in_network_url_no_hit),
        ("test_sentinel_in_node_src_no_hit", test_sentinel_in_node_src_no_hit),
        ("test_sentinel_in_link_probe_no_hit", test_sentinel_in_link_probe_no_hit),
        # assume_yes does not bypass credential scan
        ("test_assume_yes_does_not_bypass_credential_scan", test_assume_yes_does_not_bypass_credential_scan),
        # Clean bundle
        ("test_clean_bundle_gate_passes", test_clean_bundle_gate_passes),
        # assume_yes + clean
        ("test_assume_yes_clean_bundle_no_confirm_call", test_assume_yes_clean_bundle_no_confirm_call),
        # User refuses
        ("test_no_at_ownership_prompt_raises", test_no_at_ownership_prompt_raises),
        ("test_no_at_pii_review_prompt_raises", test_no_at_pii_review_prompt_raises),
        # Size budget
        ("test_size_budget_warning_emitted_non_fatal", test_size_budget_warning_emitted_non_fatal),
        # data: URI counting
        ("test_data_uri_counting_in_manifest", test_data_uri_counting_in_manifest),
        ("test_data_uri_in_node_src_not_credential_scanned", test_data_uri_in_node_src_not_credential_scanned),
        # SECRET_NAMES membership
        ("test_secret_names_set_contains_expected_members", test_secret_names_set_contains_expected_members),
        ("test_various_secret_params_detected", test_various_secret_params_detected),
        ("test_case_insensitive_param_names", test_case_insensitive_param_names),
        # total_weight_bytes
        ("test_total_weight_bytes", test_total_weight_bytes),
        ("test_total_weight_bytes_missing_file_zero", test_total_weight_bytes_missing_file_zero),
        # Additional field coverage
        ("test_credential_in_redirect_chain", test_credential_in_redirect_chain),
        ("test_credential_in_link_probe_redirect_chain", test_credential_in_link_probe_redirect_chain),
        ("test_credential_in_link_probe_final_url", test_credential_in_link_probe_final_url),
        ("test_credential_in_node_raw_href", test_credential_in_node_raw_href),
    ]

    passed = 0
    failed = 0

    for name, fn in tests:
        _test(name, fn)

    print()
    for name, ok, detail in _RESULTS:
        if ok:
            passed += 1
        else:
            failed += 1

    print(f"\n{passed} passed, {failed} failed")
    if failed:
        sys.exit(1)
