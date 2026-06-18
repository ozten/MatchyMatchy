"""
Tests for the region-rollup and maxTopLevelItems blocks added to
testbed/check-fixture.py (Unit U6).

Calls evaluate_expected_issues() directly via importlib so we exercise the
engine in isolation — no servers, no matchy binary, no filesystem variants.

Runnable as:
  python3 testbed/tests/test_check_fixture_regions.py
  pytest testbed/tests/test_check_fixture_regions.py
"""

import importlib.util
import sys
from pathlib import Path

# ---------------------------------------------------------------------------
# Load check-fixture.py as a module (the file has a hyphen in its name).
# ---------------------------------------------------------------------------

TESTS_DIR = Path(__file__).resolve().parent
TESTBED_DIR = TESTS_DIR.parent
_spec = importlib.util.spec_from_file_location(
    "check_fixture", TESTBED_DIR / "check-fixture.py"
)
_mod = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_mod)

evaluate_expected_issues = _mod.evaluate_expected_issues

# ---------------------------------------------------------------------------
# Minimal synthetic diff_result builder
# ---------------------------------------------------------------------------

def _make_issue(id_, type_, landmark=None, severity="error"):
    return {
        "id": id_,
        "type": type_,
        "category": "content",
        "severity": severity,
        "confidence": 0.9,
        "viewport": "desktop",
        "locale": None,
        "goal": "G4",
        "message": "test issue",
        "locator": {
            "anchors": {
                "text": "test",
                "role": "generic",
                "href": None,
                "alt": None,
                "ariaLabel": None,
                "nearestHeading": None,
                "landmark": landmark,
                "ordinalInLandmark": 1,
            },
            "cssSelectorOld": None,
            "cssSelectorNew": None,
            "bboxOld": None,
            "bboxNew": None,
            "seqIndexOld": None,
            "seqIndexNew": None,
        },
        "evidence": {"old": {}, "new": {}},
        "remediation": {
            "property": "display",
            "from": "block",
            "to": "none",
            "grepTarget": ".x { display: ... }",
        },
    }


def _make_region(id_, landmark, saturation, member_ids, severity="error"):
    return {
        "id": id_,
        "landmark": landmark,
        "saturation": saturation,
        "structuralCount": 44,
        "oldNodeCount": 51,
        "memberIssueIds": member_ids,
        "severity": severity,
        "summary": f"{landmark} region rollup",
    }


def _make_cluster(id_, issue_ids, shared_property="display", shared_landmark=None):
    c = {
        "id": id_,
        "issueIds": issue_ids,
        "summary": "cluster summary",
    }
    if shared_property:
        c["sharedProperty"] = shared_property
    if shared_landmark:
        c["sharedLandmark"] = shared_landmark
    return c


def _diff_result(issues=None, clusters=None, regions=None, status="warn"):
    return {
        "schemaVersion": "1.2",
        "status": status,
        "issues": issues or [],
        "clusters": clusters or [],
        "regions": regions or [],
    }


def _expected(status=None, required=None, forbidden=None, regions_spec=None,
              max_top=None, notes="test"):
    exp = {
        "status": status or ["pass", "warn", "fail"],
        "required": required or [],
        "forbidden": forbidden or [],
        "notes": notes,
    }
    if regions_spec is not None:
        exp["regions"] = regions_spec
    if max_top is not None:
        exp["maxTopLevelItems"] = max_top
    return exp


# ---------------------------------------------------------------------------
# Test runner
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
# Helper: assert a row with the given check name passes or fails
# ---------------------------------------------------------------------------

def _find_row(rows, check_name):
    for check, result, detail in rows:
        if check == check_name:
            return result, detail
    return None, None


# ---------------------------------------------------------------------------
# R1. regions.required — happy path: one contentinfo region at 0.86, passes
# ---------------------------------------------------------------------------

