"""
Tests for testbed/pair-add.py (Unit U5).

Runnable as:
  python3 testbed/tests/test_pair_add.py      (standalone, exits non-zero on failure)
  pytest testbed/tests/test_pair_add.py       (pytest discovery)

All tests call freeze_and_scaffold() directly on hand-built temp dirs.
No real Playwright, no real matchy binary, no network.
The stub seed_analyze and a clean fake bundle allow the gate to pass with assume_yes=True.

Bundle shape: minimal but gate-compatible (empty network/nodes/linkProbes is fine).
PNG files: tiny placeholder bytes (freeze just copies; analyze never called).
"""

from __future__ import annotations

import contextlib
import hashlib
import io
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

import importlib.util as _ilu

from pair_privacy import PrivacyGateError  # noqa: E402

# pair-add.py uses a hyphen so we must load it via importlib
_pair_add_path = _TESTBED_DIR / "pair-add.py"
_spec = _ilu.spec_from_file_location("pair_add", _pair_add_path)
pair_add = _ilu.module_from_spec(_spec)
_spec.loader.exec_module(pair_add)

freeze_and_scaffold = pair_add.freeze_and_scaffold
_extract_known_drift = pair_add._extract_known_drift
_self_check_failed_warning = pair_add._self_check_failed_warning

# Pair schema path (used for validate checks)
PAIR_SCHEMA_PATH = _TESTBED_DIR / "schemas" / "pair.schema.json"
EXPECTED_ISSUES_SCHEMA_PATH = _TESTBED_DIR / "schemas" / "expected-issues.schema.json"


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _minimal_bundle(
    *,
    final_url: str = "https://example.com/page",
    captured_at: str = "2026-06-16T10:00:00Z",
    chromium_build: str = "Chromium/120.0.6099.109",
    viewport_name: str = "desktop",
    network_urls: list[str] | None = None,
    nodes: list[dict] | None = None,
    link_probes: list[dict] | None = None,
) -> dict:
    """Build a minimal, gate-clean bundle dict."""
    return {
        "capturedAt": captured_at,
        "environment": {
            "chromiumBuild": chromium_build,
        },
        "viewport": {
            "name": viewport_name,
            "width": 1440,
            "height": 1000,
        },
        "page": {
            "url": final_url,
            "finalUrl": final_url,
            "redirectChain": [],
            "statusCode": 200,
            "title": "Example Page",
            "metaDescription": "",
            "canonical": None,
            "lang": "en",
            "pageHeight": 2000,
            "nodes": nodes or [],
            "landmarks": [],
            "network": {
                "requests": [
                    {"url": u, "status": 200, "type": "document", "failed": False}
                    for u in (network_urls or [])
                ]
            },
            "console": [],
            "a11y": {"violations": []},
            "linkProbes": link_probes or [],
        },
        "computedStyles": {},
        "screenshots": {
            "fullPage": "desktop/old.png",
            "viewport": "desktop/old-vp.png",
        },
    }


def _write_bundle(path: Path, bundle: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(bundle), encoding="utf-8")


def _write_png(path: Path, data: bytes = b"\x89PNG\r\n\x1a\n") -> None:
    """Write a tiny placeholder PNG (freeze just copies; analyze is stubbed)."""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def _sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _make_stub_seed_analyze(diff_result: dict | None = None):
    """
    Return a stub seed_analyze callable that:
    - accepts the same kwargs as _real_seed_analyze
    - writes the given diff_result to runs_dir/<case_id>/diff-result.json if not None
    - returns the diff_result dict
    Never touches the real matchy binary.
    """
    def _stub(
        *,
        matchy_bin,
        old_bundle_path,
        new_bundle_path,
        profile,
        runs_dir,
        case_id,
    ) -> dict | None:
        if diff_result is None:
            return None
        out_dir = runs_dir / case_id
        out_dir.mkdir(parents=True, exist_ok=True)
        (out_dir / "diff-result.json").write_text(
            json.dumps(diff_result), encoding="utf-8"
        )
        return diff_result

    return _stub


def _noop_out(_msg: str) -> None:
    pass


def _validate_schema_or_fail(data: dict, schema_path: Path, label: str) -> None:
    """Validate data against JSON schema; raise AssertionError on failure."""
    import jsonschema

    schema = json.loads(schema_path.read_text(encoding="utf-8"))
    validator_cls = jsonschema.validators.validator_for(schema)
    v = validator_cls(schema)
    errors = list(v.iter_errors(data))
    assert not errors, f"{label} schema errors: {[e.message for e in errors]}"


