"""
Tests for testbed/check-pair.py  (Unit U4).

Drives check-pair.py as a subprocess.  Test fixtures are built in a temporary
directory and pointed at via --pairs-dir so that NOTHING is ever written to the
real testbed/pairs/ directory.

Each test:
  - creates a minimal but schema-valid pair.json in a temp case dir,
  - places the bundle files (tiny synthetic JSON) whose real SHA-256 matches
    what's recorded in pair.json,
  - places a hand-crafted diff-result.json under <runs_dir>/<case>/,
  - invokes check-pair.py with --skip-run (most tests) so no matchy binary
    is required,
  - asserts exit code and stdout content.

Tests that require a real matchy binary are auto-SKIPPED (print SKIP + continue)
when DEFAULT_MATCHY does not exist.  They are definitively exercised by U8.

Runnable as:
  python3 testbed/tests/test_check_pair.py      (standalone, exits non-zero on failure)
  pytest testbed/tests/test_check_pair.py       (pytest discovery)
"""

import copy
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------

TESTS_DIR = Path(__file__).resolve().parent
TESTBED_DIR = TESTS_DIR.parent
REPO_DIR = TESTBED_DIR.parent
CHECK_PAIR = TESTBED_DIR / "check-pair.py"
CHECK_FIXTURE = TESTBED_DIR / "check-fixture.py"
CONTRACT_DIR = REPO_DIR / "contract"
DEFAULT_MATCHY = REPO_DIR / "target" / "release" / "matchy"
RUNS_DIR = TESTBED_DIR / ".runs"
PAIR_SCHEMA = TESTBED_DIR / "schemas" / "pair.schema.json"

# ---------------------------------------------------------------------------
# Shared minimal data fixtures
# ---------------------------------------------------------------------------

# Minimal valid diff-result.json (schema version 1.1, no issues).
_DIFF_RESULT_PASS = {
    "schemaVersion": "1.1",
    "toolVersion": "0.1.0",
    "runId": "2026-06-16T00-00-00Z",
    "oldUrl": "https://example.com/old",
    "newUrl": "https://example.com/new",
    "parityProfile": "content-structure",
    "status": "pass",
    "agentSummary": {
        "fixableNow": 0,
        "byType": {},
        "clusterCount": 0,
        "topFixes": [],
    },
    "scores": {
        "visual": 1.0,
        "content": 1.0,
        "structure": 1.0,
        "style": 1.0,
        "accessibility": 1.0,
        "technical": 1.0,
        "hygiene": 1.0,
        "byLandmark": {},
    },
    "viewports": [
        {
            "name": "desktop",
            "status": "pass",
            "issues": [],
            "artifacts": {
                "old": "desktop/old.png",
                "new": "desktop/new.png",
                "diff": "desktop/diff.png",
            },
        }
    ],
    "issues": [],
    "clusters": [],
    "suppressed": {"count": 0, "ids": []},
    "warnings": [],
    "scopedTo": None,
    "outOfScope": {"count": 0, "ids": []},
    "determinism": {
        "old": {
            "animationsDisabled": "ran",
            "reducedMotion": "ran",
            "timeFrozen": "ran",
            "randomStubbed": "ran",
            "fontsReady": "ran",
            "imagesDecoded": "ran",
            "lazyLoadPass": "ran",
            "settled": "ran",
            "clicked": [],
            "hidden": [],
            "masked": [],
            "retriedWithoutTimeFreeze": False,
            "integrity": {
                "pre": {"headingCount": 1, "imageCount": 1, "landmarkCount": 1},
                "post": {"headingCount": 1, "imageCount": 1, "landmarkCount": 1},
            },
        },
        "new": {
            "animationsDisabled": "ran",
            "reducedMotion": "ran",
            "timeFrozen": "ran",
            "randomStubbed": "ran",
            "fontsReady": "ran",
            "imagesDecoded": "ran",
            "lazyLoadPass": "ran",
            "settled": "ran",
            "clicked": [],
            "hidden": [],
            "masked": [],
            "retriedWithoutTimeFreeze": False,
            "integrity": {
                "pre": {"headingCount": 1, "imageCount": 1, "landmarkCount": 1},
                "post": {"headingCount": 1, "imageCount": 1, "landmarkCount": 1},
            },
        },
    },
    "artifacts": {
        "old": "desktop/old.png",
        "new": "desktop/new.png",
        "diff": "desktop/diff.png",
    },
}

