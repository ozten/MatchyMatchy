#!/usr/bin/env python3
"""
check-fixture.py  --  validate one testbed variant against its expected-issues.json

Usage:
    python3 testbed/check-fixture.py <variant-name> [options]

Options:
    --matchy <path>         path to matchy binary (default: <repo>/target/release/matchy)
    --out <dir>             output dir override (default: testbed/.runs/<variant>)
    --skip-run              reuse an existing diff-result.json; do not invoke matchy
    --expected <path>       override expected-issues.json path (for unit-style testing)
    --diff-result <path>    override diff-result.json path (for unit-style testing)

When both --expected and --diff-result are provided the script evaluates matchers without
starting servers or running matchy — handy for self-contained unit tests.

All paths are resolved relative to this script's own location so the script works from
any CWD.
"""

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------
SCRIPT_DIR = Path(__file__).resolve().parent
REPO_DIR = SCRIPT_DIR.parent
VARIANTS_DIR = SCRIPT_DIR / "variants"
RUNS_DIR = SCRIPT_DIR / ".runs"
CONTRACT_DIR = REPO_DIR / "contract"

DEFAULT_MATCHY = REPO_DIR / "target" / "release" / "matchy"
GOLDEN_URL = "http://localhost:47000/"

# Matchy uses a repo-local Playwright browser cache so it never touches the
# shared ~/.cache/ms-playwright (see docs/playwright-setup.md). matchy spawns
# `node capture.cjs`, which inherits this env var. setdefault so an explicit
# override from the environment still wins.
os.environ.setdefault("PLAYWRIGHT_BROWSERS_PATH", str(REPO_DIR / ".pw-browsers"))

SEVERITY_RANK = {"info": 0, "warning": 1, "error": 2, "critical": 3}

# ---------------------------------------------------------------------------
# Matcher helpers
# ---------------------------------------------------------------------------

def _type_matches(pattern: str, value: str | None) -> bool:
    """Exact match, or prefix-wildcard when pattern ends with '*'."""
    if value is None:
        return False
    if pattern.endswith("*"):
        return value.startswith(pattern[:-1])
    return value == pattern


def _substring(needle: str, haystack: str | None) -> bool:
    """Case-sensitive substring check; null haystack never matches."""
    if haystack is None:
        return False
    return needle in haystack


def _severity_rank(s: str) -> int:
    return SEVERITY_RANK.get(s, -1)


def _issue_matches(matcher: dict, issue: dict) -> bool:
    """
    Return True iff the issue satisfies ALL present fields in the matcher.
    Fields not present in the matcher are not constraining.
    """
    # --- type / anyOfTypes ---
    if "type" in matcher:
        if not _type_matches(matcher["type"], issue.get("type")):
            return False

    if "anyOfTypes" in matcher:
        if not any(_type_matches(p, issue.get("type")) for p in matcher["anyOfTypes"]):
            return False

    # --- goal ---
    if "goal" in matcher:
        if issue.get("goal") != matcher["goal"]:
            return False

    # --- anchors ---
    if "anchors" in matcher:
        locator = issue.get("locator") or {}
        anchors = locator.get("anchors") or {}

        am = matcher["anchors"]

        if "textContains" in am:
            if not _substring(am["textContains"], anchors.get("text")):
                return False

        if "hrefContains" in am:
            if not _substring(am["hrefContains"], anchors.get("href")):
                return False

        if "nearestHeadingContains" in am:
            if not _substring(am["nearestHeadingContains"], anchors.get("nearestHeading")):
                return False

        if "altContains" in am:
            if not _substring(am["altContains"], anchors.get("alt")):
                return False

        # landmark and role are exact-equal
        if "landmark" in am:
            if anchors.get("landmark") != am["landmark"]:
                return False

        if "role" in am:
            if anchors.get("role") != am["role"]:
                return False

    # --- evidence ---
    if "evidence" in matcher:
        ev = issue.get("evidence") or {}
        rem = issue.get("remediation") or {}
        em = matcher["evidence"]

        if "property" in em:
            prop = em["property"]
            # key present in evidence.old OR evidence.new OR equals remediation.property
            ev_old = ev.get("old") or {}
            ev_new = ev.get("new") or {}
            rem_prop = rem.get("property")
            if not (prop in ev_old or prop in ev_new or rem_prop == prop):
                return False

        if "fromContains" in em:
            from_val = str(rem.get("from", ""))
            if not _substring(em["fromContains"], from_val):
                return False

        if "toContains" in em:
            to_val = str(rem.get("to", ""))
            if not _substring(em["toContains"], to_val):
                return False

        if "oldContains" in em:
            old_val = json.dumps(ev.get("old")) if ev.get("old") is not None else ""
            if not _substring(em["oldContains"], old_val):
                return False

        if "newContains" in em:
            new_val = json.dumps(ev.get("new")) if ev.get("new") is not None else ""
            if not _substring(em["newContains"], new_val):
                return False

    # --- minSeverity / maxSeverity ---
    if "minSeverity" in matcher or "maxSeverity" in matcher:
        issue_rank = _severity_rank(issue.get("severity", ""))
        if "minSeverity" in matcher:
            if issue_rank < _severity_rank(matcher["minSeverity"]):
                return False
        if "maxSeverity" in matcher:
            if issue_rank > _severity_rank(matcher["maxSeverity"]):
                return False

    return True