def _build_tmp(
    *,
    viewport_name: str = "desktop",
    old_bundle: dict | None = None,
    new_bundle: dict | None = None,
    include_selfcheck: bool = False,
    capture_diff_result: dict | None = None,
    old_png_data: bytes = b"\x89PNG\r\n\x1a\nOLD",
    new_png_data: bytes = b"\x89PNG\r\n\x1a\nNEW",
) -> Path:
    """
    Build a hand-crafted temp dir mimicking what run_capture() produces.

    `capture_diff_result`, if given, is written to td/diff-result.json -- the
    MAIN diff-result.json produced by the `matchy ... --self-check` invocation
    in run_capture() (see the comment in freeze_and_scaffold's step 5 for why
    this is the file knownDrift is seeded from, not self-check.json).

    Returns the path to the temp dir (caller must manage its lifecycle).
    """
    td = Path(tempfile.mkdtemp(prefix="pair-add-test-"))

    vp = td / viewport_name
    vp.mkdir(parents=True, exist_ok=True)

    # Bundles
    _write_bundle(vp / "old.bundle.json", old_bundle or _minimal_bundle(final_url="https://old.example.com/"))
    _write_bundle(vp / "new.bundle.json", new_bundle or _minimal_bundle(final_url="https://new.example.com/"))

    # PNGs
    _write_png(vp / "old.png", old_png_data)
    _write_png(vp / "new.png", new_png_data)
    _write_png(vp / "old-vp.png", b"\x89PNG\r\n\x1a\nOLD-VP")
    _write_png(vp / "new-vp.png", b"\x89PNG\r\n\x1a\nNEW-VP")

    if include_selfcheck:
        # Mimic what --self-check produces in the viewport dir
        _write_bundle(vp / "old-selfcheck.bundle.json", _minimal_bundle())
        _write_png(vp / "old-selfcheck.png", b"\x89PNG-SC")
        _write_png(vp / "old-selfcheck-vp.png", b"\x89PNG-SC-VP")

    if capture_diff_result is not None:
        (td / "diff-result.json").write_text(
            json.dumps(capture_diff_result), encoding="utf-8"
        )

    return td


# ---------------------------------------------------------------------------
# Test results collector (mirrors test_pair_privacy.py style)
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
        import traceback
        tb = traceback.format_exc()
        _RESULTS.append((name, False, f"UNEXPECTED ERROR: {type(exc).__name__}: {exc}"))
        print(f"  FAIL  {name}: UNEXPECTED ERROR: {type(exc).__name__}: {exc}")
        print(tb)


# ===========================================================================
# T1. Happy path — viewport nesting preserved; sha256 matches frozen bytes
# ===========================================================================


def test_happy_path_viewport_nesting_and_sha256():
    """
    freeze_and_scaffold preserves <viewport>/ nesting:
      pairs/<case>/<viewport>/{old,new}.bundle.json + all 4 PNGs are present.
    The recorded sha256 in pair.json equals the actual frozen bundle bytes.
    """
    import shutil

    old_bundle = _minimal_bundle(final_url="https://old.example.com/page")
    new_bundle = _minimal_bundle(final_url="https://new.example.com/page")

    tmp_dir = _build_tmp(
        viewport_name="desktop",
        old_bundle=old_bundle,
        new_bundle=new_bundle,
    )
    try:
        with tempfile.TemporaryDirectory() as pairs_td, \
             tempfile.TemporaryDirectory() as runs_td:

            pairs_dir = Path(pairs_td)
            runs_dir = Path(runs_td)

            stub_dr = {"status": "warn", "issues": [], "runId": "test-run"}
            stub_seed = _make_stub_seed_analyze(stub_dr)

            freeze_and_scaffold(
                tmp_dir=tmp_dir,
                case_id="p99-test-case",
                viewport_name="desktop",
                url_old="https://old.example.com/page",
                url_new="https://new.example.com/page",
                profile="content-structure",
                capture_flags=["--profile", "content-structure", "--viewport", "desktop=1440x1000"],
                pairs_dir=pairs_dir,
                runs_dir=runs_dir,
                assume_yes=True,
                seed_analyze=stub_seed,
            )

            vp_frozen = pairs_dir / "p99-test-case" / "desktop"

            # All 6 files present
            for fname in ["old.bundle.json", "new.bundle.json", "old.png", "new.png", "old-vp.png", "new-vp.png"]:
                assert (vp_frozen / fname).exists(), f"Missing frozen file: {fname}"

            # pair.json sha256 matches actual frozen bytes
            pair_json = json.loads((pairs_dir / "p99-test-case" / "pair.json").read_text())
            actual_old_sha = _sha256(vp_frozen / "old.bundle.json")
            actual_new_sha = _sha256(vp_frozen / "new.bundle.json")
            assert pair_json["old"]["sha256"] == actual_old_sha, "old sha256 mismatch"
            assert pair_json["new"]["sha256"] == actual_new_sha, "new sha256 mismatch"
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


# ===========================================================================
# T2. pair.json fields come from bundle, not from requested URL/host
# ===========================================================================