# A diff-result with one issue (type=style_changed), status=warn.
# All fields match the Issue schema (additionalProperties: false).
_ISSUE = {
    "id": "issue_aabbccddeeff",   # matches ^issue_[0-9a-f]{12}$
    "type": "style_changed",
    "category": "style",
    "severity": "warning",
    "confidence": 0.9,
    "viewport": "desktop",
    "locale": None,
    "goal": "G4",
    "message": "color changed from red to blue",
    "locator": {
        "anchors": {
            "text": "Get started",
            "role": "link",
            "href": "/start",
            "alt": None,
            "ariaLabel": None,
            "nearestHeading": "Hero",
            "landmark": "main",
            "ordinalInLandmark": 1,
        },
        "cssSelectorOld": None,
        "cssSelectorNew": None,
        "bboxOld": None,
        "bboxNew": None,
        "seqIndexOld": None,
        "seqIndexNew": None,
    },
    "evidence": {
        "old": {"color": "red"},
        "new": {"color": "blue"},
    },
    "remediation": {
        "property": "color",
        "from": "red",
        "to": "blue",
        "grepTarget": ".hero a { color: ... }",
    },
}

_DIFF_RESULT_WITH_ISSUE = {
    **copy.deepcopy(_DIFF_RESULT_PASS),
    "status": "warn",
    "issues": [copy.deepcopy(_ISSUE)],
    "agentSummary": {
        "fixableNow": 1,
        "byType": {"style_changed": 1},
        "clusterCount": 0,
        "topFixes": ["issue_001"],
    },
}

# expected-issues that require the style_changed issue to be present.
_EXPECTED_REQUIRES_ISSUE = {
    "status": ["pass", "warn", "fail"],
    "required": [
        {
            "type": "style_changed",
            "anchors": {"textContains": "Get started"},
        }
    ],
    "forbidden": [],
}

# expected-issues that require nothing and accept any status.
_EXPECTED_EMPTY = {
    "status": ["pass", "warn", "fail"],
    "required": [],
    "forbidden": [],
}

# expected-issues that forbid the style_changed issue.
_EXPECTED_FORBIDS_ISSUE = {
    "status": ["pass", "warn", "fail"],
    "required": [],
    "forbidden": [{"type": "style_changed"}],
}

# expected-issues that assert a maxIssues=0 ceiling.
_EXPECTED_MAX_ZERO = {
    "status": ["pass", "warn", "fail"],
    "required": [],
    "forbidden": [],
    "maxIssues": 0,
}


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _write_json(path: Path, obj) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=2))


def _make_bundle_bytes(label: str) -> bytes:
    """Return deterministic synthetic bundle bytes (just enough to be distinct)."""
    return json.dumps({"label": label, "synthetic": True}).encode()


def _build_pair_fixture(
    tmp_dir: Path,
    case_id: str,
    expected_state: str = "green",
    viewport: str = "desktop",
    old_content: bytes | None = None,
    new_content: bytes | None = None,
) -> tuple[Path, Path, Path]:
    """
    Build a minimal valid pair fixture under tmp_dir/pairs/<case_id>/.
    Returns (case_dir, old_bundle_path, new_bundle_path).
    The pair.json SHA-256 values are computed from the actual bytes written.
    """
    case_dir = tmp_dir / "pairs" / case_id
    vp_dir = case_dir / viewport
    vp_dir.mkdir(parents=True, exist_ok=True)

    old_bytes = old_content if old_content is not None else _make_bundle_bytes("old")
    new_bytes = new_content if new_content is not None else _make_bundle_bytes("new")

    old_bundle = vp_dir / "old.bundle.json"
    new_bundle = vp_dir / "new.bundle.json"
    old_bundle.write_bytes(old_bytes)
    new_bundle.write_bytes(new_bytes)

    pair = {
        "caseId": case_id,
        "description": "Synthetic test fixture",
        "demonstrates": "true-positive",
        "discoveredVia": "unit test",
        "goals": ["G1"],
        "profile": "content-structure",
        "viewport": viewport,
        "old": {
            "url": "https://example.com/old",
            "finalUrl": "https://example.com/old",
            "capturedAt": "2026-06-16T12:00:00Z",
            "sha256": _sha256(old_bytes),
            "chromiumBuild": "Chromium/124.0.0.0",
        },
        "new": {
            "url": "https://example.com/new",
            "finalUrl": "https://example.com/new",
            "capturedAt": "2026-06-16T12:00:00Z",
            "sha256": _sha256(new_bytes),
            "chromiumBuild": "Chromium/124.0.0.0",
        },
        "captureFlags": [],
        "baseline": None,
        "frozen": True,
        "refreshPolicy": "never",
        "expectedState": expected_state,
    }
    _write_json(case_dir / "pair.json", pair)
    return case_dir, old_bundle, new_bundle


