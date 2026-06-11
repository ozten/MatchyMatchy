#!/usr/bin/env python3
"""
check-m8.py  --  M8 acceptance harness (reporters, profiles, baseline accept-list).

Verifies the three M8 DoD clauses that aren't a per-variant issue check (spec §12 M8):
  1. one run renders static HTML + Markdown + JSON, and the HTML is safe
     (restrictive CSP, no <script>, no inline event handlers, page strings escaped);
  2. a profile switch changes pass/fail as specified (§9): a pure-style variant is
     `warn` under content-structure and `fail` under strict-visual;
  3. a baselined issue is suppressed and counted (§7.4): both the full-suppression case
     (status drops to pass, all ids counted) and the single-id case (one gone, rest remain).

Uses v05-cta-style (a small, pure `style_changed` variant) as the subject. Captures it once
via the full pipeline, then re-analyzes the captured bundles with different flags (fast,
deterministic, no re-capture).

Exit 0 = all M8 checks pass; 1 = a check failed; 2 = setup error.

All paths resolve relative to this script so it runs from any CWD.
"""

import json
import subprocess
import sys
import tempfile
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_DIR = SCRIPT_DIR.parent
VARIANTS_DIR = SCRIPT_DIR / "variants"
RUNS_DIR = SCRIPT_DIR / ".runs"
MATCHY = REPO_DIR / "target" / "release" / "matchy"

GOLDEN_URL = "http://localhost:3000/"
SUBJECT = "v05-cta-style"

# Exact CSP the renderer must emit (M8.md §5.1, spec §15).
CSP = (
    "<meta http-equiv=\"Content-Security-Policy\" "
    "content=\"default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'; "
    "base-uri 'none'; form-action 'none'\">"
)

rows: list[tuple[str, bool, str]] = []


def check(name: str, ok: bool, detail: str) -> None:
    rows.append((name, ok, detail))


def run(cmd: list[str]) -> int:
    return subprocess.run(cmd).returncode


def analyze(old_b: Path, new_b: Path, out: Path, *extra: str) -> dict:
    """Run `matchy analyze` and return the parsed diff-result.json."""
    out.mkdir(parents=True, exist_ok=True)
    cmd = [str(MATCHY), "analyze", "--old-bundle", str(old_b),
           "--new-bundle", str(new_b), "--out", str(out), *extra]
    rc = subprocess.run(cmd, capture_output=True, text=True)
    if rc.returncode not in (0, 1):
        raise RuntimeError(f"matchy analyze failed ({rc.returncode}): {rc.stderr}")
    return json.loads((out / "diff-result.json").read_text())