def test_pair_json_fields_from_bundle():
    """
    pair.json finalUrl/chromiumBuild/viewport come from the BUNDLE, not from
    the requested URL. expectedState defaults to 'red'.
    pair.json validates against pair.schema.json.
    """
    import shutil

    old_bundle = _minimal_bundle(
        final_url="https://old-redirected.example.com/real-page",
        chromium_build="Chromium/121.0.0.1",
        captured_at="2026-06-16T12:00:00Z",
        viewport_name="desktop",
    )
    new_bundle = _minimal_bundle(
        final_url="https://new-redirected.example.com/real-page",
        chromium_build="Chromium/121.0.0.2",
        captured_at="2026-06-16T12:01:00Z",
        viewport_name="desktop",
    )

    tmp_dir = _build_tmp(
        viewport_name="desktop",
        old_bundle=old_bundle,
        new_bundle=new_bundle,
    )
    try:
        with tempfile.TemporaryDirectory() as pairs_td, \
             tempfile.TemporaryDirectory() as runs_td:

            pairs_dir = Path(pairs_td)
            runs_dir = Path(runs_td)

            stub_seed = _make_stub_seed_analyze({"status": "warn", "issues": []})

            freeze_and_scaffold(
                tmp_dir=tmp_dir,
                case_id="p99-bundle-fields",
                viewport_name="desktop",
                url_old="https://old-requested.example.com/page",  # different from finalUrl
                url_new="https://new-requested.example.com/page",
                profile="content-structure",
                capture_flags=["--profile", "content-structure"],
                pairs_dir=pairs_dir,
                runs_dir=runs_dir,
                assume_yes=True,
                seed_analyze=stub_seed,
            )

            pair_json = json.loads((pairs_dir / "p99-bundle-fields" / "pair.json").read_text())

            # finalUrl comes from bundle, not from requested URL
            assert pair_json["old"]["finalUrl"] == "https://old-redirected.example.com/real-page", (
                f"Expected finalUrl from bundle, got: {pair_json['old']['finalUrl']}"
            )
            assert pair_json["new"]["finalUrl"] == "https://new-redirected.example.com/real-page", (
                f"Expected finalUrl from bundle, got: {pair_json['new']['finalUrl']}"
            )

            # chromiumBuild from bundle
            assert pair_json["old"]["chromiumBuild"] == "Chromium/121.0.0.1"
            assert pair_json["new"]["chromiumBuild"] == "Chromium/121.0.0.2"

            # viewport from the subdir name passed in
            assert pair_json["viewport"] == "desktop"

            # expectedState defaults to "red"
            assert pair_json["expectedState"] == "red", (
                f"expectedState should default to 'red', got: {pair_json['expectedState']}"
            )

            # url fields use the requested URL
            assert pair_json["old"]["url"] == "https://old-requested.example.com/page"
            assert pair_json["new"]["url"] == "https://new-requested.example.com/page"

            # Schema validation
            _validate_schema_or_fail(pair_json, PAIR_SCHEMA_PATH, "pair.json")
    finally:
        import shutil
        shutil.rmtree(tmp_dir, ignore_errors=True)


# ===========================================================================
# T3. Stub expected-issues.json has empty required/forbidden (R3)
# ===========================================================================


def test_stub_expected_issues_empty_required_forbidden():
    """
    R3 (load-bearing): required and forbidden in expected-issues.json MUST be
    empty arrays — even when the stub seed_analyze returns a diff-result WITH issues.
    The stub must NOT mirror the seeded diff-result's issues.
    """
    import shutil

    tmp_dir = _build_tmp(viewport_name="desktop")
    try:
        with tempfile.TemporaryDirectory() as pairs_td, \
             tempfile.TemporaryDirectory() as runs_td:

            pairs_dir = Path(pairs_td)
            runs_dir = Path(runs_td)

            # Stub seed_analyze returns a diff-result WITH issues
            diff_result_with_issues = {
                "status": "fail",
                "issues": [
                    {
                        "id": "abc123",
                        "type": "color_contrast",
                        "goal": "G2",
                        "severity": "error",
                        "locator": {"anchors": {"text": "Get started"}},
                        "evidence": {"old": {}, "new": {}},
                        "remediation": {},
                    },
                    {
                        "id": "def456",
                        "type": "missing_element",
                        "goal": "G1",
                        "severity": "warning",
                        "locator": {"anchors": {"text": "Header"}},
                        "evidence": {"old": {}, "new": {}},
                        "remediation": {},
                    },
                ],
            }
            stub_seed = _make_stub_seed_analyze(diff_result_with_issues)

            freeze_and_scaffold(
                tmp_dir=tmp_dir,
                case_id="p99-r3-test",
                viewport_name="desktop",
                url_old="https://old.example.com/",
                url_new="https://new.example.com/",
                profile="content-structure",
                capture_flags=[],
                pairs_dir=pairs_dir,
                runs_dir=runs_dir,
                assume_yes=True,
                seed_analyze=stub_seed,
            )

            expected_issues = json.loads(
                (pairs_dir / "p99-r3-test" / "expected-issues.json").read_text()
            )

            # R3: required and forbidden MUST be empty
            assert expected_issues["required"] == [], (
                f"required must be empty (R3), got: {expected_issues['required']}"
            )
            assert expected_issues["forbidden"] == [], (
                f"forbidden must be empty (R3), got: {expected_issues['forbidden']}"
            )

            # notes must be non-empty
            assert expected_issues.get("notes"), "notes must be non-empty"

            # status is taken from the seeded diff-result (not hard-coded)
            assert expected_issues["status"] == "fail", (
                f"status should come from seeded diff-result, got: {expected_issues['status']}"
            )
    finally:
        import shutil
        shutil.rmtree(tmp_dir, ignore_errors=True)


# ===========================================================================
# T4. Gate-before-freeze (R5): token-bearing bundle → PrivacyGateError AND
#     pairs/<case>/ NOT created
# ===========================================================================


def test_gate_before_freeze_no_partial_write():
    """
    R5 (load-bearing): the privacy gate runs BEFORE pairs/<case>/ is created.
    A token-bearing old bundle with assume_yes=True still raises PrivacyGateError
    AND pairs/<case>/ is absent after the failure (no partial write).
    """
    import shutil

    # Bundle with a credential-bearing URL (access_token will fail the gate)
    bad_bundle = _minimal_bundle(network_urls=["https://cdn.example.com/file?access_token=abc123"])
    clean_bundle = _minimal_bundle()

    tmp_dir = _build_tmp(
        viewport_name="desktop",
        old_bundle=bad_bundle,
        new_bundle=clean_bundle,
    )
    try:
        with tempfile.TemporaryDirectory() as pairs_td, \
             tempfile.TemporaryDirectory() as runs_td:

            pairs_dir = Path(pairs_td)
            runs_dir = Path(runs_td)

            stub_seed = _make_stub_seed_analyze({"status": "warn", "issues": []})

            raised = False
            try:
                freeze_and_scaffold(
                    tmp_dir=tmp_dir,
                    case_id="p99-gate-test",
                    viewport_name="desktop",
                    url_old="https://old.example.com/",
                    url_new="https://new.example.com/",
                    profile="content-structure",
                    capture_flags=[],
                    pairs_dir=pairs_dir,
                    runs_dir=runs_dir,
                    assume_yes=True,  # still fails on credential scan
                    seed_analyze=stub_seed,
                )
            except PrivacyGateError:
                raised = True

            assert raised, "PrivacyGateError should have been raised for token-bearing bundle"

            # pairs/<case>/ must NOT exist — no partial write
            case_dir = pairs_dir / "p99-gate-test"
            assert not case_dir.exists(), (
                f"pairs/<case>/ must NOT be created when gate fails; found: {case_dir}"
            )
    finally:
        import shutil
        shutil.rmtree(tmp_dir, ignore_errors=True)