def test_regions_required_passes_single_match():
    issue = _make_issue("issue_aabbcc000001", "missing_text", landmark="contentinfo")
    region = _make_region("region_abc123def456", "contentinfo", 0.86, ["issue_aabbcc000001"])
    dr = _diff_result(issues=[issue], regions=[region])
    exp = _expected(
        regions_spec={"required": [
            {"landmark": "contentinfo", "minSaturation": 0.6, "exactlyOne": True}
        ]}
    )
    all_pass, rows = evaluate_expected_issues(dr, exp)
    result, detail = _find_row(rows, "regions[0]")
    assert result == "PASS", f"Expected PASS, got {result}: {detail}"
    assert all_pass, "all_pass should be True"


# ---------------------------------------------------------------------------
# R2. regions.required — FAIL when zero contentinfo regions exist
# ---------------------------------------------------------------------------

def test_regions_required_fails_no_match():
    dr = _diff_result(regions=[])
    exp = _expected(
        regions_spec={"required": [
            {"landmark": "contentinfo", "minSaturation": 0.6, "exactlyOne": True}
        ]}
    )
    all_pass, rows = evaluate_expected_issues(dr, exp)
    result, detail = _find_row(rows, "regions[0]")
    assert result == "FAIL", f"Expected FAIL, got {result}: {detail}"
    assert not all_pass, "all_pass should be False"


# ---------------------------------------------------------------------------
# R3. regions.required — FAIL when two contentinfo regions (exactlyOne violated)
# ---------------------------------------------------------------------------

def test_regions_required_fails_exactlyone_two_matches():
    issue1 = _make_issue("issue_aabbcc000001", "missing_text", landmark="contentinfo")
    issue2 = _make_issue("issue_aabbcc000002", "missing_link", landmark="contentinfo")
    region1 = _make_region("region_abc123def456", "contentinfo", 0.86, ["issue_aabbcc000001"])
    region2 = _make_region("region_abc123def457", "contentinfo", 0.75, ["issue_aabbcc000002"])
    dr = _diff_result(issues=[issue1, issue2], regions=[region1, region2])
    exp = _expected(
        regions_spec={"required": [
            {"landmark": "contentinfo", "minSaturation": 0.6, "exactlyOne": True}
        ]}
    )
    all_pass, rows = evaluate_expected_issues(dr, exp)
    result, detail = _find_row(rows, "regions[0]")
    assert result == "FAIL", f"Expected FAIL (exactlyOne violated), got {result}: {detail}"
    assert not all_pass, "all_pass should be False"


# ---------------------------------------------------------------------------
# R4. minSaturation boundary — region at exactly 0.6 passes minSaturation: 0.6
# ---------------------------------------------------------------------------

def test_regions_min_saturation_boundary_exact():
    issue = _make_issue("issue_aabbcc000001", "missing_text", landmark="contentinfo")
    region = _make_region("region_abc123def456", "contentinfo", 0.6, ["issue_aabbcc000001"])
    dr = _diff_result(issues=[issue], regions=[region])
    exp = _expected(
        regions_spec={"required": [
            {"landmark": "contentinfo", "minSaturation": 0.6}
        ]}
    )
    all_pass, rows = evaluate_expected_issues(dr, exp)
    result, detail = _find_row(rows, "regions[0]")
    assert result == "PASS", f"Region at exactly minSaturation should PASS, got {result}: {detail}"
    assert all_pass, "all_pass should be True"


# ---------------------------------------------------------------------------
# R5. minSaturation — region below threshold FAILS
# ---------------------------------------------------------------------------

def test_regions_min_saturation_below_fails():
    issue = _make_issue("issue_aabbcc000001", "missing_text", landmark="contentinfo")
    # saturation 0.59 < minSaturation 0.6
    region = _make_region("region_abc123def456", "contentinfo", 0.59, ["issue_aabbcc000001"])
    dr = _diff_result(issues=[issue], regions=[region])
    exp = _expected(
        regions_spec={"required": [
            {"landmark": "contentinfo", "minSaturation": 0.6}
        ]}
    )
    all_pass, rows = evaluate_expected_issues(dr, exp)
    result, detail = _find_row(rows, "regions[0]")
    assert result == "FAIL", f"Region below minSaturation should FAIL, got {result}: {detail}"
    assert not all_pass, "all_pass should be False"