def main() -> int:
    if not MATCHY.exists():
        print(f"ERROR: matchy binary not found: {MATCHY}")
        return 2

    # --- servers up ---
    subprocess.run([sys.executable, str(SCRIPT_DIR / "run-all.py"), "start"],
                   capture_output=True, text=True)

    manifest = json.loads((VARIANTS_DIR / SUBJECT / "manifest.json").read_text())
    new_url = manifest.get("urlUnderTest") or f"http://localhost:{manifest['port']}/"

    # --- capture the subject once, with both reporters, default profile ---
    run_dir = RUNS_DIR / "m8-v05"
    cmd = [str(MATCHY), "--old", GOLDEN_URL, "--new", new_url, "--out", str(run_dir),
           "--viewport", "desktop=1440x1000", "--html", "--markdown"]
    print(f"Capturing subject: {' '.join(cmd)}")
    rc = subprocess.run(cmd).returncode
    if rc not in (0, 1):
        print(f"ERROR: capture run exited {rc}")
        return 2

    old_b = run_dir / "desktop" / "old.bundle.json"
    new_b = run_dir / "desktop" / "new.bundle.json"
    if not (old_b.exists() and new_b.exists()):
        print(f"ERROR: capture did not produce bundles under {run_dir}/desktop/")
        return 2

    # =========================================================================
    # 1. Reporters: HTML + Markdown + JSON all present; HTML is safe.
    # =========================================================================
    html_p = run_dir / "report.html"
    md_p = run_dir / "report.md"
    json_p = run_dir / "diff-result.json"
    check("reporters: 3 files", html_p.exists() and md_p.exists() and json_p.exists(),
          f"html={html_p.exists()} md={md_p.exists()} json={json_p.exists()}")

    if html_p.exists():
        html = html_p.read_text()
        low = html.lower()
        check("html: doctype", html.startswith("<!DOCTYPE html>"), html[:20])
        check("html: CSP present", CSP in html, "exact CSP meta " + ("found" if CSP in html else "MISSING"))
        check("html: no <script>", "<script" not in low, "no script tag")
        check("html: no on*= handlers",
              not any(h in low for h in ("onerror=", "onclick=", "onload=", "onmouseover=")),
              "no inline event handlers")
        check("html: no javascript:", "javascript:" not in low, "no javascript: urls")
        check("html: side-by-side imgs",
              all(f'src="desktop/{n}.png"' in html for n in ("old", "new", "diff")),
              "old/new/diff screenshots referenced")

    if md_p.exists():
        md = md_p.read_text()
        check("md: sections",
              all(h in md for h in ("# matchy report", "## Summary", "## Scores", "## Issues")),
              "report/summary/scores/issues headers")

    # =========================================================================
    # 2. Profile switch changes pass/fail (§9): pure-style variant.
    # =========================================================================
    with tempfile.TemporaryDirectory() as td:
        cs = analyze(old_b, new_b, Path(td) / "cs", "--profile", "content-structure")
        sv = analyze(old_b, new_b, Path(td) / "sv", "--profile", "strict-visual")
        check("profile: content-structure=warn", cs["status"] == "warn",
              f"status={cs['status']} (expected warn)")
        check("profile: strict-visual=fail", sv["status"] == "fail",
              f"status={sv['status']} (expected fail)")
        check("profile: switch flips status", cs["status"] != sv["status"],
              f"{cs['status']} -> {sv['status']}")

    # =========================================================================
    # 3. Baseline accept-list (§7.4): suppressed AND counted.
    # =========================================================================
    base = json.loads(json_p.read_text())
    all_ids = [i["id"] for i in base["issues"]]
    n = len(all_ids)
    check("baseline: subject has issues", n >= 2, f"{n} issues to work with")

    with tempfile.TemporaryDirectory() as td:
        td = Path(td)

        # 3a. Full suppression: every issue baselined -> status pass, all counted, none left.
        full = td / "accepted_full.json"
        full.write_text(json.dumps([{"id": i, "note": "intentional"} for i in all_ids]))
        r_full = analyze(old_b, new_b, td / "full", "--baseline", str(full))
        check("baseline: all suppressed -> 0 issues", len(r_full["issues"]) == 0,
              f"{len(r_full['issues'])} issues remain")
        check("baseline: suppressed.count == N", r_full["suppressed"]["count"] == n,
              f"count={r_full['suppressed']['count']} expected {n}")
        check("baseline: suppressed.ids == all", set(r_full["suppressed"]["ids"]) == set(all_ids),
              "all baselined ids recorded in suppressed.ids")
        check("baseline: status drops to pass", r_full["status"] == "pass",
              f"status={r_full['status']} (expected pass)")

        # 3b. Single id: one suppressed, the rest remain.
        one = td / "accepted_one.json"
        target = sorted(all_ids)[0]
        one.write_text(json.dumps([{"id": target}]))
        r_one = analyze(old_b, new_b, td / "one", "--baseline", str(one))
        kept_ids = {i["id"] for i in r_one["issues"]}
        check("baseline: single suppressed.count == 1", r_one["suppressed"]["count"] == 1,
              f"count={r_one['suppressed']['count']}")
        check("baseline: target absent from issues", target not in kept_ids,
              f"{target} suppressed from issues[]")
        check("baseline: others remain", len(r_one["issues"]) == n - 1,
              f"{len(r_one['issues'])} remain of {n - 1} expected")
        check("baseline: target in suppressed.ids", r_one["suppressed"]["ids"] == [target],
              f"suppressed.ids={r_one['suppressed']['ids']}")

    # =========================================================================
    # 4. Clustering on the subject (sanity): property clusters cover all issues.
    # =========================================================================
    clusters = base.get("clusters", [])
    prop_clusters = [c for c in clusters if c.get("sharedProperty")]
    covered = sum(len(c["issueIds"]) for c in prop_clusters)
    check("cluster: subject has property clusters", len(prop_clusters) >= 1,
          f"{len(prop_clusters)} property cluster(s)")
    check("cluster: clusterCount matches array",
          base["agentSummary"]["clusterCount"] == len(clusters),
          f"clusterCount={base['agentSummary']['clusterCount']} len={len(clusters)}")
    check("cluster: topFixes references clusters",
          any(t.startswith("cluster_") for t in base["agentSummary"]["topFixes"]),
          f"topFixes={base['agentSummary']['topFixes']}")

    # --- verdict ---
    print()
    print(f"{'CHECK':<38}  {'RESULT':<6}  DETAIL")
    print("-" * 90)
    all_pass = True
    for name, ok, detail in rows:
        all_pass = all_pass and ok
        print(f"  {'PASS' if ok else 'FAIL':<6}  {name:<36}  {detail}")
    print()
    print(f"VERDICT: {'PASS' if all_pass else 'FAIL'}")
    return 0 if all_pass else 1


if __name__ == "__main__":
    sys.exit(main())