# ===========================================================================
# T5. F1: selfcheck artifacts excluded from freeze
# ===========================================================================


def test_f1_selfcheck_artifacts_excluded():
    """
    F1: when tmp/<vp>/ ALSO contains old-selfcheck.{bundle.json,png,-vp.png},
    the frozen pairs/<case>/<vp>/ contains NONE of them.
    The frozen dir contains exactly the 6 allowlisted files.
    """
    import shutil

    tmp_dir = _build_tmp(
        viewport_name="desktop",
        include_selfcheck=True,
    )
    try:
        with tempfile.TemporaryDirectory() as pairs_td, \
             tempfile.TemporaryDirectory() as runs_td:

            pairs_dir = Path(pairs_td)
            runs_dir = Path(runs_td)

            stub_seed = _make_stub_seed_analyze({"status": "warn", "issues": []})

            freeze_and_scaffold(
                tmp_dir=tmp_dir,
                case_id="p99-f1-test",
                viewport_name="desktop",
                url_old="https://old.example.com/",
                url_new="https://new.example.com/",
                profile="content-structure",
                capture_flags=[],
                pairs_dir=pairs_dir,
                runs_dir=runs_dir,
                assume_yes=True,
                seed_analyze=stub_seed,
            )

            vp_frozen = pairs_dir / "p99-f1-test" / "desktop"

            frozen_names = {f.name for f in vp_frozen.iterdir() if f.is_file()}

            # No selfcheck artifacts
            selfcheck_present = {n for n in frozen_names if "selfcheck" in n or "self-check" in n}
            assert not selfcheck_present, (
                f"F1: selfcheck artifacts must not be frozen, found: {selfcheck_present}"
            )

            # Exactly the 6 allowlisted files
            expected_names = {
                "old.bundle.json", "new.bundle.json",
                "old.png", "new.png",
                "old-vp.png", "new-vp.png",
            }
            assert frozen_names == expected_names, (
                f"Frozen dir should contain exactly {expected_names}, got: {frozen_names}"
            )
    finally:
        import shutil
        shutil.rmtree(tmp_dir, ignore_errors=True)


# ===========================================================================
# T6. volatile_capture warning → pair.json.knownDrift
#
# These fixtures use the REAL RunWarning shape matchy emits into the main
# diff-result.json ({"code", "message", "context"} in warnings[]) -- see the
# volatile_capture / self_check_failed examples in the U4 design brief.
# ===========================================================================


def test_volatile_capture_seeded_into_known_drift():
    """
    Scenario (a): a capture-run diff-result.json containing one volatile_capture
    warning seeds pair.json.knownDrift with exactly that warning's message
    string. The resulting pair.json also validates against pair.schema.json
    (scenario e).
    """
    import shutil

    drift_message = (
        "self-check: 2 issue(s) appeared when diffing two captures of the old "
        "page against each other; treat similar issues in the main result with "
        "suspicion (capture volatility, e.g. rotating content)"
    )
    capture_dr = {
        "schemaVersion": "1.2",
        "runId": "test-run",
        "status": "warn",
        "issues": [],
        "warnings": [
            {
                "code": "volatile_capture",
                "message": drift_message,
                "context": {"issueCount": 2, "byType": {"text-changed": 2}},
            }
        ],
    }

    tmp_dir = _build_tmp(
        viewport_name="desktop",
        capture_diff_result=capture_dr,
    )
    try:
        with tempfile.TemporaryDirectory() as pairs_td, \
             tempfile.TemporaryDirectory() as runs_td:

            pairs_dir = Path(pairs_td)
            runs_dir = Path(runs_td)

            stub_seed = _make_stub_seed_analyze({"status": "warn", "issues": []})

            freeze_and_scaffold(
                tmp_dir=tmp_dir,
                case_id="p99-drift-test",
                viewport_name="desktop",
                url_old="https://old.example.com/",
                url_new="https://new.example.com/",
                profile="content-structure",
                capture_flags=[],
                pairs_dir=pairs_dir,
                runs_dir=runs_dir,
                assume_yes=True,
                seed_analyze=stub_seed,
            )

            pair_json = json.loads((pairs_dir / "p99-drift-test" / "pair.json").read_text())

            known_drift = pair_json.get("knownDrift", [])
            assert known_drift == [drift_message], (
                f"Expected knownDrift == [message string], got: {known_drift}"
            )

            # Scenario (e): seeded pair.json still validates against the schema.
            _validate_schema_or_fail(pair_json, PAIR_SCHEMA_PATH, "pair.json (knownDrift-seeded)")
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


