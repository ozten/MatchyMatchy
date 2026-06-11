#!/usr/bin/env python3
"""
migrate-ports.py — relocate the testbed's fixed port range.

Old range: golden 3000, variants 3001-3021.
New range: golden 47000, variants 47001-47021  (uniform offset +44000).

Why an offset (not a re-record): a fresh capture on this machine already equals the
recorded goldens within tolerance, so the captures are valid — only the origin's port
changes. Relocating is therefore a pure, reversible string substitution.

SAFETY: goldens/expected-issues contain bare 4-digit sequences in the 30xx range that are
NOT ports (float mantissas like 0.0041593011, content-addressed ids like issue_93006).
So in those files we ONLY rewrite the unambiguous `localhost:<port>` form. Config/harness
files get targeted rewrites of their specific port assignments. Site files are never touched.

Run from repo root:  python3 scripts/migrate-ports.py [--dry-run]
"""
import argparse
import re
import sys
from pathlib import Path

OLD_BASE = 3000
NEW_BASE = 47000
OFFSET = NEW_BASE - OLD_BASE          # +44000
OLD_PORTS = range(OLD_BASE, OLD_BASE + 22)   # 3000..3021 inclusive

REPO = Path(__file__).resolve().parent.parent
TESTBED = REPO / "testbed"

# Matches a testbed port (3000-3021) only when it is a standalone integer
# (not preceded/followed by another digit). Used for config files where the
# surrounding context is known-safe (PORT =, "port":, schema bounds).
PORT_INT = re.compile(r"(?<!\d)(30(?:0[0-9]|1[0-9]|2[0-1]))(?!\d)")
# Matches localhost:<testbed-port>. The unambiguous form used inside goldens,
# expected-issues, harness URLs, and manifest urlUnderTest.
LOCALHOST_PORT = re.compile(r"(localhost:)(30(?:0[0-9]|1[0-9]|2[0-1]))(?!\d)")


def bump_int(m: re.Match) -> str:
    return str(int(m.group(1)) + OFFSET)


def bump_localhost(m: re.Match) -> str:
    return f"{m.group(1)}{int(m.group(2)) + OFFSET}"


def edit(path: Path, fn, dry: bool) -> int:
    """Apply fn(text)->(text,count); write unless dry. Return change count."""
    if not path.exists():
        print(f"  SKIP (missing): {path.relative_to(REPO)}")
        return 0
    before = path.read_text()
    after, n = fn(before)
    if n and not dry:
        path.write_text(after)
    if n:
        print(f"  {n:3d}  {path.relative_to(REPO)}")
    return n


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()
    dry = args.dry_run
    total = 0

    print(f"Port migration {OLD_BASE}-{OLD_BASE+21} -> {NEW_BASE}-{NEW_BASE+21} "
          f"(offset {OFFSET:+d}){'  [DRY RUN]' if dry else ''}\n")

    # 1. serve.py (golden + 21 variants): `PORT = 30NN` and docstring `port 30NN`.
    #    Custom handlers use relative Location paths, so PORT_INT on the whole file is
    #    safe here (no other 30xx integers exist in these scripts).
    print("serve.py (PORT + docstring):")
    serve_files = [TESTBED / "golden" / "serve.py"] + sorted(
        TESTBED.glob("variants/*/serve.py"))
    for f in serve_files:
        total += edit(f, lambda t: PORT_INT.subn(bump_int, t), dry)

    # 2. manifest.json: `"port": 30NN` (integer) + any `localhost:30NN` (urlUnderTest).
    #    Scope to those two forms; never touch free-text descriptions.
    print("manifest.json (port + urlUnderTest):")
    for f in sorted(TESTBED.glob("variants/*/manifest.json")):
        def fix(t):
            t, a = re.subn(r'("port":\s*)(30(?:0[0-9]|1[0-9]|2[0-1]))(?!\d)',
                           lambda m: m.group(1) + str(int(m.group(2)) + OFFSET), t)
            t, b = LOCALHOST_PORT.subn(bump_localhost, t)
            return t, a + b
        total += edit(f, fix, dry)

    # 3. harness: run-all.py GOLDEN_PORT, check-fixture/check-m8 GOLDEN_URL.
    print("harness:")
    total += edit(TESTBED / "run-all.py",
                  lambda t: re.subn(r"(GOLDEN_PORT\s*=\s*)3000\b",
                                    lambda m: m.group(1) + str(NEW_BASE), t), dry)
    for name in ("check-fixture.py", "check-m8.py"):
        total += edit(TESTBED / name, lambda t: LOCALHOST_PORT.subn(bump_localhost, t), dry)

    # 4. manifest.schema.json: shift the allowed [min, max] band.
    print("schema:")
    def fix_schema(t):
        t, a = re.subn(r'("minimum":\s*)3001\b', lambda m: m.group(1) + "47001", t)
        t, b = re.subn(r'("maximum":\s*)3099\b', lambda m: m.group(1) + "47099", t)
        # urlUnderTest pattern hardcodes the port width; new ports are 5 digits.
        t, c = re.subn(r'(localhost:\[0-9\]\{)4(\}/)', lambda m: m.group(1) + "5" + m.group(2), t)
        return t, a + b + c
    total += edit(TESTBED / "schemas" / "manifest.schema.json", fix_schema, dry)

    # 5. goldens + expected-issues: localhost:<port> ONLY (avoid float/id false positives).
    print("goldens + expected-issues (localhost:port only):")
    targets = sorted(TESTBED.glob("goldens/*.diffresult.json")) + \
        sorted(TESTBED.glob("variants/*/expected-issues.json"))
    for f in targets:
        total += edit(f, lambda t: LOCALHOST_PORT.subn(bump_localhost, t), dry)

    print(f"\nTotal substitutions: {total}{'  (dry run — nothing written)' if dry else ''}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