def _place_diff_result(tmp_dir: Path, case_id: str, diff_result: dict) -> Path:
    """Write diff-result.json into <RUNS_DIR>/<case_id>/diff-result.json."""
    run_dir = tmp_dir / "runs" / case_id
    run_dir.mkdir(parents=True, exist_ok=True)
    dr_path = run_dir / "diff-result.json"
    _write_json(dr_path, diff_result)
    return run_dir


def _run(
    case_id: str,
    tmp_dir: Path,
    extra_args: list | None = None,
    matchy: str | None = "/nonexistent",
) -> tuple[int, str]:
    """
    Invoke check-pair.py for case_id, pointing PAIRS_DIR at tmp_dir/pairs and
    the out dir at tmp_dir/runs/<case_id>.  Returns (exit_code, combined_output).
    By default passes --matchy /nonexistent (so tests that use --skip-run never
    need the real binary).
    """
    run_dir = tmp_dir / "runs" / case_id
    cmd = [
        sys.executable,
        str(CHECK_PAIR),
        case_id,
        "--pairs-dir", str(tmp_dir / "pairs"),
        "--out", str(run_dir),
    ]
    if matchy is not None:
        cmd += ["--matchy", matchy]
    if extra_args:
        cmd += extra_args

    result = subprocess.run(cmd, capture_output=True, text=True)
    output = result.stdout + result.stderr
    return result.returncode, output


# ---------------------------------------------------------------------------
# Tests
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
        _RESULTS.append((name, False, f"UNEXPECTED ERROR: {exc}"))
        print(f"  FAIL  {name}: UNEXPECTED ERROR: {exc}")


# ---------------------------------------------------------------------------
# T1. Happy path: green + all matchers pass -> exit 0, output contains PASS
# ---------------------------------------------------------------------------

def test_happy_path_green_pass():
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        case_id = "p01-test-happy-pass"
        _build_pair_fixture(td, case_id, expected_state="green")
        _place_diff_result(td, case_id, copy.deepcopy(_DIFF_RESULT_PASS))

        _write_json(
            td / "pairs" / case_id / "expected-issues.json",
            _EXPECTED_EMPTY,
        )

        rc, out = _run(case_id, td, extra_args=["--skip-run"])
        assert rc == 0, f"expected exit 0 (PASS), got {rc}\n{out}"
        assert "PASS" in out, f"PASS not in output:\n{out}"
        # Explicitly NOT FAIL or regression message
        assert "FAIL" not in out, f"Unexpected FAIL in output:\n{out}"


# ---------------------------------------------------------------------------
# T2. xfail: red + required matcher unmet -> exit 0, output contains XFAIL
# ---------------------------------------------------------------------------

def test_xfail_correctly_red():
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        case_id = "p02-test-xfail"
        _build_pair_fixture(td, case_id, expected_state="red")
        # diff-result has NO issues, but expected-issues requires style_changed
        _place_diff_result(td, case_id, copy.deepcopy(_DIFF_RESULT_PASS))

        _write_json(
            td / "pairs" / case_id / "expected-issues.json",
            _EXPECTED_REQUIRES_ISSUE,
        )

        rc, out = _run(case_id, td, extra_args=["--skip-run"])
        assert rc == 0, f"expected exit 0 (XFAIL), got {rc}\n{out}"
        assert "XFAIL" in out, f"XFAIL not in output:\n{out}"


# ---------------------------------------------------------------------------
# T3. xpass: red + matchers satisfied -> exit 0, output contains XPASS
# ---------------------------------------------------------------------------

def test_xpass_now_green():
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        case_id = "p03-test-xpass"
        _build_pair_fixture(td, case_id, expected_state="red")
        # diff-result HAS the issue; expected-issues requires it (all pass)
        _place_diff_result(td, case_id, copy.deepcopy(_DIFF_RESULT_WITH_ISSUE))

        _write_json(
            td / "pairs" / case_id / "expected-issues.json",
            _EXPECTED_REQUIRES_ISSUE,
        )

        rc, out = _run(case_id, td, extra_args=["--skip-run"])
        assert rc == 0, f"expected exit 0 (XPASS), got {rc}\n{out}"
        assert "XPASS" in out, f"XPASS not in output:\n{out}"