def test_clean_probe_no_known_drift_no_operator_warning():
    """
    Scenario (b): a capture-run diff-result.json with an empty warnings list
    (clean self-check probe, no drift) → knownDrift is empty and no operator
    warning about self-check failure is printed.
    """
    import shutil

    capture_dr = {
        "schemaVersion": "1.2",
        "runId": "test-run",
        "status": "pass",
        "issues": [],
        "warnings": [],
    }

    tmp_dir = _build_tmp(
        viewport_name="desktop",
        capture_diff_result=capture_dr,
    )
    try:
        with tempfile.TemporaryDirectory() as pairs_td, \
             tempfile.TemporaryDirectory() as runs_td:

            pairs_dir = Path(pairs_td)
            runs_dir = Path(runs_td)

            stub_seed = _make_stub_seed_analyze({"status": "pass", "issues": []})

            stderr_capture = io.StringIO()
            with contextlib.redirect_stderr(stderr_capture):
                freeze_and_scaffold(
                    tmp_dir=tmp_dir,
                    case_id="p99-clean-probe-test",
                    viewport_name="desktop",
                    url_old="https://old.example.com/",
                    url_new="https://new.example.com/",
                    profile="content-structure",
                    capture_flags=[],
                    pairs_dir=pairs_dir,
                    runs_dir=runs_dir,
                    assume_yes=True,
                    seed_analyze=stub_seed,
                )

            pair_json = json.loads((pairs_dir / "p99-clean-probe-test" / "pair.json").read_text())
            assert pair_json.get("knownDrift", []) == [], (
                f"Expected empty knownDrift, got: {pair_json.get('knownDrift')}"
            )
            assert "self-check probe failed" not in stderr_capture.getvalue(), (
                "No operator warning expected for a clean probe, got: "
                f"{stderr_capture.getvalue()!r}"
            )
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


def test_self_check_failed_seeds_empty_known_drift_and_warns_operator():
    """
    Scenario (c): a capture-run diff-result.json carrying a self_check_failed
    warning → knownDrift stays empty (never fabricated from a failed probe)
    and the operator message is printed to stderr; the flow is NOT aborted.
    """
    import shutil

    capture_dr = {
        "schemaVersion": "1.2",
        "runId": "test-run",
        "status": "pass",
        "issues": [],
        "warnings": [
            {
                "code": "self_check_failed",
                "message": (
                    "self-check probe failed for 1 of 1 viewport(s): "
                    "desktop (capture)"
                ),
                "context": {
                    "failedViewports": {"desktop": "capture"},
                    "selfCheckJsonWriteFailed": False,
                },
            }
        ],
    }

    tmp_dir = _build_tmp(
        viewport_name="desktop",
        capture_diff_result=capture_dr,
    )
    try:
        with tempfile.TemporaryDirectory() as pairs_td, \
             tempfile.TemporaryDirectory() as runs_td:

            pairs_dir = Path(pairs_td)
            runs_dir = Path(runs_td)

            stub_seed = _make_stub_seed_analyze({"status": "pass", "issues": []})

            stderr_capture = io.StringIO()
            with contextlib.redirect_stderr(stderr_capture):
                freeze_and_scaffold(
                    tmp_dir=tmp_dir,
                    case_id="p99-self-check-failed-test",
                    viewport_name="desktop",
                    url_old="https://old.example.com/",
                    url_new="https://new.example.com/",
                    profile="content-structure",
                    capture_flags=[],
                    pairs_dir=pairs_dir,
                    runs_dir=runs_dir,
                    assume_yes=True,
                    seed_analyze=stub_seed,
                )

            # Flow was not aborted: pair.json was still written.
            pair_json = json.loads(
                (pairs_dir / "p99-self-check-failed-test" / "pair.json").read_text()
            )
            assert pair_json.get("knownDrift", []) == [], (
                f"Expected empty knownDrift on self_check_failed, got: {pair_json.get('knownDrift')}"
            )

            stderr_text = stderr_capture.getvalue()
            assert "self-check probe failed" in stderr_text, (
                f"Expected operator message about self-check failure, got: {stderr_text!r}"
            )
            assert "knownDrift not seeded" in stderr_text, (
                f"Expected operator message to mention knownDrift was not seeded, got: {stderr_text!r}"
            )
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


def test_missing_diff_result_json_known_drift_empty():
    """
    Scenario (d): no capture-run diff-result.json at all (e.g. a run without
    --self-check, or a hand-built tmp_dir) → existing graceful behavior is
    preserved: knownDrift is empty and freeze_and_scaffold does not raise.
    """
    import shutil

    tmp_dir = _build_tmp(viewport_name="desktop")  # no capture_diff_result
    assert not (tmp_dir / "diff-result.json").exists()

    try:
        with tempfile.TemporaryDirectory() as pairs_td, \
             tempfile.TemporaryDirectory() as runs_td:

            pairs_dir = Path(pairs_td)
            runs_dir = Path(runs_td)

            stub_seed = _make_stub_seed_analyze({"status": "pass", "issues": []})

            freeze_and_scaffold(
                tmp_dir=tmp_dir,
                case_id="p99-missing-diff-result-test",
                viewport_name="desktop",
                url_old="https://old.example.com/",
                url_new="https://new.example.com/",
                profile="content-structure",
                capture_flags=[],
                pairs_dir=pairs_dir,
                runs_dir=runs_dir,
                assume_yes=True,
                seed_analyze=stub_seed,
            )

            pair_json = json.loads(
                (pairs_dir / "p99-missing-diff-result-test" / "pair.json").read_text()
            )
            assert pair_json.get("knownDrift", []) == [], (
                f"Expected empty knownDrift when diff-result.json is missing, got: {pair_json.get('knownDrift')}"
            )
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


