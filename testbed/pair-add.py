#!/usr/bin/env python3
"""
pair-add.py — capture → privacy gate → freeze → scaffold a Tier-3 real-pair fixture.

Usage (add):
    python3 testbed/pair-add.py \\
        --case p01-hiya-number-registration \\
        --url-old https://old.example.com/page \\
        --url-new https://new.example.com/page \\
        [--profile content-structure] \\
        [--viewport desktop=1440x1000] \\
        [--hide ".analytics" --hide ".banner"] \\
        [--mask ".token-field"] \\
        [--matchy PATH] \\
        [--yes]

Usage (refresh):
    python3 testbed/pair-add.py --refresh --case p01-hiya-number-registration [--yes]

The only step that touches the network/browser is run_capture().
All other logic lives in freeze_and_scaffold(), which is directly testable with
a hand-built temp directory.

Architecture
------------
  run_capture(...)        — subprocess: matchy --self-check → <tmp>/<viewport>/
  freeze_and_scaffold(...)— all logic operating on an already-populated tmp_dir;
                            the test calls this directly.
  main()                  — arg parse, make tmpdir, call run_capture + freeze_and_scaffold.

Freeze is all-or-nothing: the privacy gate runs BEFORE pairs/<case>/ is created.
Nothing is written to the tracked tree if the gate raises PrivacyGateError.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

# ---------------------------------------------------------------------------
# Path constants (resolved once at import time)
# ---------------------------------------------------------------------------

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_DIR = SCRIPT_DIR.parent

DEFAULT_MATCHY = REPO_DIR / "target" / "release" / "matchy"

# Default locations for the tracked pairs tree and the ephemeral .runs dir.
# Tests override these via freeze_and_scaffold's pairs_dir/runs_dir params.
PAIRS_DIR = SCRIPT_DIR / "pairs"
RUNS_DIR = SCRIPT_DIR / ".runs"

PAIR_SCHEMA_PATH = SCRIPT_DIR / "schemas" / "pair.schema.json"
EXPECTED_ISSUES_SCHEMA_PATH = SCRIPT_DIR / "schemas" / "expected-issues.schema.json"

# ---------------------------------------------------------------------------
# Import privacy gate
# ---------------------------------------------------------------------------

sys.path.insert(0, str(SCRIPT_DIR))
import pair_privacy  # noqa: E402


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _sha256(path: Path) -> str:
    """Return the hex SHA-256 digest of a file's bytes."""
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _validate_schema(data: dict, schema_path: Path, label: str) -> None:
    """
    Validate data against schema_path using jsonschema.
    Raises SystemExit(2) with a clear message if validation fails.
    """
    import jsonschema

    if not schema_path.exists():
        print(f"ERROR: schema file not found: {schema_path}", file=sys.stderr)
        sys.exit(2)
    schema = json.loads(schema_path.read_text(encoding="utf-8"))
    validator_cls = jsonschema.validators.validator_for(schema)
    v = validator_cls(schema)
    errors = list(v.iter_errors(data))
    if errors:
        msgs = "; ".join(e.message for e in errors[:5])
        print(f"ERROR: {label} schema validation failed: {msgs}", file=sys.stderr)
        sys.exit(2)


def _check_url_reachable(url: str) -> bool:
    """Return True if the URL returns an HTTP response (any status)."""
    try:
        req = urllib.request.Request(url, method="HEAD")
        with urllib.request.urlopen(req, timeout=10):
            return True
    except Exception:
        return False


def _build_capture_flags(
    *,
    profile: str,
    viewport: str,
    hide: list[str],
    mask: list[str],
) -> list[str]:
    """Build the list of capture flags (excluding --old / --new / --out)."""
    flags: list[str] = []
    flags += ["--profile", profile]
    flags += ["--viewport", viewport]
    for sel in hide:
        flags += ["--hide", sel]
    for sel in mask:
        flags += ["--mask", sel]
    return flags


# ---------------------------------------------------------------------------
# Seed analyze — the real implementation used in production
# ---------------------------------------------------------------------------