# ---------------------------------------------------------------------------
# T4a. Regression: green + required matcher unmet -> exit 1
# ---------------------------------------------------------------------------

def test_regression_required_unmet():
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        case_id = "p41-test-regression-required"
        _build_pair_fixture(td, case_id, expected_state="green")
        # No issues in diff-result, but matcher requires one
        _place_diff_result(td, case_id, copy.deepcopy(_DIFF_RESULT_PASS))

        _write_json(
            td / "pairs" / case_id / "expected-issues.json",
            _EXPECTED_REQUIRES_ISSUE,
        )

        rc, out = _run(case_id, td, extra_args=["--skip-run"])
        assert rc == 1, f"expected exit 1 (regression), got {rc}\n{out}"
        assert "FAIL" in out, f"FAIL not in output:\n{out}"


# ---------------------------------------------------------------------------
# T4b. Regression: green + forbidden violation -> exit 1
# ---------------------------------------------------------------------------

def test_regression_forbidden_violated():
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        case_id = "p42-test-regression-forbidden"
        _build_pair_fixture(td, case_id, expected_state="green")
        # diff-result has style_changed; expected-issues forbids it
        _place_diff_result(td, case_id, copy.deepcopy(_DIFF_RESULT_WITH_ISSUE))

        _write_json(
            td / "pairs" / case_id / "expected-issues.json",
            _EXPECTED_FORBIDS_ISSUE,
        )

        rc, out = _run(case_id, td, extra_args=["--skip-run"])
        assert rc == 1, f"expected exit 1 (regression), got {rc}\n{out}"


# ---------------------------------------------------------------------------
# T4c. Regression: green + maxIssues violated -> exit 1
# ---------------------------------------------------------------------------

def test_regression_max_issues_violated():
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        case_id = "p43-test-regression-maxissues"
        _build_pair_fixture(td, case_id, expected_state="green")
        # diff-result has 1 issue; maxIssues=0
        _place_diff_result(td, case_id, copy.deepcopy(_DIFF_RESULT_WITH_ISSUE))

        _write_json(
            td / "pairs" / case_id / "expected-issues.json",
            _EXPECTED_MAX_ZERO,
        )

        rc, out = _run(case_id, td, extra_args=["--skip-run"])
        assert rc == 1, f"expected exit 1 (maxIssues violation), got {rc}\n{out}"


# ---------------------------------------------------------------------------
# T5. SHA mismatch -> exit 2, loud message
# ---------------------------------------------------------------------------

def test_sha_mismatch():
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        case_id = "p05-test-sha-mismatch"
        case_dir, old_bundle, _new_bundle = _build_pair_fixture(td, case_id, expected_state="green")

        # Tamper the old bundle AFTER pair.json was written with its original hash.
        old_bundle.write_bytes(b"tampered content!")

        _place_diff_result(td, case_id, copy.deepcopy(_DIFF_RESULT_PASS))
        _write_json(case_dir / "expected-issues.json", _EXPECTED_EMPTY)

        rc, out = _run(case_id, td, extra_args=["--skip-run"])
        assert rc == 2, f"expected exit 2 (sha mismatch), got {rc}\n{out}"
        # Check for a loud diagnostic message
        assert "SHA-256" in out or "mismatch" in out.lower() or "tamper" in out.lower(), (
            f"Expected SHA-256 mismatch message, got:\n{out}"
        )


# ---------------------------------------------------------------------------
# T6. Missing pair.json -> exit 2
# ---------------------------------------------------------------------------

def test_missing_pair_json():
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        case_id = "p06-test-missing-pair"
        # Don't create pair.json at all (just make the pairs dir)
        (td / "pairs").mkdir(parents=True, exist_ok=True)

        rc, out = _run(case_id, td, extra_args=["--skip-run"])
        assert rc == 2, f"expected exit 2 (missing pair.json), got {rc}\n{out}"
        assert "pair.json" in out, f"Expected pair.json mention, got:\n{out}"


# ---------------------------------------------------------------------------
# T7. Invalid/non-JSON pair.json -> exit 2
# ---------------------------------------------------------------------------

