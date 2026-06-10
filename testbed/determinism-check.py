#!/usr/bin/env python3
"""
determinism-check.py  --  verify byte-identical re-analysis of capture bundles

Usage:
    python3 testbed/determinism-check.py <variant-name> [--matchy <path>]

Locates the two desktop bundles (old.bundle.json, new.bundle.json) under
    testbed/.runs/<variant>/desktop/
then runs:
    matchy analyze --old-bundle A --new-bundle B --out <tmp1>
    matchy analyze --old-bundle A --new-bundle B --out <tmp2>
Strips the "runId" key from both diff-result.json files (parse JSON, delete
the key, re-serialize with json.dumps preserving insertion order), and
byte-compares the two results.

PASS = byte-identical.  Exit 0/1.

All paths are resolved relative to this script's own location so the script
works from any CWD.
"""

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------
SCRIPT_DIR = Path(__file__).resolve().parent
REPO_DIR = SCRIPT_DIR.parent
RUNS_DIR = SCRIPT_DIR / ".runs"
DEFAULT_MATCHY = REPO_DIR / "target" / "release" / "matchy"

EXCLUDED_KEYS = {"runId", "capturedAt"}


def _strip_excluded(data: dict) -> dict:
    """Recursively remove excluded keys, returning a new structure."""
    if isinstance(data, dict):
        return {k: _strip_excluded(v) for k, v in data.items() if k not in EXCLUDED_KEYS}
    if isinstance(data, list):
        return [_strip_excluded(item) for item in data]
    return data


def _canonical_bytes(path: Path) -> bytes:
    """Load JSON, strip excluded keys, re-serialize preserving insertion order."""
    data = json.loads(path.read_text())
    stripped = _strip_excluded(data)
    return json.dumps(stripped, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def _find_bundles(run_dir: Path) -> tuple[Path, Path]:
    """
    Locate old.bundle.json and new.bundle.json under the desktop subdir of run_dir.
    Returns (old_bundle, new_bundle).
    Raises FileNotFoundError if not found.
    """
    desktop_dir = run_dir / "desktop"
    if not desktop_dir.exists():
        raise FileNotFoundError(f"Desktop capture dir not found: {desktop_dir}")

    old_bundle = desktop_dir / "old.bundle.json"
    new_bundle = desktop_dir / "new.bundle.json"

    for p in (old_bundle, new_bundle):
        if not p.exists():
            raise FileNotFoundError(f"Bundle not found: {p}")

    return old_bundle, new_bundle


def main() -> int:
    parser = argparse.ArgumentParser(
        prog="determinism-check.py",
        description="Verify byte-identical re-analysis (matchy analyze) for a variant.",
    )
    parser.add_argument("variant", help="variant name (e.g. v02-banner-added)")
    parser.add_argument("--matchy", default=None, help="path to matchy binary")
    args = parser.parse_args()

    variant = args.variant
    run_dir = RUNS_DIR / variant
    matchy_bin = Path(args.matchy).resolve() if args.matchy else DEFAULT_MATCHY

    print(f"=== determinism-check: {variant} ===")
    print()

    if not matchy_bin.exists():
        print(f"ERROR: matchy binary not found: {matchy_bin}")
        return 1

    # --- Locate bundles ---
    try:
        old_bundle, new_bundle = _find_bundles(run_dir)
    except FileNotFoundError as exc:
        print(f"ERROR: {exc}")
        print(f"  Run 'make fixture VARIANT={variant}' first to produce capture bundles.")
        return 1

    print(f"old bundle: {old_bundle}")
    print(f"new bundle: {new_bundle}")
    print()

    # --- Two analysis passes in temp dirs ---
    with tempfile.TemporaryDirectory(prefix="ppd-det-") as tmp_root:
        tmp1 = Path(tmp_root) / "run1"
        tmp2 = Path(tmp_root) / "run2"
        tmp1.mkdir()
        tmp2.mkdir()

        for label, out_dir in [("run 1", tmp1), ("run 2", tmp2)]:
            cmd = [
                str(matchy_bin),
                "analyze",
                "--old-bundle", str(old_bundle),
                "--new-bundle", str(new_bundle),
                "--out", str(out_dir),
            ]
            print(f"  {label}: {' '.join(cmd)}")
            result = subprocess.run(cmd, capture_output=True, text=True)
            if result.returncode not in (0, 1):
                print(f"  ERROR: matchy exited {result.returncode}")
                print(result.stderr)
                return 1

        dr1 = tmp1 / "diff-result.json"
        dr2 = tmp2 / "diff-result.json"

        for p in (dr1, dr2):
            if not p.exists():
                print(f"  ERROR: diff-result.json not produced: {p}")
                return 1

        b1 = _canonical_bytes(dr1)
        b2 = _canonical_bytes(dr2)

    print()
    if b1 == b2:
        print("PASS  (analysis is byte-deterministic)")
        return 0
    else:
        # Find first differing byte position
        min_len = min(len(b1), len(b2))
        first_diff = next((i for i in range(min_len) if b1[i] != b2[i]), min_len)
        print(f"FAIL  (outputs differ; first difference at byte offset {first_diff})")
        # Show context around first diff in the string form
        s1 = b1.decode("utf-8", errors="replace")
        s2 = b2.decode("utf-8", errors="replace")
        ctx_start = max(0, first_diff - 40)
        ctx_end = min(len(s1), len(s2), first_diff + 40)
        print(f"  run1 context: ...{s1[ctx_start:ctx_end]}...")
        print(f"  run2 context: ...{s2[ctx_start:ctx_end]}...")
        return 1


if __name__ == "__main__":
    sys.exit(main())