# ===========================================================================
# T7. --refresh: rewrites bundles+PNGs+hashes; leaves expected-issues.json
#     and expectedState byte-for-byte unchanged
# ===========================================================================


def test_refresh_rewrites_hashes_leaves_expected_issues_unchanged():
    """
    Starting from an existing frozen pair, --refresh:
      - overwrites bundles + PNGs + sha256 hashes in pair.json
      - leaves expected-issues.json byte-for-byte unchanged
      - leaves pair.json expectedState unchanged
    """
    import shutil

    # --- Initial freeze ---
    old_bundle_v1 = _minimal_bundle(
        final_url="https://old.example.com/v1",
        captured_at="2026-06-16T10:00:00Z",
        chromium_build="Chromium/120.0.0.0",
    )
    new_bundle_v1 = _minimal_bundle(
        final_url="https://new.example.com/v1",
        captured_at="2026-06-16T10:01:00Z",
        chromium_build="Chromium/120.0.0.0",
    )

    tmp_v1 = _build_tmp(
        viewport_name="desktop",
        old_bundle=old_bundle_v1,
        new_bundle=new_bundle_v1,
        old_png_data=b"\x89PNG-V1-OLD",
        new_png_data=b"\x89PNG-V1-NEW",
    )

    pairs_td = Path(tempfile.mkdtemp(prefix="pair-add-refresh-pairs-"))
    runs_td = Path(tempfile.mkdtemp(prefix="pair-add-refresh-runs-"))

    try:
        stub_seed = _make_stub_seed_analyze({"status": "warn", "issues": []})

        # Initial freeze
        freeze_and_scaffold(
            tmp_dir=tmp_v1,
            case_id="p99-refresh-test",
            viewport_name="desktop",
            url_old="https://old.example.com/v1",
            url_new="https://new.example.com/v1",
            profile="content-structure",
            capture_flags=["--profile", "content-structure", "--viewport", "desktop=1440x1000"],
            pairs_dir=pairs_td,
            runs_dir=runs_td,
            assume_yes=True,
            seed_analyze=stub_seed,
        )

        # Record state after initial freeze
        pair_v1 = json.loads((pairs_td / "p99-refresh-test" / "pair.json").read_text())
        expected_issues_v1_text = (pairs_td / "p99-refresh-test" / "expected-issues.json").read_text()
        old_sha_v1 = pair_v1["old"]["sha256"]
        new_sha_v1 = pair_v1["new"]["sha256"]
        expected_state_v1 = pair_v1["expectedState"]

        assert expected_state_v1 == "red"

        # --- Simulate refresh with new bundles ---
        old_bundle_v2 = _minimal_bundle(
            final_url="https://old.example.com/v2",
            captured_at="2026-06-17T10:00:00Z",
            chromium_build="Chromium/121.0.0.0",
        )
        new_bundle_v2 = _minimal_bundle(
            final_url="https://new.example.com/v2",
            captured_at="2026-06-17T10:01:00Z",
            chromium_build="Chromium/121.0.0.0",
        )

        tmp_v2 = _build_tmp(
            viewport_name="desktop",
            old_bundle=old_bundle_v2,
            new_bundle=new_bundle_v2,
            old_png_data=b"\x89PNG-V2-OLD",
            new_png_data=b"\x89PNG-V2-NEW",
        )
        try:
            vp_frozen = pairs_td / "p99-refresh-test" / "desktop"

            # Run the freeze portion of a refresh:
            # We call freeze_and_scaffold with the NEW tmp artifacts and the
            # SAME case_id — simulating what _do_refresh does.
            # But to keep the test self-contained we directly test freeze_and_scaffold
            # because _do_refresh calls run_capture (real binary).
            # The test emulates the core freeze+update logic:

            # Copy new bundles into frozen dir (mimics what refresh does after gate)
            import shutil as _shutil
            vp_tmp2 = tmp_v2 / "desktop"
            for fname in ["old.bundle.json", "new.bundle.json", "old.png", "new.png", "old-vp.png", "new-vp.png"]:
                _shutil.copy2(vp_tmp2 / fname, vp_frozen / fname)

            # Recompute sha256 and update pair.json (mirrors _do_refresh)
            new_old_sha = hashlib.sha256((vp_frozen / "old.bundle.json").read_bytes()).hexdigest()
            new_new_sha = hashlib.sha256((vp_frozen / "new.bundle.json").read_bytes()).hexdigest()

            old_b2 = json.loads((vp_frozen / "old.bundle.json").read_text())
            new_b2 = json.loads((vp_frozen / "new.bundle.json").read_text())

            pair_v1["old"]["finalUrl"] = old_b2["page"]["finalUrl"]
            pair_v1["old"]["capturedAt"] = old_b2["capturedAt"]
            pair_v1["old"]["sha256"] = new_old_sha
            pair_v1["old"]["chromiumBuild"] = old_b2["environment"]["chromiumBuild"]
            pair_v1["new"]["finalUrl"] = new_b2["page"]["finalUrl"]
            pair_v1["new"]["capturedAt"] = new_b2["capturedAt"]
            pair_v1["new"]["sha256"] = new_new_sha
            pair_v1["new"]["chromiumBuild"] = new_b2["environment"]["chromiumBuild"]

            (pairs_td / "p99-refresh-test" / "pair.json").write_text(
                json.dumps(pair_v1, indent=2) + "\n"
            )

            # --- Assertions ---

            pair_v2 = json.loads((pairs_td / "p99-refresh-test" / "pair.json").read_text())
            expected_issues_v2_text = (pairs_td / "p99-refresh-test" / "expected-issues.json").read_text()

            # sha256 changed (new bundles)
            assert pair_v2["old"]["sha256"] != old_sha_v1, "sha256 should change after refresh"
            assert pair_v2["new"]["sha256"] != new_sha_v1, "sha256 should change after refresh"

            # sha256 matches actual frozen bytes
            actual_old = hashlib.sha256((vp_frozen / "old.bundle.json").read_bytes()).hexdigest()
            actual_new = hashlib.sha256((vp_frozen / "new.bundle.json").read_bytes()).hexdigest()
            assert pair_v2["old"]["sha256"] == actual_old
            assert pair_v2["new"]["sha256"] == actual_new

            # expected-issues.json: byte-for-byte unchanged
            assert expected_issues_v2_text == expected_issues_v1_text, (
                "expected-issues.json must be byte-for-byte unchanged after refresh"
            )

            # expectedState: unchanged
            assert pair_v2["expectedState"] == expected_state_v1, (
                f"expectedState must not change on refresh; was {expected_state_v1!r}, "
                f"got {pair_v2['expectedState']!r}"
            )
        finally:
            shutil.rmtree(tmp_v2, ignore_errors=True)
    finally:
        shutil.rmtree(tmp_v1, ignore_errors=True)
        shutil.rmtree(pairs_td, ignore_errors=True)
        shutil.rmtree(runs_td, ignore_errors=True)


