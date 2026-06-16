"""
Tests for testbed/schemas/pair.schema.json.

Runnable as:
  python3 testbed/tests/test_pair_schema.py   (standalone, exits non-zero on failure)
  pytest testbed/tests/test_pair_schema.py    (pytest discovery)
"""

import copy
import json
import os
import sys
from pathlib import Path

import jsonschema

# ---------------------------------------------------------------------------
# Load schema once
# ---------------------------------------------------------------------------

_SCHEMA_PATH = Path(__file__).parent.parent / "schemas" / "pair.schema.json"
_SCHEMA = json.loads(_SCHEMA_PATH.read_text())
_VALIDATOR_CLS = jsonschema.validators.validator_for(_SCHEMA)


def _validate(instance: dict) -> list[jsonschema.exceptions.ValidationError]:
    v = _VALIDATOR_CLS(_SCHEMA)
    return list(v.iter_errors(instance))


def _assert_valid(instance: dict) -> None:
    errors = _validate(instance)
    if errors:
        msgs = "\n".join(f"  - {e.message} (path: {list(e.path)})" for e in errors)
        raise AssertionError(f"Expected valid, got {len(errors)} error(s):\n{msgs}")


def _assert_invalid(instance: dict, reason: str = "") -> None:
    errors = _validate(instance)
    if not errors:
        raise AssertionError(
            f"Expected invalid ({reason}) but schema accepted the instance."
        )


# ---------------------------------------------------------------------------
# Minimal valid manifest (built inline — no seed fixture required)
# ---------------------------------------------------------------------------

_CAPTURE_RECORD = {
    "url": "https://example.com/page",
    "finalUrl": "https://example.com/page",
    "capturedAt": "2026-06-16T12:00:00Z",
    "sha256": "a" * 64,
    "chromiumBuild": "Chromium/124.0.6367.82",
}

_VALID = {
    "caseId": "p01-example-case",
    "description": "A minimal valid real-pair fixture for schema testing.",
    "demonstrates": "true-positive",
    "discoveredVia": "manual audit",
    "goals": ["G1", "G4"],
    "profile": "desktop-en",
    "viewport": "desktop",
    "old": copy.deepcopy(_CAPTURE_RECORD),
    "new": copy.deepcopy(_CAPTURE_RECORD),
    "captureFlags": ["--block-ads"],
    "baseline": None,
    "frozen": True,
    "refreshPolicy": "never",
    "expectedState": "green",
}


# ---------------------------------------------------------------------------
# Test 1: Happy path — fully-populated valid manifest
# ---------------------------------------------------------------------------

def test_happy_path_valid():
    """A fully-populated valid manifest validates with zero errors."""
    _assert_valid(_VALID)

    # Also valid with optional knownDrift present
    with_drift = {**_VALID, "knownDrift": ["Minor font-rendering delta on Linux"]}
    _assert_valid(with_drift)

    # baseline may be a non-null string
    with_baseline = {**_VALID, "baseline": "baselines/p01.json"}
    _assert_valid(with_baseline)


# ---------------------------------------------------------------------------
# Test 2: caseId pattern enforcement
# ---------------------------------------------------------------------------

def test_case_id_wrong_prefix_rejected():
    """caseId starting with 'v' (wrong prefix) must be rejected."""
    bad = {**_VALID, "caseId": "v01-foo"}
    _assert_invalid(bad, reason="wrong prefix 'v'")


def test_case_id_one_digit_rejected():
    """caseId with only one digit after 'p' must be rejected."""
    bad = {**_VALID, "caseId": "p1-foo"}
    _assert_invalid(bad, reason="only one digit after 'p'")


# ---------------------------------------------------------------------------
# Test 3: demonstrates enum and goals pattern
# ---------------------------------------------------------------------------

def test_demonstrates_bad_value_rejected():
    """demonstrates: 'regression' (not in enum) must be rejected."""
    bad = {**_VALID, "demonstrates": "regression"}
    _assert_invalid(bad, reason="'regression' not in demonstrates enum")


def test_goals_bad_value_rejected():
    """goals: ['G9'] (out of range) must be rejected."""
    bad = {**_VALID, "goals": ["G9"]}
    _assert_invalid(bad, reason="G9 not in G1-G8 range")


# ---------------------------------------------------------------------------
# Test 4: sha256 pattern and missing required fields
# ---------------------------------------------------------------------------

def test_sha256_wrong_length_rejected():
    """sha256 of 63 hex chars must be rejected."""
    bad_record = {**_CAPTURE_RECORD, "sha256": "a" * 63}
    bad = {**_VALID, "old": bad_record}
    _assert_invalid(bad, reason="sha256 is 63 chars, not 64")


def test_sha256_non_hex_rejected():
    """sha256 containing non-hex chars must be rejected."""
    bad_record = {**_CAPTURE_RECORD, "sha256": "g" * 64}
    bad = {**_VALID, "old": bad_record}
    _assert_invalid(bad, reason="sha256 contains non-hex char 'g'")


def test_missing_expected_state_rejected():
    """Missing expectedState must be rejected."""
    bad = {k: v for k, v in _VALID.items() if k != "expectedState"}
    _assert_invalid(bad, reason="expectedState is required")


def test_missing_old_sub_field_rejected():
    """old object missing required sub-field 'sha256' must be rejected."""
    bad_record = {k: v for k, v in _CAPTURE_RECORD.items() if k != "sha256"}
    bad = {**_VALID, "old": bad_record}
    _assert_invalid(bad, reason="old.sha256 is required")


# ---------------------------------------------------------------------------
# Test 5: frozen const and expectedState enum
# ---------------------------------------------------------------------------

def test_frozen_false_rejected():
    """frozen: false must be rejected (const true only)."""
    bad = {**_VALID, "frozen": False}
    _assert_invalid(bad, reason="frozen must be const true")


def test_expected_state_pending_rejected():
    """expectedState: 'pending' (not in enum) must be rejected."""
    bad = {**_VALID, "expectedState": "pending"}
    _assert_invalid(bad, reason="'pending' not in expectedState enum")


def test_missing_expected_state_also_rejected():
    """Absence of expectedState (same as test 4 — belt and suspenders) must be rejected."""
    bad = {k: v for k, v in _VALID.items() if k != "expectedState"}
    _assert_invalid(bad, reason="expectedState missing entirely")


# ---------------------------------------------------------------------------
# Standalone runner
# ---------------------------------------------------------------------------

if __name__ == "__main__":
    tests = [
        test_happy_path_valid,
        test_case_id_wrong_prefix_rejected,
        test_case_id_one_digit_rejected,
        test_demonstrates_bad_value_rejected,
        test_goals_bad_value_rejected,
        test_sha256_wrong_length_rejected,
        test_sha256_non_hex_rejected,
        test_missing_expected_state_rejected,
        test_missing_old_sub_field_rejected,
        test_frozen_false_rejected,
        test_expected_state_pending_rejected,
        test_missing_expected_state_also_rejected,
    ]

    passed = 0
    failed = 0
    for fn in tests:
        try:
            fn()
            print(f"  PASS  {fn.__name__}")
            passed += 1
        except AssertionError as exc:
            print(f"  FAIL  {fn.__name__}: {exc}")
            failed += 1

    print(f"\n{passed} passed, {failed} failed")
    if failed:
        sys.exit(1)