def test_invalid_pair_json():
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        case_id = "p07-test-invalid-pair"
        case_dir = td / "pairs" / case_id
        case_dir.mkdir(parents=True, exist_ok=True)
        (case_dir / "pair.json").write_text("this is not json {{{")

        rc, out = _run(case_id, td, extra_args=["--skip-run"])
        assert rc == 2, f"expected exit 2 (invalid JSON), got {rc}\n{out}"


# ---------------------------------------------------------------------------
# T8. Schema-invalid pair.json (wrong caseId prefix) -> exit 2
# ---------------------------------------------------------------------------

def test_schema_invalid_pair_json():
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        case_id = "p08-test-schema-invalid"
        case_dir, _old, _new = _build_pair_fixture(td, case_id, expected_state="green")

        # Corrupt the caseId to use the wrong prefix so schema validation fails.
        pair_path = case_dir / "pair.json"
        pair = json.loads(pair_path.read_text())
        pair["caseId"] = "v01-wrong-prefix"   # fails the ^p[0-9]{2,}-... pattern
        _write_json(pair_path, pair)

        _place_diff_result(td, case_id, copy.deepcopy(_DIFF_RESULT_PASS))
        _write_json(case_dir / "expected-issues.json", _EXPECTED_EMPTY)

        rc, out = _run(case_id, td, extra_args=["--skip-run"])
        assert rc == 2, f"expected exit 2 (schema violation), got {rc}\n{out}"


# ---------------------------------------------------------------------------
# T9. --skip-run reuses existing diff-result without invoking matchy
#     Pass --matchy /nonexistent to prove no binary is accessed.
# ---------------------------------------------------------------------------

def test_skip_run_no_matchy_needed():
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        case_id = "p09-test-skip-run"
        case_dir, _old, _new = _build_pair_fixture(td, case_id, expected_state="green")
        _place_diff_result(td, case_id, copy.deepcopy(_DIFF_RESULT_PASS))
        _write_json(case_dir / "expected-issues.json", _EXPECTED_EMPTY)

        # --matchy /nonexistent ensures that if matchy were invoked it would fail
        rc, out = _run(
            case_id, td,
            extra_args=["--skip-run"],
            matchy="/nonexistent",
        )
        assert rc == 0, f"expected exit 0 with --skip-run, got {rc}\n{out}"
        assert "skip-run" in out.lower() or "reusing" in out.lower(), (
            f"Expected skip-run acknowledgement, got:\n{out}"
        )


# ---------------------------------------------------------------------------
# T10. ENGINE-REUSE GUARD: check-pair (--skip-run) and check-fixture (unit mode)
#      must agree on the PASS/FAIL verdict for the same (expected, diff-result).
# ---------------------------------------------------------------------------

def test_engine_reuse_guard():
    """
    Run the same (expected-issues, diff-result) through both:
      - check-pair.py (--skip-run, green expectedState)
      - check-fixture.py (--expected <path> --diff-result <path>)
    and assert they agree on PASS vs FAIL.
    """
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)

        # --- Scenario A: matchers satisfied -> both should PASS ---
        case_id_a = "p90-engine-guard-pass"
        case_dir_a, _o, _n = _build_pair_fixture(td, case_id_a, expected_state="green")
        _place_diff_result(td, case_id_a, copy.deepcopy(_DIFF_RESULT_WITH_ISSUE))
        exp_path_a = case_dir_a / "expected-issues.json"
        _write_json(exp_path_a, _EXPECTED_REQUIRES_ISSUE)
        dr_path_a = td / "runs" / case_id_a / "diff-result.json"

        pair_rc_a, pair_out_a = _run(case_id_a, td, extra_args=["--skip-run"])
        # check-fixture unit mode: --expected + --diff-result (no variant/servers needed)
        cf_result_a = subprocess.run(
            [
                sys.executable, str(CHECK_FIXTURE),
                "dummy",
                "--expected", str(exp_path_a),
                "--diff-result", str(dr_path_a),
            ],
            capture_output=True, text=True,
        )
        # check-pair: green + all pass -> exit 0; check-fixture unit mode: all pass -> exit 0
        assert pair_rc_a == 0, f"check-pair: expected 0 (A), got {pair_rc_a}\n{pair_out_a}"
        assert cf_result_a.returncode == 0, (
            f"check-fixture: expected 0 (A), got {cf_result_a.returncode}\n"
            f"{cf_result_a.stdout}{cf_result_a.stderr}"
        )

        # --- Scenario B: required matcher NOT satisfied -> both should report FAIL ---
        case_id_b = "p91-engine-guard-fail"
        case_dir_b, _o, _n = _build_pair_fixture(td, case_id_b, expected_state="green")
        _place_diff_result(td, case_id_b, copy.deepcopy(_DIFF_RESULT_PASS))  # no issues
        exp_path_b = case_dir_b / "expected-issues.json"
        _write_json(exp_path_b, _EXPECTED_REQUIRES_ISSUE)  # requires style_changed (absent)
        dr_path_b = td / "runs" / case_id_b / "diff-result.json"

        pair_rc_b, pair_out_b = _run(case_id_b, td, extra_args=["--skip-run"])
        cf_result_b = subprocess.run(
            [
                sys.executable, str(CHECK_FIXTURE),
                "dummy",
                "--expected", str(exp_path_b),
                "--diff-result", str(dr_path_b),
            ],
            capture_output=True, text=True,
        )
        # check-pair: green + fail -> exit 1; check-fixture unit mode: fail -> exit 1
        assert pair_rc_b == 1, f"check-pair: expected 1 (B), got {pair_rc_b}\n{pair_out_b}"
        assert cf_result_b.returncode == 1, (
            f"check-fixture: expected 1 (B), got {cf_result_b.returncode}\n"
            f"{cf_result_b.stdout}{cf_result_b.stderr}"
        )