def _real_seed_analyze(
    *,
    matchy_bin: Path,
    old_bundle_path: Path,
    new_bundle_path: Path,
    profile: str,
    runs_dir: Path,
    case_id: str,
) -> dict | None:
    """
    Run `matchy analyze` on the frozen bundles and write diff-result.json into
    runs_dir/<case_id>/diff-result.json.

    Returns the diff-result dict if successful, else None (with a WARNING).
    The matchy binary is optional — if absent, we skip and warn.
    """
    if not matchy_bin.exists():
        print(
            f"WARNING: matchy binary not found at {matchy_bin} — skipping analyze seed.\n"
            f"  The expected-issues.json stub will use status='warn'.\n"
            f"  Run check-pair.py later once matchy is built.",
            file=sys.stderr,
        )
        return None

    out_dir = runs_dir / case_id
    out_dir.mkdir(parents=True, exist_ok=True)

    cmd = [
        str(matchy_bin),
        "analyze",
        "--old-bundle", str(old_bundle_path),
        "--new-bundle", str(new_bundle_path),
        "--out", str(out_dir),
        "--profile", profile,
        "--fail-on", "never",
    ]
    print(f"  Seeding analyze: {' '.join(cmd)}")
    result = subprocess.run(cmd, capture_output=True, text=True)
    rc = result.returncode

    if rc not in (0, 1):
        print(
            f"WARNING: matchy analyze exited {rc} — diff-result.json may not exist.\n"
            f"  stderr: {result.stderr[:500]}",
            file=sys.stderr,
        )
        return None

    dr_path = out_dir / "diff-result.json"
    if not dr_path.exists():
        print(
            f"WARNING: matchy analyze exited {rc} but {dr_path} not found — skipping.",
            file=sys.stderr,
        )
        return None

    try:
        return json.loads(dr_path.read_text(encoding="utf-8"))
    except Exception as exc:
        print(f"WARNING: could not parse diff-result.json: {exc}", file=sys.stderr)
        return None


# ---------------------------------------------------------------------------
# run_capture — the only step that touches network/browser
# ---------------------------------------------------------------------------


def run_capture(
    matchy_bin: Path,
    url_old: str,
    url_new: str,
    out_dir: Path,
    profile: str,
    viewport: str,
    hide: list[str],
    mask: list[str],
) -> None:
    """
    Run `matchy --old URL_OLD --new URL_NEW --out <out_dir> --self-check ...`.

    Writes, under out_dir/<viewport_name>/:
        old.bundle.json, new.bundle.json
        old.png, new.png, old-vp.png, new-vp.png
        old-selfcheck.{bundle.json,png,-vp.png}

    And writes out_dir/self-check.json.

    Aborts with a clear error if a URL is unreachable or matchy exits non-zero.
    """
    if not matchy_bin.exists():
        print(
            f"ERROR: matchy binary not found at {matchy_bin}\n"
            f"  Build it first:  cargo build --release --bin matchy",
            file=sys.stderr,
        )
        sys.exit(2)

    # Pre-flight reachability checks
    for label, url in [("old", url_old), ("new", url_new)]:
        print(f"  Checking {label} URL is reachable: {url} ...")
        if not _check_url_reachable(url):
            print(
                f"ERROR: {label} URL is not reachable: {url}\n"
                f"  Ensure the server is running before calling pair-add.",
                file=sys.stderr,
            )
            sys.exit(2)

    cmd = [
        str(matchy_bin),
        "--old", url_old,
        "--new", url_new,
        "--out", str(out_dir),
        "--self-check",
        "--profile", profile,
        "--viewport", viewport,
    ]
    for sel in hide:
        cmd += ["--hide", sel]
    for sel in mask:
        cmd += ["--mask", sel]

    print(f"  Running capture: {' '.join(cmd)}")
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print(
            f"ERROR: matchy capture exited {result.returncode}.\n"
            f"  stdout: {result.stdout[:1000]}\n"
            f"  stderr: {result.stderr[:1000]}",
            file=sys.stderr,
        )
        sys.exit(2)


# ---------------------------------------------------------------------------
# freeze_and_scaffold — the load-bearing logic (fully testable without capture)
# ---------------------------------------------------------------------------