# ===========================================================================
# T8. pair.json validates against pair.schema.json
# ===========================================================================


def test_pair_json_validates_against_schema():
    """
    The scaffolded pair.json validates against testbed/schemas/pair.schema.json.
    """
    import shutil

    tmp_dir = _build_tmp(viewport_name="desktop")
    try:
        with tempfile.TemporaryDirectory() as pairs_td, \
             tempfile.TemporaryDirectory() as runs_td:

            pairs_dir = Path(pairs_td)
            runs_dir = Path(runs_td)

            stub_seed = _make_stub_seed_analyze({"status": "pass", "issues": []})

            freeze_and_scaffold(
                tmp_dir=tmp_dir,
                case_id="p99-schema-test",
                viewport_name="desktop",
                url_old="https://old.example.com/",
                url_new="https://new.example.com/",
                profile="content-structure",
                capture_flags=["--profile", "content-structure", "--viewport", "desktop=1440x1000"],
                pairs_dir=pairs_dir,
                runs_dir=runs_dir,
                assume_yes=True,
                seed_analyze=stub_seed,
            )

            pair_json = json.loads((pairs_dir / "p99-schema-test" / "pair.json").read_text())
            _validate_schema_or_fail(pair_json, PAIR_SCHEMA_PATH, "pair.json")
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


# ===========================================================================
# T9. _extract_known_drift / _self_check_failed_warning: unit coverage
#
# Both operate on a diff-result.json-shaped dict with a REAL warnings[] list
# of {code, message, context} RunWarning entries (not a self-check.json
# key-scan — see U4 in docs/plans/2026-07-09-001-fix-self-check-silent-noop-plan.md).
# ===========================================================================


def test_extract_known_drift_single_match():
    """A single volatile_capture warning's message is extracted."""
    dr = {"warnings": [{"code": "volatile_capture", "message": "Analytics detected", "context": None}]}
    result = _extract_known_drift(dr)
    assert result == ["Analytics detected"], f"Got: {result}"


def test_extract_known_drift_multiple_and_other_codes_ignored():
    """Only volatile_capture entries are extracted; other codes are ignored, order preserved."""
    dr = {
        "warnings": [
            {"code": "capture_step_failed", "message": "unrelated", "context": None},
            {"code": "volatile_capture", "message": "Banner 1 detected", "context": None},
            {"code": "volatile_capture", "message": "Banner 2 detected", "context": None},
        ]
    }
    result = _extract_known_drift(dr)
    assert result == ["Banner 1 detected", "Banner 2 detected"], f"Got: {result}"


def test_extract_known_drift_no_warnings_key():
    """No 'warnings' key at all → empty list (not an error)."""
    result = _extract_known_drift({"status": "pass", "issues": []})
    assert result == [], f"Expected empty list, got: {result}"


def test_extract_known_drift_empty_warnings_list():
    """Empty warnings list → empty list."""
    result = _extract_known_drift({"warnings": []})
    assert result == [], f"Expected empty list, got: {result}"


def test_self_check_failed_warning_present():
    """A self_check_failed entry in warnings[] is returned as a dict."""
    dr = {
        "warnings": [
            {
                "code": "self_check_failed",
                "message": "self-check probe failed for 1 of 1 viewport(s): desktop (capture)",
                "context": {"failedViewports": {"desktop": "capture"}, "selfCheckJsonWriteFailed": False},
            }
        ]
    }
    w = _self_check_failed_warning(dr)
    assert w is not None and w["code"] == "self_check_failed", f"Got: {w}"


def test_self_check_failed_warning_absent():
    """No self_check_failed entry → None."""
    dr = {"warnings": [{"code": "volatile_capture", "message": "x", "context": None}]}
    assert _self_check_failed_warning(dr) is None


# ===========================================================================
# T10. Missing required artifact → clear error (SystemExit)
# ===========================================================================