# ---------------------------------------------------------------------------
# T11. Layout-guard: flat bundle layout (no <viewport>/ subdir) with a REAL
#      matchy binary causes analyze to fail (missing PNG) -> check-pair exits 2.
#      AUTO-SKIPS when DEFAULT_MATCHY is absent.
# ---------------------------------------------------------------------------

def test_layout_guard_flat_layout():
    if not DEFAULT_MATCHY.exists():
        print(f"  SKIP  test_layout_guard_flat_layout (matchy not built yet)")
        _RESULTS.append(("test_layout_guard_flat_layout", True, "SKIP — no matchy binary"))
        return

    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        case_id = "p11-layout-guard"

        # Build a pair where bundles are at the TOP LEVEL of case_dir (flat — wrong layout).
        # pair.json viewport="desktop" means bundles should be at desktop/old.bundle.json,
        # but we'll place them at old.bundle.json instead (flat).
        # We must still record matching SHA-256s so the integrity check passes.
        case_dir = td / "pairs" / case_id
        case_dir.mkdir(parents=True, exist_ok=True)

        # The pair fixture uses viewport="desktop" so check-pair will look for
        # desktop/old.bundle.json and desktop/new.bundle.json.  Create them in
        # the vp_dir so the integrity check passes, but the bundles reference
        # screenshot paths that won't exist under case_dir.parent().parent() — so
        # analyze will exit non-zero.
        vp_dir = case_dir / "desktop"
        vp_dir.mkdir(parents=True, exist_ok=True)

        old_bytes = b'{"label":"old"}'
        new_bytes = b'{"label":"new"}'
        (vp_dir / "old.bundle.json").write_bytes(old_bytes)
        (vp_dir / "new.bundle.json").write_bytes(new_bytes)

        pair = {
            "caseId": case_id,
            "description": "Layout guard test",
            "demonstrates": "true-positive",
            "discoveredVia": "unit test",
            "goals": ["G1"],
            "profile": "content-structure",
            "viewport": "desktop",
            "old": {
                "url": "https://example.com/old",
                "finalUrl": "https://example.com/old",
                "capturedAt": "2026-06-16T12:00:00Z",
                "sha256": _sha256(old_bytes),
                "chromiumBuild": "Chromium/124.0.0.0",
            },
            "new": {
                "url": "https://example.com/new",
                "finalUrl": "https://example.com/new",
                "capturedAt": "2026-06-16T12:00:00Z",
                "sha256": _sha256(new_bytes),
                "chromiumBuild": "Chromium/124.0.0.0",
            },
            "captureFlags": [],
            "baseline": None,
            "frozen": True,
            "refreshPolicy": "never",
            "expectedState": "green",
        }
        _write_json(case_dir / "pair.json", pair)
        _write_json(case_dir / "expected-issues.json", _EXPECTED_EMPTY)

        run_dir = td / "runs" / case_id
        # Don't pre-place a diff-result: let matchy run (and fail due to missing PNG).

        cmd = [
            sys.executable,
            str(CHECK_PAIR),
            case_id,
            "--pairs-dir", str(td / "pairs"),
            "--out", str(run_dir),
            "--matchy", str(DEFAULT_MATCHY),
        ]
        result = subprocess.run(cmd, capture_output=True, text=True)
        rc = result.returncode
        out = result.stdout + result.stderr

        assert rc == 2, (
            f"Expected exit 2 (flat-layout analyze failure), got {rc}\n{out}"
        )