def freeze_and_scaffold(
    *,
    tmp_dir: Path,
    case_id: str,
    viewport_name: str,
    url_old: str,
    url_new: str,
    profile: str,
    capture_flags: list[str],
    pairs_dir: Path,
    runs_dir: Path,
    assume_yes: bool = False,
    seed_analyze=None,
    gate=None,
) -> None:
    """
    Operate on an already-populated tmp_dir (written by run_capture or hand-built
    in tests) to:
      1. Locate required artifacts.
      2. Run the privacy gate BEFORE any write to pairs_dir.
      3. Freeze (explicit allowlist) preserving <viewport>/ nesting.
      4. Compute SHA-256 of frozen bundles.
      5. Read self-check.json for volatile_capture warnings.
      6. Write pair.json scaffold (schema-valid).
      7. Seed matchy analyze.
      8. Write expected-issues.json STUB (required/forbidden always empty).
      9. Validate pair.json against pair.schema.json.

    Parameters
    ----------
    tmp_dir         : Path — the temp directory populated by run_capture.
    case_id         : str  — e.g. "p01-hiya-number-registration".
    viewport_name   : str  — e.g. "desktop".
    url_old         : str  — requested old URL.
    url_new         : str  — requested new URL.
    profile         : str  — capture profile name.
    capture_flags   : list[str] — the CLI flags passed to matchy (for pair.json.captureFlags).
    pairs_dir       : Path — root of the tracked pairs tree (real: testbed/pairs/).
    runs_dir        : Path — root of the ephemeral .runs tree (real: testbed/.runs/).
    assume_yes      : bool — skip interactive privacy gate prompts.
    seed_analyze    : callable | None — injectable; defaults to _real_seed_analyze.
    gate            : callable | None — injectable; defaults to pair_privacy.run_gate.
    """
    if seed_analyze is None:
        seed_analyze = _real_seed_analyze
    if gate is None:
        gate = pair_privacy.run_gate

    vp_tmp = tmp_dir / viewport_name

    # ------------------------------------------------------------------
    # 1. Locate required artifacts in tmp_dir/<viewport_name>/
    # ------------------------------------------------------------------
    REQUIRED_FILES = [
        "old.bundle.json",
        "new.bundle.json",
        "old.png",
        "new.png",
        "old-vp.png",
        "new-vp.png",
    ]
    for fname in REQUIRED_FILES:
        fpath = vp_tmp / fname
        if not fpath.exists():
            print(
                f"ERROR: required artifact missing from temp dir: {fpath}\n"
                f"  Ensure run_capture completed successfully.",
                file=sys.stderr,
            )
            sys.exit(2)

    old_bundle_tmp = vp_tmp / "old.bundle.json"
    new_bundle_tmp = vp_tmp / "new.bundle.json"
    png_paths_tmp = [
        vp_tmp / "old.png",
        vp_tmp / "new.png",
        vp_tmp / "old-vp.png",
        vp_tmp / "new-vp.png",
    ]

    # ------------------------------------------------------------------
    # 2. PRIVACY GATE — BEFORE any write to pairs_dir
    # ------------------------------------------------------------------
    print("  Running privacy gate ...")
    gate(
        old_bundle_path=old_bundle_tmp,
        new_bundle_path=new_bundle_tmp,
        png_paths=png_paths_tmp,
        urls=[url_old, url_new],
        assume_yes=assume_yes,
    )
    print("  Privacy gate: passed.")

    # ------------------------------------------------------------------
    # 3. FREEZE via EXPLICIT ALLOWLIST (preserve <viewport>/ nesting)
    #    pairs/<case>/<viewport>/
    #    ONLY: old.bundle.json, new.bundle.json, old.png, new.png, old-vp.png, new-vp.png
    #    NEVER: old-selfcheck.{bundle.json,png,-vp.png} or self-check.json
    # ------------------------------------------------------------------
    case_dir = pairs_dir / case_id
    vp_frozen = case_dir / viewport_name
    vp_frozen.mkdir(parents=True, exist_ok=True)

    FREEZE_ALLOWLIST = [
        "old.bundle.json",
        "new.bundle.json",
        "old.png",
        "new.png",
        "old-vp.png",
        "new-vp.png",
    ]
    for fname in FREEZE_ALLOWLIST:
        shutil.copy2(vp_tmp / fname, vp_frozen / fname)

    # F1 guard: assert no selfcheck artifacts were frozen
    for frozen_file in vp_frozen.iterdir():
        if "selfcheck" in frozen_file.name or "self-check" in frozen_file.name:
            # This should be impossible given the allowlist, but defend in depth.
            frozen_file.unlink()
            print(
                f"WARNING: selfcheck artifact unexpectedly found in frozen dir and removed: {frozen_file}",
                file=sys.stderr,
            )

    print(f"  Frozen {len(FREEZE_ALLOWLIST)} files into {vp_frozen}")

    # ------------------------------------------------------------------
    # 4. Compute SHA-256 of the two FROZEN bundle files
    # ------------------------------------------------------------------
    old_frozen = vp_frozen / "old.bundle.json"
    new_frozen = vp_frozen / "new.bundle.json"
    old_sha = _sha256(old_frozen)
    new_sha = _sha256(new_frozen)

    # ------------------------------------------------------------------
    # 5. Read self-check.json for volatile_capture warnings (then discard)
    # ------------------------------------------------------------------
    known_drift: list[str] = []
    self_check_path = tmp_dir / "self-check.json"
    if self_check_path.exists():
        try:
            sc = json.loads(self_check_path.read_text(encoding="utf-8"))
            # Look for volatile_capture warnings anywhere in the structure.
            # self-check.json shape varies; we scan for the key "volatile_capture".
            known_drift = _extract_volatile_capture_warnings(sc)
            if known_drift:
                print(f"  volatile_capture warnings detected: {known_drift}")
        except Exception as exc:
            print(f"  WARNING: could not parse self-check.json: {exc}", file=sys.stderr)
        # Do NOT copy self-check.json into pairs/.

    # ------------------------------------------------------------------
    # 6. Read bundle fields needed for pair.json
    # ------------------------------------------------------------------
    old_bundle = json.loads(old_frozen.read_text(encoding="utf-8"))
    new_bundle = json.loads(new_frozen.read_text(encoding="utf-8"))

    old_final_url = old_bundle.get("page", {}).get("finalUrl", url_old)
    new_final_url = new_bundle.get("page", {}).get("finalUrl", url_new)
    old_captured_at = old_bundle.get("capturedAt", "")
    new_captured_at = new_bundle.get("capturedAt", "")
    old_chromium = old_bundle.get("environment", {}).get("chromiumBuild", "")
    new_chromium = new_bundle.get("environment", {}).get("chromiumBuild", "")

    # viewport.name from the bundle (may differ from viewport_name if matchy renames)
    # We use viewport_name (the subdir) as the authoritative "viewport" in pair.json
    # because that is what analyze resolves paths relative to.

    # ------------------------------------------------------------------
    # 7. Seed matchy analyze
    # ------------------------------------------------------------------
    diff_result = seed_analyze(
        matchy_bin=DEFAULT_MATCHY,
        old_bundle_path=old_frozen,
        new_bundle_path=new_frozen,
        profile=profile,
        runs_dir=runs_dir,
        case_id=case_id,
    )

    # ------------------------------------------------------------------
    # 8. Determine status for the expected-issues.json stub
    # ------------------------------------------------------------------
    # R3 (load-bearing): required and forbidden are ALWAYS empty in the stub.
    # Never auto-populate from the seeded diff-result.
    stub_status = "warn"
    if diff_result is not None:
        stub_status = diff_result.get("status", "warn")

    # ------------------------------------------------------------------
    # Write pair.json scaffold
    # ------------------------------------------------------------------
    pair_json: dict = {
        "caseId": case_id,
        "description": (
            "SCAFFOLD — describe what this pair demonstrates, "
            "then edit before committing."
        ),
        "demonstrates": "false-negative",
        "discoveredVia": "pair-add",
        "goals": [],
        "profile": profile,
        "viewport": viewport_name,
        "old": {
            "url": url_old,
            "finalUrl": old_final_url,
            "capturedAt": old_captured_at,
            "sha256": old_sha,
            "chromiumBuild": old_chromium,
        },
        "new": {
            "url": url_new,
            "finalUrl": new_final_url,
            "capturedAt": new_captured_at,
            "sha256": new_sha,
            "chromiumBuild": new_chromium,
        },
        "captureFlags": capture_flags,
        "baseline": None,
        "frozen": True,
        "refreshPolicy": "never",
        "expectedState": "red",
    }
    if known_drift:
        pair_json["knownDrift"] = known_drift

    pair_json_path = case_dir / "pair.json"
    pair_json_path.write_text(
        json.dumps(pair_json, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(f"  Wrote {pair_json_path}")

    # ------------------------------------------------------------------
    # Write expected-issues.json STUB
    # R3: required and forbidden MUST be empty — never auto-populated.
    # ------------------------------------------------------------------
    stub = {
        "status": stub_status,
        "required": [],
        "forbidden": [],
        "notes": (
            "STUB — author the intent here. "
            "Do NOT copy diff-result issues: the current output is presumed wrong "
            "(that is why this pair was added). "
            "Put what matchy SHOULD emit in `required`; "
            "pin noise/knownDrift via `forbidden`/`maxIssues`."
        ),
    }
    expected_issues_path = case_dir / "expected-issues.json"
    expected_issues_path.write_text(
        json.dumps(stub, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(f"  Wrote {expected_issues_path}")

    # ------------------------------------------------------------------
    # 9. Validate pair.json against pair.schema.json
    # ------------------------------------------------------------------
    _validate_schema(pair_json, PAIR_SCHEMA_PATH, "pair.json scaffold")
    print("  pair.json schema: valid")
    print()
    print(f"Fixture ready: {case_dir}")
    print(
        f"  Next steps:\n"
        f"    1. Run `matchy explain` on the frozen bundles to classify the diff.\n"
        f"    2. Edit {expected_issues_path.name} — set `required` / `forbidden` / `status`.\n"
        f"    3. Update pair.json: set `description`, `demonstrates`, `goals`, `expectedState`.\n"
        f"    4. Commit the fixture (git add testbed/pairs/{case_id}/)."
    )


# ---------------------------------------------------------------------------
# Helper: extract volatile_capture warnings from self-check.json
# ---------------------------------------------------------------------------


def _extract_volatile_capture_warnings(sc: dict) -> list[str]:
    """
    Extract 'volatile_capture' warning strings from a self-check.json dict.
    The exact shape is not publicly documented; we search for the key recursively
    up to 3 levels deep to be robust against minor changes.
    """
    warnings: list[str] = []

    def _scan(obj, depth: int) -> None:
        if depth > 4:
            return
        if isinstance(obj, dict):
            for k, v in obj.items():
                if k == "volatile_capture":
                    if isinstance(v, str):
                        warnings.append(v)
                    elif isinstance(v, list):
                        for item in v:
                            if isinstance(item, str):
                                warnings.append(item)
                else:
                    _scan(v, depth + 1)
        elif isinstance(obj, list):
            for item in obj:
                _scan(item, depth + 1)

    _scan(sc, 0)
    return warnings


# ---------------------------------------------------------------------------
# --refresh mode helpers
# ---------------------------------------------------------------------------


def _do_refresh(
    *,
    case_id: str,
    matchy_bin: Path,
    pairs_dir: Path,
    runs_dir: Path,
    assume_yes: bool,
    seed_analyze,
    gate,
) -> None:
    """
    Re-capture using recorded captureFlags and re-freeze.
    Leaves expected-issues.json and expectedState (and demonstrates/description/
    goals/knownDrift) UNTOUCHED.
    """
    case_dir = pairs_dir / case_id
    pair_json_path = case_dir / "pair.json"
    if not pair_json_path.exists():
        print(f"ERROR: pair.json not found: {pair_json_path}", file=sys.stderr)
        sys.exit(2)

    pair = json.loads(pair_json_path.read_text(encoding="utf-8"))
    url_old = pair["old"]["url"]
    url_new = pair["new"]["url"]
    viewport_name = pair["viewport"]
    profile = pair.get("profile", "content-structure")
    capture_flags = pair.get("captureFlags", [])

    # Reconstruct hide/mask from captureFlags
    hide: list[str] = []
    mask: list[str] = []
    viewport_arg = f"{viewport_name}=1440x1000"  # fallback

    i = 0
    while i < len(capture_flags):
        if capture_flags[i] == "--hide" and i + 1 < len(capture_flags):
            hide.append(capture_flags[i + 1])
            i += 2
        elif capture_flags[i] == "--mask" and i + 1 < len(capture_flags):
            mask.append(capture_flags[i + 1])
            i += 2
        elif capture_flags[i] == "--viewport" and i + 1 < len(capture_flags):
            viewport_arg = capture_flags[i + 1]
            i += 2
        else:
            i += 1

    print(f"=== pair-add --refresh: {case_id} ===")
    print(f"  old URL : {url_old}")
    print(f"  new URL : {url_new}")
    print(f"  viewport: {viewport_name}  ({viewport_arg})")
    print()

    with tempfile.TemporaryDirectory(prefix="pair-add-refresh-") as _td:
        tmp_dir = Path(_td)

        run_capture(
            matchy_bin=matchy_bin,
            url_old=url_old,
            url_new=url_new,
            out_dir=tmp_dir,
            profile=profile,
            viewport=viewport_arg,
            hide=hide,
            mask=mask,
        )

        vp_tmp = tmp_dir / viewport_name
        old_bundle_tmp = vp_tmp / "old.bundle.json"
        new_bundle_tmp = vp_tmp / "new.bundle.json"
        png_paths_tmp = [
            vp_tmp / "old.png",
            vp_tmp / "new.png",
            vp_tmp / "old-vp.png",
            vp_tmp / "new-vp.png",
        ]

        # Privacy gate — must re-run on refreshed content
        print("  Running privacy gate (refresh) ...")
        gate(
            old_bundle_path=old_bundle_tmp,
            new_bundle_path=new_bundle_tmp,
            png_paths=png_paths_tmp,
            urls=[url_old, url_new],
            assume_yes=assume_yes,
        )
        print("  Privacy gate: passed.")

        # Overwrite frozen bundles + PNGs
        vp_frozen = case_dir / viewport_name
        FREEZE_ALLOWLIST = [
            "old.bundle.json",
            "new.bundle.json",
            "old.png",
            "new.png",
            "old-vp.png",
            "new-vp.png",
        ]
        for fname in FREEZE_ALLOWLIST:
            shutil.copy2(vp_tmp / fname, vp_frozen / fname)

        # Recompute SHA-256
        old_sha = _sha256(vp_frozen / "old.bundle.json")
        new_sha = _sha256(vp_frozen / "new.bundle.json")

        # Read updated bundle fields
        old_bundle = json.loads((vp_frozen / "old.bundle.json").read_text(encoding="utf-8"))
        new_bundle = json.loads((vp_frozen / "new.bundle.json").read_text(encoding="utf-8"))

        old_final_url = old_bundle.get("page", {}).get("finalUrl", url_old)
        new_final_url = new_bundle.get("page", {}).get("finalUrl", url_new)
        old_captured_at = old_bundle.get("capturedAt", "")
        new_captured_at = new_bundle.get("capturedAt", "")
        old_chromium = old_bundle.get("environment", {}).get("chromiumBuild", "")
        new_chromium = new_bundle.get("environment", {}).get("chromiumBuild", "")

    # Update pair.json — only the provenance fields (NOT expectedState / demonstrates /
    # description / goals / knownDrift)
    pair["old"]["finalUrl"] = old_final_url
    pair["old"]["capturedAt"] = old_captured_at
    pair["old"]["sha256"] = old_sha
    pair["old"]["chromiumBuild"] = old_chromium
    pair["new"]["finalUrl"] = new_final_url
    pair["new"]["capturedAt"] = new_captured_at
    pair["new"]["sha256"] = new_sha
    pair["new"]["chromiumBuild"] = new_chromium

    pair_json_path.write_text(
        json.dumps(pair, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(f"  Refreshed pair.json: {pair_json_path}")

    # Re-seed analyze
    seed_analyze(
        matchy_bin=matchy_bin,
        old_bundle_path=vp_frozen / "old.bundle.json",
        new_bundle_path=vp_frozen / "new.bundle.json",
        profile=profile,
        runs_dir=runs_dir,
        case_id=case_id,
    )

    # Validate refreshed pair.json
    _validate_schema(pair, PAIR_SCHEMA_PATH, "pair.json (refreshed)")
    print("  pair.json schema: valid (refresh)")
    print()
    print(f"Refresh complete: {case_dir}")


# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="pair-add.py",
        description="Capture → privacy gate → freeze → scaffold a Tier-3 real-pair fixture.",
    )
    parser.add_argument(
        "--case", required=True, dest="case_id",
        help="case identifier (e.g. p01-hiya-number-registration)",
    )
    parser.add_argument("--url-old", default=None, help="old page URL (add mode)")
    parser.add_argument("--url-new", default=None, help="new page URL (add mode)")
    parser.add_argument(
        "--profile", default="content-structure",
        help="capture profile (default: content-structure)",
    )
    parser.add_argument(
        "--viewport", default="desktop=1440x1000",
        help="viewport name=WxH (default: desktop=1440x1000)",
    )
    parser.add_argument(
        "--hide", action="append", default=[], metavar="SEL",
        help="CSS selector to hide (repeatable)",
    )
    parser.add_argument(
        "--mask", action="append", default=[], metavar="SEL",
        help="CSS selector to mask (repeatable)",
    )
    parser.add_argument(
        "--matchy", default=None,
        help=f"path to matchy binary (default: {DEFAULT_MATCHY})",
    )
    parser.add_argument(
        "--yes", action="store_true",
        help="skip interactive privacy gate prompts (trusted/already-reviewed re-freeze only)",
    )
    parser.add_argument(
        "--refresh", action="store_true",
        help="re-capture using recorded captureFlags; leave expected-issues.json untouched",
    )
    # Internal overrides for testing / CI
    parser.add_argument("--pairs-dir", default=None, dest="pairs_dir", help=argparse.SUPPRESS)
    parser.add_argument("--runs-dir", default=None, dest="runs_dir", help=argparse.SUPPRESS)

    args = parser.parse_args()

    matchy_bin = Path(args.matchy).resolve() if args.matchy else DEFAULT_MATCHY
    pairs_dir = Path(args.pairs_dir).resolve() if args.pairs_dir else PAIRS_DIR
    runs_dir = Path(args.runs_dir).resolve() if args.runs_dir else RUNS_DIR

    # Parse viewport name (the NAME part before '=')
    if "=" in args.viewport:
        viewport_name = args.viewport.split("=", 1)[0]
    else:
        viewport_name = args.viewport

    if args.refresh:
        _do_refresh(
            case_id=args.case_id,
            matchy_bin=matchy_bin,
            pairs_dir=pairs_dir,
            runs_dir=runs_dir,
            assume_yes=args.yes,
            seed_analyze=_real_seed_analyze,
            gate=pair_privacy.run_gate,
        )
        return

    # Add mode
    if not args.url_old or not args.url_new:
        print(
            "ERROR: --url-old and --url-new are required in add mode.",
            file=sys.stderr,
        )
        sys.exit(2)

    capture_flags = _build_capture_flags(
        profile=args.profile,
        viewport=args.viewport,
        hide=args.hide,
        mask=args.mask,
    )

    print(f"=== pair-add: {args.case_id} ===")
    print(f"  old URL : {args.url_old}")
    print(f"  new URL : {args.url_new}")
    print(f"  viewport: {viewport_name}  ({args.viewport})")
    print(f"  profile : {args.profile}")
    print()

    with tempfile.TemporaryDirectory(prefix="pair-add-") as _td:
        tmp_dir = Path(_td)

        run_capture(
            matchy_bin=matchy_bin,
            url_old=args.url_old,
            url_new=args.url_new,
            out_dir=tmp_dir,
            profile=args.profile,
            viewport=args.viewport,
            hide=args.hide,
            mask=args.mask,
        )

        freeze_and_scaffold(
            tmp_dir=tmp_dir,
            case_id=args.case_id,
            viewport_name=viewport_name,
            url_old=args.url_old,
            url_new=args.url_new,
            profile=args.profile,
            capture_flags=capture_flags,
            pairs_dir=pairs_dir,
            runs_dir=runs_dir,
            assume_yes=args.yes,
        )


if __name__ == "__main__":
    main()
