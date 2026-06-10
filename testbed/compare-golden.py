#!/usr/bin/env python3
"""
compare-golden.py  --  deep structural comparison of two DiffResult JSON files

Usage:
    python3 testbed/compare-golden.py <golden.diffresult.json> <fresh-diff-result.json>

Rules (per M1.md §6.2):
  - Excluded keys (at any depth): "runId", "capturedAt"
  - Floats: equal within abs tolerance 1e-4
  - Integers: exact
  - Strings: exact
  - Arrays: order-sensitive, length-exact
  - Objects: all keys must match (after exclusion)
  - On mismatch: print JSON-pointer-style path for each difference (cap at 20)

Exit 0 = identical within tolerances; exit 1 = differences found.

All paths are resolved relative to this script's own location so the script works
from any CWD.
"""

import json
import sys
from pathlib import Path

FLOAT_TOLERANCE = 1e-4
MAX_DIFFS = 20
EXCLUDED_KEYS = {"runId", "capturedAt"}


def _compare(a, b, path: str, diffs: list) -> None:
    """Recursively compare a and b, appending (path, a_val, b_val) to diffs."""
    if len(diffs) >= MAX_DIFFS:
        return

    # --- dict ---
    if isinstance(a, dict) and isinstance(b, dict):
        # Filter out excluded keys from both sides
        a_keys = {k for k in a if k not in EXCLUDED_KEYS}
        b_keys = {k for k in b if k not in EXCLUDED_KEYS}

        for k in sorted(a_keys - b_keys):
            diffs.append((f"{path}/{k}", a[k], "<missing>"))
        for k in sorted(b_keys - a_keys):
            diffs.append((f"{path}/{k}", "<missing>", b[k]))
        for k in sorted(a_keys & b_keys):
            _compare(a[k], b[k], f"{path}/{k}", diffs)
        return

    # --- list ---
    if isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            diffs.append((path, f"<list len={len(a)}>", f"<list len={len(b)}>"))
            return
        for i, (av, bv) in enumerate(zip(a, b)):
            _compare(av, bv, f"{path}/{i}", diffs)
        return

    # --- float ---
    if isinstance(a, float) or isinstance(b, float):
        # Treat int/float as comparable
        try:
            af = float(a)
            bf = float(b)
            if abs(af - bf) <= FLOAT_TOLERANCE:
                return
            diffs.append((path, a, b))
        except (TypeError, ValueError):
            diffs.append((path, a, b))
        return

    # --- int, str, bool, None ---
    if a != b:
        diffs.append((path, a, b))


def main() -> int:
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <golden.diffresult.json> <fresh-diff-result.json>")
        return 2

    golden_path = Path(sys.argv[1])
    fresh_path = Path(sys.argv[2])

    for p in (golden_path, fresh_path):
        if not p.exists():
            print(f"ERROR: file not found: {p}")
            return 1

    try:
        golden = json.loads(golden_path.read_text())
    except json.JSONDecodeError as exc:
        print(f"ERROR: JSON parse error in {golden_path}: {exc}")
        return 1

    try:
        fresh = json.loads(fresh_path.read_text())
    except json.JSONDecodeError as exc:
        print(f"ERROR: JSON parse error in {fresh_path}: {exc}")
        return 1

    diffs: list[tuple[str, object, object]] = []
    _compare(golden, fresh, "", diffs)

    if not diffs:
        print("PASS  (no differences within tolerance)")
        return 0

    print(f"FAIL  ({len(diffs)} difference(s) found, showing up to {MAX_DIFFS}):")
    print()
    print(f"  {'PATH':<50}  {'GOLDEN':<30}  FRESH")
    print("  " + "-" * 100)
    for path, a_val, b_val in diffs[:MAX_DIFFS]:
        a_str = repr(a_val)[:29]
        b_str = repr(b_val)[:40]
        print(f"  {path:<50}  {a_str:<30}  {b_str}")

    if len(diffs) > MAX_DIFFS:
        print(f"  ... and {len(diffs) - MAX_DIFFS} more (capped)")

    return 1


if __name__ == "__main__":
    sys.exit(main())