# ---------------------------------------------------------------------------
# R6. memberIncludesType — passes when a member resolves to the named type
# ---------------------------------------------------------------------------

def test_regions_member_includes_type_passes():
    issue = _make_issue("issue_aabbcc000001", "broken_link", landmark="contentinfo")
    region = _make_region("region_abc123def456", "contentinfo", 0.86, ["issue_aabbcc000001"])
    dr = _diff_result(issues=[issue], regions=[region])
    exp = _expected(
        regions_spec={"required": [
            {"landmark": "contentinfo", "memberIncludesType": "broken_link"}
        ]}
    )
    all_pass, rows = evaluate_expected_issues(dr, exp)
    result, detail = _find_row(rows, "regions[0]")
    assert result == "PASS", f"memberIncludesType match should PASS, got {result}: {detail}"
    assert all_pass, "all_pass should be True"


# ---------------------------------------------------------------------------
# R7. memberIncludesType — fails when none of the members has the named type
# ---------------------------------------------------------------------------

def test_regions_member_includes_type_fails():
    issue = _make_issue("issue_aabbcc000001", "style_changed", landmark="contentinfo")
    region = _make_region("region_abc123def456", "contentinfo", 0.86, ["issue_aabbcc000001"])
    dr = _diff_result(issues=[issue], regions=[region])
    exp = _expected(
        regions_spec={"required": [
            {"landmark": "contentinfo", "memberIncludesType": "broken_link"}
        ]}
    )
    all_pass, rows = evaluate_expected_issues(dr, exp)
    result, detail = _find_row(rows, "regions[0]")
    assert result == "FAIL", f"memberIncludesType miss should FAIL, got {result}: {detail}"
    assert not all_pass, "all_pass should be False"


# ---------------------------------------------------------------------------
# R8. maxTopLevelItems — PASSES when top-level <= cap
# ---------------------------------------------------------------------------

def test_max_top_level_items_passes():
    # 2 standalone issues + 1 cluster + 1 region = 4 top-level items
    issue_standalone1 = _make_issue("issue_sa000000001", "style_changed", landmark="main")
    issue_standalone2 = _make_issue("issue_sa000000002", "style_changed", landmark="main")
    issue_in_cluster = _make_issue("issue_cl000000001", "style_changed", landmark="main")
    issue_in_region = _make_issue("issue_rg000000001", "missing_text", landmark="contentinfo")
    cluster = _make_cluster("cluster_abc123def456", ["issue_cl000000001"])
    region = _make_region("region_abc123def456", "contentinfo", 0.86, ["issue_rg000000001"])
    dr = _diff_result(
        issues=[issue_standalone1, issue_standalone2, issue_in_cluster, issue_in_region],
        clusters=[cluster],
        regions=[region],
    )
    exp = _expected(max_top=5)
    all_pass, rows = evaluate_expected_issues(dr, exp)
    result, detail = _find_row(rows, "maxTopLevelItems")
    assert result == "PASS", f"Expected PASS (4 <= 5), got {result}: {detail}"
    assert all_pass, "all_pass should be True"
    assert "4 top-level" in detail, f"Detail should mention count: {detail}"


# ---------------------------------------------------------------------------
# R9. maxTopLevelItems — FAILS when over cap
# ---------------------------------------------------------------------------

def test_max_top_level_items_fails_over_cap():
    # 3 standalone + 0 clusters + 0 regions = 3 top-level, cap = 2
    issue1 = _make_issue("issue_sa000000001", "style_changed", landmark="main")
    issue2 = _make_issue("issue_sa000000002", "style_changed", landmark="main")
    issue3 = _make_issue("issue_sa000000003", "style_changed", landmark="main")
    dr = _diff_result(issues=[issue1, issue2, issue3])
    exp = _expected(max_top=2)
    all_pass, rows = evaluate_expected_issues(dr, exp)
    result, detail = _find_row(rows, "maxTopLevelItems")
    assert result == "FAIL", f"Expected FAIL (3 > 2), got {result}: {detail}"
    assert not all_pass, "all_pass should be False"


