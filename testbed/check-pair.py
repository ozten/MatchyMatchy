#!/usr/bin/env python3
"""
check-pair.py  --  replay and assert a Tier-3 real-pair regression fixture.

Usage:
    python3 testbed/check-pair.py <case-id> [options]

Options:
    --matchy <path>     path to matchy binary (default: <repo>/target/release/matchy)
    --skip-run          reuse existing diff-result.json without invoking matchy
    --out <dir>         output dir override (default: testbed/.runs/<case-id>)

Flow:
  1. Load + validate pair.json against pair.schema.json.
  2. Recompute SHA-256 of both frozen bundle files; abort on mismatch.
  3. Run `matchy analyze --old-bundle ... --new-bundle ... --fail-on never` (unless --skip-run).
  4. Schema-validate the resulting diff-result.json.
  5. Evaluate expected-issues.json via the shared engine (imported from check-fixture.py).
  6. Reconcile with expectedState -> exit 0 (PASS or XFAIL or XPASS) / 1 (regression) / 2 (error).

Engine reuse (R4): evaluate_expected_issues and helpers imported from check-fixture.py
via importlib.  check-fixture.py is import-safe (main() guarded by __name__ == "__main__").

All paths are resolved relative to this script's own location so the script works from
any CWD.
"""

import argparse
import hashlib
import importlib.util
import json
import subprocess
import sys
from pathlib import Path

# ---------------------------------------------------------------------------
# Import check-fixture.py engine (R4 — non-negotiable)
# ---------------------------------------------------------------------------

_cf_path = Path(__file__).resolve().parent / "check-fixture.py"
_spec = importlib.util.spec_from_file_location("check_fixture", _cf_path)
check_fixture = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(check_fixture)

# Re-export the symbols we need (keeps the rest of the script readable)
evaluate_expected_issues = check_fixture.evaluate_expected_issues
_validate_schema = check_fixture._validate_schema
_print_row = check_fixture._print_row

# Reuse path constants from check-fixture so they stay in sync.
SCRIPT_DIR = check_fixture.SCRIPT_DIR
CONTRACT_DIR = check_fixture.CONTRACT_DIR
DEFAULT_MATCHY = check_fixture.DEFAULT_MATCHY
RUNS_DIR = check_fixture.RUNS_DIR

# Tier-3 pairs live here (no servers needed; purely frozen bundles).
PAIRS_DIR = SCRIPT_DIR / "pairs"
PAIR_SCHEMA_PATH = SCRIPT_DIR / "schemas" / "pair.schema.json"


# ---------------------------------------------------------------------------
# SHA-256 integrity helper
# ---------------------------------------------------------------------------