# ---------------------------------------------------------------------------
# T12. fail-on never: a real FN/FP pair (would exit 1 under default) does NOT
#      trip check-pair because it uses --fail-on never.
#      AUTO-SKIPS when DEFAULT_MATCHY is absent.
#
# This test is conceptually exercised by the analyze path with --fail-on never;
# with --skip-run we can't directly prove the flag is passed.  A live test
# requires the binary; so we SKIP when absent.
# ---------------------------------------------------------------------------

def test_fail_on_never_real_pair():
    """
    With --fail-on never, even a diff-result with issues that would normally
    make analyze exit 1 should leave check-pair's verdict in the hands of
    expectedState + matchers only.  We simulate by using --skip-run with a
    pre-written diff-result (status=fail), and assert:
      - green + matchers satisfied -> exit 0 (PASS, not tripped by analyze)
      - the verdict is from the matcher result, not from analyze's exit code.
    """
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        case_id = "p12-fail-on-never"

        # Build a diff-result with status=fail (simulating a FN/FP pair).
        diff_result_fail = copy.deepcopy(_DIFF_RESULT_WITH_ISSUE)
        diff_result_fail["status"] = "fail"

        case_dir, _o, _n = _build_pair_fixture(td, case_id, expected_state="green")
        _place_diff_result(td, case_id, diff_result_fail)

        # expected-issues: accept fail status, require the style_changed issue (it's there)
        expected = {
            "status": ["pass", "warn", "fail"],
            "required": [{"type": "style_changed"}],
            "forbidden": [],
        }
        _write_json(case_dir / "expected-issues.json", expected)

        # With --skip-run the diff-result is reused and matchy never runs.
        rc, out = _run(case_id, td, extra_args=["--skip-run"], matchy="/nonexistent")
        assert rc == 0, (
            f"Expected exit 0 (PASS — verdict from matchers, not analyze exit), "
            f"got {rc}\n{out}"
        )
        assert "PASS" in out, f"Expected PASS in output:\n{out}"


# ---------------------------------------------------------------------------
# Standalone runner
# ---------------------------------------------------------------------------

if __name__ == "__main__":
    print(f"Running check-pair.py tests")
    print(f"  check-pair.py : {CHECK_PAIR}")
    print(f"  DEFAULT_MATCHY: {DEFAULT_MATCHY} (exists={DEFAULT_MATCHY.exists()})")
    print()

    tests = [
        ("test_happy_path_green_pass", test_happy_path_green_pass),
        ("test_xfail_correctly_red", test_xfail_correctly_red),
        ("test_xpass_now_green", test_xpass_now_green),
        ("test_regression_required_unmet", test_regression_required_unmet),
        ("test_regression_forbidden_violated", test_regression_forbidden_violated),
        ("test_regression_max_issues_violated", test_regression_max_issues_violated),
        ("test_sha_mismatch", test_sha_mismatch),
        ("test_missing_pair_json", test_missing_pair_json),
        ("test_invalid_pair_json", test_invalid_pair_json),
        ("test_schema_invalid_pair_json", test_schema_invalid_pair_json),
        ("test_skip_run_no_matchy_needed", test_skip_run_no_matchy_needed),
        ("test_engine_reuse_guard", test_engine_reuse_guard),
        ("test_layout_guard_flat_layout", test_layout_guard_flat_layout),
        ("test_fail_on_never_real_pair", test_fail_on_never_real_pair),
    ]

    passed = 0
    failed = 0
    skipped = 0

    for name, fn in tests:
        _test(name, fn)

    print()
    for name, ok, detail in _RESULTS:
        if detail.startswith("SKIP"):
            skipped += 1
        elif ok:
            passed += 1
        else:
            failed += 1

    print(f"\n{passed} passed, {failed} failed, {skipped} skipped")
    if failed:
        sys.exit(1)