# ---------------------------------------------------------------------------
# R10. maxTopLevelItems — region member NOT double-counted as standalone
# ---------------------------------------------------------------------------

def test_max_top_level_items_no_double_count():
    """An issue in a region's memberIssueIds must NOT appear in standalone count."""
    issue_in_region = _make_issue("issue_rg000000001", "missing_text", landmark="contentinfo")
    issue_standalone = _make_issue("issue_sa000000001", "style_changed", landmark="main")
    region = _make_region("region_abc123def456", "contentinfo", 0.86, ["issue_rg000000001"])
    dr = _diff_result(
        issues=[issue_in_region, issue_standalone],
        regions=[region],
    )
    # 1 standalone + 0 clusters + 1 region = 2 top-level (NOT 3)
    exp = _expected(max_top=2)
    all_pass, rows = evaluate_expected_issues(dr, exp)
    result, detail = _find_row(rows, "maxTopLevelItems")
    assert result == "PASS", (
        f"Expected PASS (1 standalone, not 2 — region member excluded), "
        f"got {result}: {detail}"
    )
    assert all_pass, "all_pass should be True"
    assert "1 standalone" in detail, f"Standalone count should be 1: {detail}"


# ---------------------------------------------------------------------------
# R11. maxTopLevelItems — cluster member NOT double-counted as standalone
# ---------------------------------------------------------------------------

def test_max_top_level_items_cluster_member_not_standalone():
    """An issue in a cluster's issueIds must NOT appear in standalone count."""
    issue_in_cluster = _make_issue("issue_cl000000001", "style_changed", landmark="main")
    issue_standalone = _make_issue("issue_sa000000001", "style_changed", landmark="main")
    cluster = _make_cluster("cluster_abc123def456", ["issue_cl000000001"])
    dr = _diff_result(
        issues=[issue_in_cluster, issue_standalone],
        clusters=[cluster],
    )
    # 1 standalone + 1 cluster + 0 regions = 2 top-level
    exp = _expected(max_top=2)
    all_pass, rows = evaluate_expected_issues(dr, exp)
    result, detail = _find_row(rows, "maxTopLevelItems")
    assert result == "PASS", (
        f"Expected PASS (1 standalone, not 2 — cluster member excluded), "
        f"got {result}: {detail}"
    )
    assert all_pass, "all_pass should be True"
    assert "1 standalone" in detail, f"Standalone count should be 1: {detail}"


# ---------------------------------------------------------------------------
# R12. No regions spec in expected — engine ignores the regions field silently
# ---------------------------------------------------------------------------

def test_no_regions_spec_is_ignored():
    region = _make_region("region_abc123def456", "contentinfo", 0.86, [])
    dr = _diff_result(regions=[region])
    exp = _expected()  # no regions_spec key
    all_pass, rows = evaluate_expected_issues(dr, exp)
    # No regions[N] row should appear
    region_rows = [(c, r, d) for c, r, d in rows if c.startswith("regions[")]
    assert len(region_rows) == 0, f"Expected no regions rows, got: {region_rows}"
    assert all_pass, "all_pass should be True when no regions spec"


# ---------------------------------------------------------------------------
# R13. regions.required without exactlyOne — non-unique match is fine
# ---------------------------------------------------------------------------