def _sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> int:
    parser = argparse.ArgumentParser(
        prog="check-pair.py",
        description="Replay and assert a Tier-3 real-pair regression fixture.",
    )
    parser.add_argument("case_id", help="case identifier (e.g. p01-hiya-number-registration)")
    parser.add_argument("--matchy", default=None, help="path to matchy binary")
    parser.add_argument(
        "--skip-run", action="store_true",
        help="reuse existing diff-result.json without invoking matchy",
    )
    parser.add_argument("--out", default=None, help="output dir override")
    # Internal override used by tests to redirect PAIRS_DIR without touching real fixtures.
    parser.add_argument(
        "--pairs-dir", default=None, dest="pairs_dir",
        help=argparse.SUPPRESS,  # internal: point at a temp pairs dir for testing
    )
    args = parser.parse_args()

    case_id = args.case_id

    # Allow test isolation: override PAIRS_DIR via --pairs-dir.
    pairs_dir = Path(args.pairs_dir).resolve() if args.pairs_dir else PAIRS_DIR
    case_dir = pairs_dir / case_id

    print(f"=== check-pair: {case_id} ===")
    print()

    # ------------------------------------------------------------------
    # 1. Load pair.json
    # ------------------------------------------------------------------
    pair_json_path = case_dir / "pair.json"
    if not pair_json_path.exists():
        print(f"ERROR: pair.json not found: {pair_json_path}")
        return 2
    try:
        pair = json.loads(pair_json_path.read_text())
    except json.JSONDecodeError as exc:
        print(f"ERROR: pair.json is not valid JSON: {exc}")
        return 2

    # ------------------------------------------------------------------
    # 2. Schema-validate pair.json against pair.schema.json
    # ------------------------------------------------------------------
    ok, msg = _validate_schema(pair, PAIR_SCHEMA_PATH, "pair.json")
    if not ok:
        print(f"ERROR: pair.json schema validation failed: {msg}")
        return 2

    # ------------------------------------------------------------------
    # 3. Read key fields
    # ------------------------------------------------------------------
    viewport = pair["viewport"]
    profile = pair.get("profile", "content-structure")
    baseline = pair.get("baseline")           # string path or null
    expected_state = pair["expectedState"]    # "green" or "red"

    vp_dir = case_dir / viewport
    old_bundle_path = vp_dir / "old.bundle.json"
    new_bundle_path = vp_dir / "new.bundle.json"

    # ------------------------------------------------------------------
    # 4. SHA-256 integrity check
    # ------------------------------------------------------------------
    for side, bundle_path, sha_key in [
        ("old", old_bundle_path, ("old", "sha256")),
        ("new", new_bundle_path, ("new", "sha256")),
    ]:
        if not bundle_path.exists():
            print(f"ERROR: bundle file missing: {bundle_path}")
            return 2
        expected_sha = pair[sha_key[0]][sha_key[1]]
        actual_sha = _sha256_file(bundle_path)
        if actual_sha != expected_sha:
            print(
                f"ERROR: SHA-256 mismatch for {side} bundle!\n"
                f"  file : {bundle_path}\n"
                f"  want : {expected_sha}\n"
                f"  got  : {actual_sha}\n"
                f"The frozen bundle has been tampered with or replaced.  Aborting."
            )
            return 2

    print(f"  SHA-256 integrity: OK (both bundles match pair.json)")
    print()

    # ------------------------------------------------------------------
    # 5. Resolve output dir and diff-result path
    # ------------------------------------------------------------------
    if args.out:
        run_dir = Path(args.out).resolve()
    else:
        run_dir = RUNS_DIR / case_id

    diff_result_path = run_dir / "diff-result.json"

    # ------------------------------------------------------------------
    # 6. Replay (unless --skip-run)
    # ------------------------------------------------------------------
    if args.skip_run:
        print(f"  --skip-run: reusing {diff_result_path}")
        print()
    else:
        # Resolve the matchy binary.
        if args.matchy:
            matchy_bin = Path(args.matchy).resolve()
        else:
            matchy_bin = DEFAULT_MATCHY

        if not matchy_bin.exists():
            print(
                f"ERROR: matchy binary not found: {matchy_bin}\n"
                f"  Build it first:  cargo build --release --bin matchy"
            )
            return 2

        run_dir.mkdir(parents=True, exist_ok=True)

        cmd = [
            str(matchy_bin),
            "analyze",
            "--old-bundle", str(old_bundle_path),
            "--new-bundle", str(new_bundle_path),
            "--out", str(run_dir),
            "--profile", profile,
            "--fail-on", "never",
        ]
        if baseline:
            cmd += ["--baseline", str(Path(baseline).resolve())]

        print(f"  Running: {' '.join(cmd)}")
        result = subprocess.run(cmd, capture_output=True, text=True)
        rc = result.returncode

        # analyze exits 0 (pass) or 1 (issues found / fail-on threshold) on a clean
        # run; --fail-on never makes it always exit 0 on a clean run.  Anything else
        # (2 = tool error, e.g. missing PNG, malformed bundle) is a harness error.
        if rc not in (0, 1):
            print(f"ERROR: matchy analyze exited {rc} (expected 0 or 1 on a clean run)")
            if result.stderr:
                print("  stderr:", result.stderr[:2000])
            if result.stdout:
                print("  stdout:", result.stdout[:1000])
            return 2
        print()

    # ------------------------------------------------------------------
    # 7. Load diff-result.json
    # ------------------------------------------------------------------
    if not diff_result_path.exists():
        print(f"ERROR: diff-result.json not found: {diff_result_path}")
        return 2
    try:
        diff_result = json.loads(diff_result_path.read_text())
    except json.JSONDecodeError as exc:
        print(f"ERROR: diff-result.json is not valid JSON: {exc}")
        return 2

    # ------------------------------------------------------------------
    # 7a. Schema-validate diff-result.json
    # ------------------------------------------------------------------
    dr_schema_path = CONTRACT_DIR / "diff-result.schema.json"
    schema_ok, schema_msg = _validate_schema(diff_result, dr_schema_path, "diff-result.json")
    schema_rows = [("SCHEMA diff-result", "PASS" if schema_ok else "FAIL", schema_msg)]
    if not schema_ok:
        print(f"ERROR: diff-result.json schema validation failed: {schema_msg}")
        return 2

    # ------------------------------------------------------------------
    # 8. Load expected-issues.json
    # ------------------------------------------------------------------
    expected_path = case_dir / "expected-issues.json"
    if not expected_path.exists():
        print(f"ERROR: expected-issues.json not found: {expected_path}")
        return 2
    try:
        expected = json.loads(expected_path.read_text())
    except json.JSONDecodeError as exc:
        print(f"ERROR: expected-issues.json is not valid JSON: {exc}")
        return 2

    # ------------------------------------------------------------------
    # 9. Evaluate matchers via the shared engine (R4)
    # ------------------------------------------------------------------
    matcher_pass, matcher_rows = evaluate_expected_issues(diff_result, expected)

    # ------------------------------------------------------------------
    # 10. Print verdict table
    # ------------------------------------------------------------------
    print(f"{'CHECK':<26}  {'RESULT':<6}  DETAIL")
    print("-" * 80)

    for row in schema_rows:
        _print_row(*row)

    if schema_rows and matcher_rows:
        print()

    for row in matcher_rows:
        _print_row(*row)

    print()

    # ------------------------------------------------------------------
    # 11. Reconcile with expectedState and choose exit code
    # ------------------------------------------------------------------
    #
    #   expectedState  all_pass  → label                        exit
    #   green          True      → PASS                          0
    #   green          False     → FAIL (regression)             1
    #   red            False     → XFAIL (locked red)            0   (R11 — does NOT break gate)
    #   red            True      → XPASS (now green — flip it)   0   (non-fatal warning)
    #
    if expected_state == "green":
        if matcher_pass:
            print("VERDICT: PASS")
            return 0
        else:
            print("VERDICT: FAIL (regression)")
            return 1
    elif expected_state == "red":
        if not matcher_pass:
            print("VERDICT: XFAIL (locked red — pending FN/FP)")
            print(
                "  NOTE: This fixture is intentionally red (R11 xfail mechanism).\n"
                "        It does not break the CI gate. Fix the underlying analysis\n"
                "        defect, then flip expectedState to \"green\" in pair.json."
            )
            return 0
        else:
            print("VERDICT: XPASS (now green — flip expectedState to \"green\")")
            print(
                "  WARNING: All matchers now pass on a fixture that was marked red.\n"
                "           Flip pair.json expectedState from \"red\" to \"green\" and\n"
                "           consider recording a byte golden."
            )
            return 0
    else:
        # Should not reach here if pair.json validated against the schema.
        print(f"ERROR: unexpected expectedState value: {expected_state!r}")
        return 2


if __name__ == "__main__":
    sys.exit(main())