# ---------------------------------------------------------------------------
# Schema validation
# ---------------------------------------------------------------------------

def _validate_schema(data: dict, schema_path: Path, label: str) -> tuple[bool, str]:
    """Validate data against schema_path. Returns (ok, message)."""
    if not schema_path.exists():
        return False, f"schema file not found: {schema_path}"
    try:
        import jsonschema
        schema = json.loads(schema_path.read_text())
        validator_cls = jsonschema.validators.validator_for(schema)
        v = validator_cls(schema)
        errors = list(v.iter_errors(data))
        if errors:
            msgs = "; ".join(e.message for e in errors[:3])
            return False, f"{len(errors)} schema error(s): {msgs}"
        return True, "valid"
    except Exception as exc:
        return False, f"validation exception: {exc}"


# ---------------------------------------------------------------------------
# Verdict table printer
# ---------------------------------------------------------------------------

def _print_row(check: str, result: str, detail: str) -> None:
    marker = "PASS" if result == "PASS" else "FAIL"
    print(f"  {marker:<6}  {check:<22}  {detail}")


# ---------------------------------------------------------------------------
# Core evaluation
# ---------------------------------------------------------------------------

def evaluate_expected_issues(
    diff_result: dict,
    expected: dict,
) -> tuple[bool, list[tuple[str, str, str]]]:
    """
    Evaluate expected-issues.json against the DiffResult.
    Returns (all_pass, rows) where each row is (check_name, PASS|FAIL, detail).
    """
    rows: list[tuple[str, str, str]] = []
    all_pass = True
    issues: list[dict] = diff_result.get("issues", [])

    # --- required: greedy assignment ---
    consumed: set[int] = set()  # indices into issues[]
    required_entries = expected.get("required", [])
    for i, matcher in enumerate(required_entries):
        matched_idx = None
        for j, issue in enumerate(issues):
            if j in consumed:
                continue
            if _issue_matches(matcher, issue):
                matched_idx = j
                break
        if matched_idx is not None:
            consumed.add(matched_idx)
            detail = f"matched issue {issues[matched_idx].get('id', '?')} ({issues[matched_idx].get('type', '?')})"
            rows.append((f"required[{i}]", "PASS", detail))
        else:
            all_pass = False
            # Describe what the matcher wanted
            parts = []
            if "type" in matcher:
                parts.append(f"type={matcher['type']}")
            if "anyOfTypes" in matcher:
                parts.append(f"anyOfTypes={matcher['anyOfTypes']}")
            if "anchors" in matcher:
                parts.append(f"anchors={matcher['anchors']}")
            if "goal" in matcher:
                parts.append(f"goal={matcher['goal']}")
            rows.append((f"required[{i}]", "FAIL", f"no match for: {', '.join(parts) or repr(matcher)}"))

    # --- forbidden: no issue may satisfy any forbidden matcher ---
    forbidden_entries = expected.get("forbidden", [])
    for i, matcher in enumerate(forbidden_entries):
        violators = [
            issue for issue in issues
            if _issue_matches(matcher, issue)
        ]
        if violators:
            all_pass = False
            parts = []
            for iss in violators[:3]:
                anch = (iss.get("locator") or {}).get("anchors") or {}
                parts.append(f"id={iss.get('id', '?')} type={iss.get('type', '?')} text={anch.get('text', '')!r}")
            rows.append((f"forbidden[{i}]", "FAIL", f"violated by: {'; '.join(parts)}"))
        else:
            parts = []
            if "type" in matcher:
                parts.append(f"type={matcher['type']}")
            if "anyOfTypes" in matcher:
                parts.append(f"anyOfTypes={matcher['anyOfTypes']}")
            rows.append((f"forbidden[{i}]", "PASS", f"no issue matches: {', '.join(parts) or repr(matcher)}"))

    # --- status ---
    actual_status = diff_result.get("status")
    allowed = expected.get("status")
    if isinstance(allowed, str):
        allowed_set = {allowed}
    else:
        allowed_set = set(allowed) if allowed else set()
    if actual_status in allowed_set:
        rows.append(("status", "PASS", f"status={actual_status!r} in allowed={sorted(allowed_set)}"))
    else:
        all_pass = False
        rows.append(("status", "FAIL", f"status={actual_status!r} not in allowed={sorted(allowed_set)}"))

    # --- maxIssues ---
    if "maxIssues" in expected:
        cap = expected["maxIssues"]
        count = len(issues)
        if count <= cap:
            rows.append(("maxIssues", "PASS", f"{count} issues <= cap {cap}"))
        else:
            all_pass = False
            rows.append(("maxIssues", "FAIL", f"{count} issues > cap {cap}"))

    # --- clusters (M8) ---
    # expected["clusters"]["required"] is a list of cluster matchers, each with any of:
    #   sharedProperty / sharedLandmark : match clusters whose field equals this
    #   minMembers   : each matching cluster must have >= this many issueIds
    #   exactlyOne   : exactly one cluster must match the sharedProperty/sharedLandmark filter
    #   memberType   : every member id must resolve to an issue of this type
    clusters_spec = expected.get("clusters")
    if isinstance(clusters_spec, dict):
        clusters: list[dict] = diff_result.get("clusters", [])
        issues_by_id = {i.get("id"): i for i in issues}
        for ci, cm in enumerate(clusters_spec.get("required", [])):
            # Filter clusters by the shared key the matcher constrains.
            def _key_match(c: dict) -> bool:
                if "sharedProperty" in cm and c.get("sharedProperty") != cm["sharedProperty"]:
                    return False
                if "sharedLandmark" in cm and c.get("sharedLandmark") != cm["sharedLandmark"]:
                    return False
                return True

            matched = [c for c in clusters if _key_match(c)]
            problems: list[str] = []

            if cm.get("exactlyOne") and len(matched) != 1:
                problems.append(f"expected exactly 1 matching cluster, got {len(matched)}")
            if not matched:
                problems.append("no cluster matched the shared-key filter")

            for c in matched:
                members = c.get("issueIds", [])
                if "minMembers" in cm and len(members) < cm["minMembers"]:
                    problems.append(
                        f"cluster {c.get('id')} has {len(members)} members < minMembers {cm['minMembers']}"
                    )
                if "memberType" in cm:
                    bad = [
                        m for m in members
                        if (issues_by_id.get(m) or {}).get("type") != cm["memberType"]
                    ]
                    if bad:
                        problems.append(
                            f"cluster {c.get('id')} has {len(bad)} member(s) not of type {cm['memberType']}"
                        )

            key_desc = cm.get("sharedProperty") or cm.get("sharedLandmark") or repr(cm)
            if problems:
                all_pass = False
                rows.append((f"clusters[{ci}]", "FAIL", f"{key_desc}: {'; '.join(problems)}"))
            else:
                detail = f"{key_desc}: {len(matched)} cluster(s)"
                if matched and matched[0].get("issueIds"):
                    detail += f", {len(matched[0]['issueIds'])} members"
                rows.append((f"clusters[{ci}]", "PASS", detail))

    return all_pass, rows


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> int:
    parser = argparse.ArgumentParser(
        prog="check-fixture.py",
        description="Validate a testbed variant against its expected-issues.json.",
    )
    parser.add_argument("variant", help="variant name (e.g. v02-banner-added)")
    parser.add_argument("--matchy", default=None, help="path to matchy binary")
    parser.add_argument("--out", default=None, help="output dir override")
    parser.add_argument(
        "--skip-run", action="store_true",
        help="reuse existing diff-result.json without running matchy",
    )
    parser.add_argument(
        "--expected", default=None,
        help="override path to expected-issues.json (unit-testing escape hatch)",
    )
    parser.add_argument(
        "--diff-result", default=None, dest="diff_result",
        help="override path to diff-result.json (unit-testing escape hatch)",
    )
    args = parser.parse_args()

    variant = args.variant
    unit_mode = args.expected is not None or args.diff_result is not None

    # Resolve short prefixes like "v06" to the full variant directory name.
    if not unit_mode and not (VARIANTS_DIR / variant).is_dir():
        candidates = sorted(
            d.name for d in VARIANTS_DIR.iterdir()
            if d.is_dir() and d.name.startswith(variant)
        )
        if len(candidates) == 1:
            variant = candidates[0]
        elif len(candidates) > 1:
            print(f"ERROR: ambiguous variant '{args.variant}': {', '.join(candidates)}")
            return 1
        else:
            print(f"ERROR: no variant matching '{args.variant}' under {VARIANTS_DIR}")
            return 1

    # Resolve output dir
    if args.out:
        run_dir = Path(args.out).resolve()
    else:
        run_dir = RUNS_DIR / variant

    # Resolve expected-issues.json
    if args.expected:
        expected_path = Path(args.expected).resolve()
    else:
        expected_path = VARIANTS_DIR / variant / "expected-issues.json"

    # Resolve diff-result.json
    if args.diff_result:
        diff_result_path = Path(args.diff_result).resolve()
    else:
        diff_result_path = run_dir / "diff-result.json"

    print(f"=== check-fixture: {variant} ===")
    print()

    overall_pass = True
    schema_rows: list[tuple[str, str, str]] = []

    if not unit_mode:
        # --- 1. Ensure servers are up ---
        print("Starting servers (idempotent)...")
        result = subprocess.run(
            [sys.executable, str(SCRIPT_DIR / "run-all.py"), "start"],
            capture_output=True, text=True,
        )
        if result.returncode != 0:
            print("WARN: run-all.py start returned non-zero:")
            print(result.stdout)
            print(result.stderr)
        else:
            print("  Servers OK")
        print()

        # --- 2. Determine URLs ---
        manifest_path = VARIANTS_DIR / variant / "manifest.json"
        if not manifest_path.exists():
            print(f"ERROR: manifest not found: {manifest_path}")
            return 1
        try:
            manifest = json.loads(manifest_path.read_text())
        except json.JSONDecodeError as exc:
            print(f"ERROR: manifest parse error: {exc}")
            return 1

        old_url = GOLDEN_URL
        new_url = manifest.get("urlUnderTest") or f"http://localhost:{manifest['port']}/"

        # --- 3. Run matchy (unless --skip-run) ---
        if args.skip_run:
            print(f"--skip-run: reusing {diff_result_path}")
        else:
            matchy_bin = Path(args.matchy).resolve() if args.matchy else DEFAULT_MATCHY
            if not matchy_bin.exists():
                print(f"ERROR: matchy binary not found: {matchy_bin}")
                return 1

            run_dir.mkdir(parents=True, exist_ok=True)
            cmd = [
                str(matchy_bin),
                "--old", old_url,
                "--new", new_url,
                "--out", str(run_dir),
                "--viewport", "desktop=1440x1000",
            ]
            print(f"Running: {' '.join(cmd)}")
            result = subprocess.run(cmd)
            if result.returncode not in (0, 1):
                print(f"ERROR: matchy exited with code {result.returncode}")
                return 1
            print()

    # --- 4. Load diff-result.json ---
    if not diff_result_path.exists():
        print(f"ERROR: diff-result.json not found: {diff_result_path}")
        return 1
    try:
        diff_result = json.loads(diff_result_path.read_text())
    except json.JSONDecodeError as exc:
        print(f"ERROR: diff-result.json parse error: {exc}")
        return 1

    # --- 4a. Schema validation ---
    dr_schema_path = CONTRACT_DIR / "diff-result.schema.json"
    ok, msg = _validate_schema(diff_result, dr_schema_path, "diff-result.json")
    if not ok:
        overall_pass = False
    schema_rows.append(("SCHEMA diff-result", "PASS" if ok else "FAIL", msg))

    # Validate any bundle files
    bundle_schema_path = CONTRACT_DIR / "capture-bundle.schema.json"
    # Bundles live under run_dir/<viewport>/*.bundle.json
    if diff_result_path.parent == run_dir or not unit_mode:
        bundle_files = list(run_dir.rglob("*.bundle.json"))
    else:
        bundle_files = []

    if bundle_schema_path.exists() and bundle_files:
        for bundle_path in sorted(bundle_files):
            try:
                bundle_data = json.loads(bundle_path.read_text())
            except json.JSONDecodeError as exc:
                overall_pass = False
                schema_rows.append((
                    f"SCHEMA {bundle_path.name}", "FAIL",
                    f"JSON parse error: {exc}",
                ))
                continue
            ok, msg = _validate_schema(bundle_data, bundle_schema_path, bundle_path.name)
            if not ok:
                overall_pass = False
            schema_rows.append((f"SCHEMA {bundle_path.name}", "PASS" if ok else "FAIL", msg))
    elif not bundle_schema_path.exists() and bundle_files:
        overall_pass = False
        schema_rows.append((
            "SCHEMA bundles", "FAIL",
            f"capture-bundle.schema.json not found: {bundle_schema_path}",
        ))

    # --- 5. Load expected-issues.json ---
    if not expected_path.exists():
        print(f"ERROR: expected-issues.json not found: {expected_path}")
        return 1
    try:
        expected = json.loads(expected_path.read_text())
    except json.JSONDecodeError as exc:
        print(f"ERROR: expected-issues.json parse error: {exc}")
        return 1

    # --- 6. Evaluate matchers ---
    matcher_pass, matcher_rows = evaluate_expected_issues(diff_result, expected)
    if not matcher_pass:
        overall_pass = False

    # --- 7. Print verdict table ---
    print(f"{'CHECK':<26}  {'RESULT':<6}  DETAIL")
    print("-" * 80)

    for check, result, detail in schema_rows:
        _print_row(check, result, detail)

    if schema_rows and matcher_rows:
        print()

    for check, result, detail in matcher_rows:
        _print_row(check, result, detail)

    print()
    verdict = "PASS" if overall_pass else "FAIL"
    print(f"VERDICT: {verdict}")

    return 0 if overall_pass else 1


if __name__ == "__main__":
    sys.exit(main())