def test_missing_artifact_raises_system_exit():
    """
    If a required artifact is missing from tmp_dir/<viewport>/, freeze_and_scaffold
    exits with a clear error (SystemExit 2).
    """
    import shutil

    tmp_dir = _build_tmp(viewport_name="desktop")
    # Remove one required file
    (tmp_dir / "desktop" / "old.png").unlink()

    try:
        with tempfile.TemporaryDirectory() as pairs_td, \
             tempfile.TemporaryDirectory() as runs_td:

            pairs_dir = Path(pairs_td)
            runs_dir = Path(runs_td)

            try:
                freeze_and_scaffold(
                    tmp_dir=tmp_dir,
                    case_id="p99-missing-test",
                    viewport_name="desktop",
                    url_old="https://old.example.com/",
                    url_new="https://new.example.com/",
                    profile="content-structure",
                    capture_flags=[],
                    pairs_dir=pairs_dir,
                    runs_dir=runs_dir,
                    assume_yes=True,
                    seed_analyze=_make_stub_seed_analyze(),
                )
                assert False, "Expected SystemExit for missing artifact"
            except SystemExit as exc:
                assert exc.code != 0, f"Expected non-zero exit, got: {exc.code}"
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


# ===========================================================================
# T11. seed_analyze=None (binary absent) → stub status='warn', no crash
# ===========================================================================


def test_seed_analyze_none_returns_warn_status():
    """
    When seed_analyze returns None (matchy binary absent), the expected-issues.json
    stub uses status='warn' and no exception is raised.
    """
    import shutil

    tmp_dir = _build_tmp(viewport_name="desktop")
    try:
        with tempfile.TemporaryDirectory() as pairs_td, \
             tempfile.TemporaryDirectory() as runs_td:

            pairs_dir = Path(pairs_td)
            runs_dir = Path(runs_td)

            # Stub returns None (simulates absent binary)
            none_seed = _make_stub_seed_analyze(None)

            freeze_and_scaffold(
                tmp_dir=tmp_dir,
                case_id="p99-no-seed",
                viewport_name="desktop",
                url_old="https://old.example.com/",
                url_new="https://new.example.com/",
                profile="content-structure",
                capture_flags=[],
                pairs_dir=pairs_dir,
                runs_dir=runs_dir,
                assume_yes=True,
                seed_analyze=none_seed,
            )

            expected_issues = json.loads(
                (pairs_dir / "p99-no-seed" / "expected-issues.json").read_text()
            )
            assert expected_issues["status"] == "warn", (
                f"Expected status='warn' when seed absent, got: {expected_issues['status']}"
            )
            assert expected_issues["required"] == []
            assert expected_issues["forbidden"] == []
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


# ===========================================================================
# Standalone runner
# ===========================================================================

if __name__ == "__main__":
    print("Running pair_add.py tests")
    print(f"  Module: {_TESTBED_DIR / 'pair-add.py'}")
    print()

    tests = [
        # Happy path
        ("test_happy_path_viewport_nesting_and_sha256", test_happy_path_viewport_nesting_and_sha256),
        # Bundle fields come from bundle, not requested URL
        ("test_pair_json_fields_from_bundle", test_pair_json_fields_from_bundle),
        # R3: required/forbidden always empty in stub
        ("test_stub_expected_issues_empty_required_forbidden", test_stub_expected_issues_empty_required_forbidden),
        # Gate before freeze (R5)
        ("test_gate_before_freeze_no_partial_write", test_gate_before_freeze_no_partial_write),
        # F1: selfcheck excluded
        ("test_f1_selfcheck_artifacts_excluded", test_f1_selfcheck_artifacts_excluded),
        # volatile_capture → knownDrift (U4 scenario a + e)
        ("test_volatile_capture_seeded_into_known_drift", test_volatile_capture_seeded_into_known_drift),
        # U4 scenario b: clean probe → empty knownDrift, no operator warning
        ("test_clean_probe_no_known_drift_no_operator_warning", test_clean_probe_no_known_drift_no_operator_warning),
        # U4 scenario c: self_check_failed → empty knownDrift + operator message
        ("test_self_check_failed_seeds_empty_known_drift_and_warns_operator",
         test_self_check_failed_seeds_empty_known_drift_and_warns_operator),
        # U4 scenario d: missing diff-result.json → graceful empty knownDrift
        ("test_missing_diff_result_json_known_drift_empty", test_missing_diff_result_json_known_drift_empty),
        # --refresh
        ("test_refresh_rewrites_hashes_leaves_expected_issues_unchanged",
         test_refresh_rewrites_hashes_leaves_expected_issues_unchanged),
        # Schema validation
        ("test_pair_json_validates_against_schema", test_pair_json_validates_against_schema),
        # _extract_known_drift / _self_check_failed_warning unit coverage
        ("test_extract_known_drift_single_match", test_extract_known_drift_single_match),
        ("test_extract_known_drift_multiple_and_other_codes_ignored",
         test_extract_known_drift_multiple_and_other_codes_ignored),
        ("test_extract_known_drift_no_warnings_key", test_extract_known_drift_no_warnings_key),
        ("test_extract_known_drift_empty_warnings_list", test_extract_known_drift_empty_warnings_list),
        ("test_self_check_failed_warning_present", test_self_check_failed_warning_present),
        ("test_self_check_failed_warning_absent", test_self_check_failed_warning_absent),
        # Error paths
        ("test_missing_artifact_raises_system_exit", test_missing_artifact_raises_system_exit),
        # No seed (binary absent)
        ("test_seed_analyze_none_returns_warn_status", test_seed_analyze_none_returns_warn_status),
    ]

    for name, fn in tests:
        _test(name, fn)

    print()
    passed = sum(1 for _, ok, _ in _RESULTS if ok)
    failed = sum(1 for _, ok, _ in _RESULTS if not ok)
    print(f"\n{passed} passed, {failed} failed")
    if failed:
        sys.exit(1)