def test_regions_no_exactlyone_multiple_matches_ok():
    """Without exactlyOne, two regions with the same landmark both pass."""
    issue1 = _make_issue("issue_aabbcc000001", "missing_text", landmark="contentinfo")
    issue2 = _make_issue("issue_aabbcc000002", "missing_link", landmark="contentinfo")
    region1 = _make_region("region_abc123def456", "contentinfo", 0.86, ["issue_aabbcc000001"])
    region2 = _make_region("region_abc123def457", "contentinfo", 0.75, ["issue_aabbcc000002"])
    dr = _diff_result(issues=[issue1, issue2], regions=[region1, region2])
    exp = _expected(
        regions_spec={"required": [
            {"landmark": "contentinfo", "minSaturation": 0.6}
        ]}
    )
    all_pass, rows = evaluate_expected_issues(dr, exp)
    result, detail = _find_row(rows, "regions[0]")
    assert result == "PASS", f"Without exactlyOne, multiple matches should PASS, got {result}: {detail}"
    assert all_pass, "all_pass should be True"


# ---------------------------------------------------------------------------
# R14. maxTopLevelItems — exactly at cap PASSES (boundary: top_level == cap uses <=)
# ---------------------------------------------------------------------------

def test_max_top_level_items_exact_boundary_passes():
    """top_level == cap must PASS because the check uses <=, not <."""
    # 3 standalone issues, cap = 3 → top_level == cap → PASS
    issue1 = _make_issue("issue_sa000000001", "style_changed", landmark="main")
    issue2 = _make_issue("issue_sa000000002", "style_changed", landmark="main")
    issue3 = _make_issue("issue_sa000000003", "style_changed", landmark="main")
    dr = _diff_result(issues=[issue1, issue2, issue3])
    exp = _expected(max_top=3)
    all_pass, rows = evaluate_expected_issues(dr, exp)
    result, detail = _find_row(rows, "maxTopLevelItems")
    assert result == "PASS", (
        f"Expected PASS when top_level == cap (boundary, <= not <), got {result}: {detail}"
    )
    assert all_pass, "all_pass should be True at exact boundary"
    assert "<= cap 3" in detail, f"Detail should confirm <= cap: {detail}"


# ---------------------------------------------------------------------------
# Standalone runner
# ---------------------------------------------------------------------------

if __name__ == "__main__":
    print("Running test_check_fixture_regions.py")
    print(f"  check-fixture.py: {TESTBED_DIR / 'check-fixture.py'}")
    print()

    tests = [
        ("test_regions_required_passes_single_match", test_regions_required_passes_single_match),
        ("test_regions_required_fails_no_match", test_regions_required_fails_no_match),
        ("test_regions_required_fails_exactlyone_two_matches", test_regions_required_fails_exactlyone_two_matches),
        ("test_regions_min_saturation_boundary_exact", test_regions_min_saturation_boundary_exact),
        ("test_regions_min_saturation_below_fails", test_regions_min_saturation_below_fails),
        ("test_regions_member_includes_type_passes", test_regions_member_includes_type_passes),
        ("test_regions_member_includes_type_fails", test_regions_member_includes_type_fails),
        ("test_max_top_level_items_passes", test_max_top_level_items_passes),
        ("test_max_top_level_items_fails_over_cap", test_max_top_level_items_fails_over_cap),
        ("test_max_top_level_items_no_double_count", test_max_top_level_items_no_double_count),
        ("test_max_top_level_items_cluster_member_not_standalone", test_max_top_level_items_cluster_member_not_standalone),
        ("test_no_regions_spec_is_ignored", test_no_regions_spec_is_ignored),
        ("test_regions_no_exactlyone_multiple_matches_ok", test_regions_no_exactlyone_multiple_matches_ok),
        ("test_max_top_level_items_exact_boundary_passes", test_max_top_level_items_exact_boundary_passes),
    ]

    for name, fn in tests:
        _test(name, fn)

    passed = sum(1 for _, ok, d in _RESULTS if ok and not d.startswith("SKIP"))
    failed = sum(1 for _, ok, _ in _RESULTS if not ok)
    skipped = sum(1 for _, ok, d in _RESULTS if ok and d.startswith("SKIP"))

    print(f"\n{passed} passed, {failed} failed, {skipped} skipped")
    if failed:
        sys.exit(1)
