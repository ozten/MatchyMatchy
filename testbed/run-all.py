#!/usr/bin/env python3
"""
run-all.py start | stop | check  (--check is an alias for check)

Manages golden + variant HTTP servers for the page-pair-diff testbed.
All paths are resolved relative to this script's own location so the
script works from any CWD.
"""

import argparse
import json
import os
import signal
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

import jsonschema

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------
SCRIPT_DIR = Path(__file__).resolve().parent
GOLDEN_DIR = SCRIPT_DIR / "golden"
VARIANTS_DIR = SCRIPT_DIR / "variants"
SCHEMAS_DIR = SCRIPT_DIR / "schemas"
PIDS_FILE = SCRIPT_DIR / ".pids.json"

MANIFEST_SCHEMA_PATH = SCHEMAS_DIR / "manifest.schema.json"
EXPECTED_ISSUES_SCHEMA_PATH = SCHEMAS_DIR / "expected-issues.schema.json"

GOLDEN_PORT = 47000
TCP_POLL_INTERVAL = 0.25  # seconds between connection attempts
TCP_TIMEOUT = 20  # seconds total wait

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _port_open(port: int) -> bool:
    """Return True if a TCP listener is accepting connections on localhost:port."""
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=1):
            return True
    except OSError:
        return False


def _wait_for_ports(port_map: dict[str, int], timeout: float = TCP_TIMEOUT) -> dict[str, bool]:
    """
    Poll until every port in port_map accepts a TCP connection or timeout.
    Returns {name: True/False} for each entry.
    """
    remaining = dict(port_map)
    ready: dict[str, bool] = {}
    deadline = time.monotonic() + timeout

    while remaining and time.monotonic() < deadline:
        for name, port in list(remaining.items()):
            if _port_open(port):
                ready[name] = True
                del remaining[name]
        if remaining:
            time.sleep(TCP_POLL_INTERVAL)

    for name in remaining:
        ready[name] = False
    return ready


def _load_pids() -> dict[str, int]:
    if PIDS_FILE.exists():
        try:
            return json.loads(PIDS_FILE.read_text())
        except (json.JSONDecodeError, OSError):
            return {}
    return {}


def _save_pids(pids: dict[str, int]) -> None:
    PIDS_FILE.write_text(json.dumps(pids, indent=2))


def _discover_servers() -> list[dict]:
    """
    Return list of server descriptors, golden first then variants sorted by name.
    Each descriptor:
      {name, port, directory, serve_py}
    """
    servers = []

    golden_serve = GOLDEN_DIR / "serve.py"
    if golden_serve.exists():
        servers.append({
            "name": "golden",
            "port": GOLDEN_PORT,
            "directory": GOLDEN_DIR,
            "serve_py": golden_serve,
        })

    variant_dirs = sorted(VARIANTS_DIR.iterdir()) if VARIANTS_DIR.exists() else []
    for vdir in variant_dirs:
        if not vdir.is_dir():
            continue
        serve_py = vdir / "serve.py"
        manifest_path = vdir / "manifest.json"
        if not serve_py.exists():
            continue
        if not manifest_path.exists():
            continue
        try:
            manifest = json.loads(manifest_path.read_text())
            port = manifest["port"]
        except (json.JSONDecodeError, KeyError, OSError):
            continue
        servers.append({
            "name": vdir.name,
            "port": port,
            "directory": vdir,
            "serve_py": serve_py,
            "manifest_path": manifest_path,
        })

    return servers


def _spawn_server(server: dict) -> int:
    """Spawn serve.py for a server; return pid."""
    log_path = server["directory"] / "serve.log"
    with open(log_path, "a") as log_fh:
        proc = subprocess.Popen(
            ["python3", "serve.py"],
            cwd=str(server["directory"]),
            stdout=log_fh,
            stderr=log_fh,
            start_new_session=True,
        )
    return proc.pid


def _no_redirect_opener():
    """Build a urllib opener that does NOT follow HTTP redirects."""
    class NoRedirectHandler(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, req, fp, code, msg, headers, newurl):
            return None  # suppress redirect

    return urllib.request.build_opener(NoRedirectHandler())


# ---------------------------------------------------------------------------
# Commands
# ---------------------------------------------------------------------------

def cmd_start() -> int:
    servers = _discover_servers()
    pids = _load_pids()
    newly_started: dict[str, int] = {}

    for srv in servers:
        if _port_open(srv["port"]):
            # Already up — skip
            continue
        pid = _spawn_server(srv)
        newly_started[srv["name"]] = pid
        pids[srv["name"]] = pid

    if newly_started:
        _save_pids(pids)

    # Wait for all ports
    all_ports = {srv["name"]: srv["port"] for srv in servers}
    ready = _wait_for_ports(all_ports)

    # Print table
    print(f"{'NAME':<30} {'PORT':<6} {'STATUS'}")
    print("-" * 55)
    all_ok = True
    for srv in servers:
        name = srv["name"]
        port = srv["port"]
        if ready[name]:
            if name in newly_started:
                status = "started"
            else:
                status = "already up"
        else:
            status = "FAILED"
            all_ok = False
        print(f"{name:<30} {port:<6} {status}")

    return 0 if all_ok else 1


def cmd_stop() -> int:
    pids = _load_pids()
    if not pids:
        print("No PIDs recorded — nothing to stop.")
    else:
        for name, pid in sorted(pids.items()):
            try:
                os.kill(pid, signal.SIGTERM)
                print(f"  SIGTERM -> {name} (pid {pid})")
            except ProcessLookupError:
                print(f"  {name} (pid {pid}) already gone")
        PIDS_FILE.unlink(missing_ok=True)

    # Report any ports still listening
    servers = _discover_servers()
    still_up = [srv for srv in servers if _port_open(srv["port"])]
    if still_up:
        print("Ports still listening after stop:")
        for srv in still_up:
            print(f"  {srv['name']} :{srv['port']}")
    else:
        print("No ports listening.")

    return 0


def cmd_check() -> int:
    servers = _discover_servers()

    # Load schemas once
    manifest_schema = json.loads(MANIFEST_SCHEMA_PATH.read_text())
    expected_issues_schema = json.loads(EXPECTED_ISSUES_SCHEMA_PATH.read_text())
    validator_cls = jsonschema.validators.validator_for(manifest_schema)

    # Start any servers not already up; remember which ones we started
    started_here: set[str] = set()
    servers_to_wait: dict[str, int] = {}

    for srv in servers:
        if not _port_open(srv["port"]):
            pid = _spawn_server(srv)
            started_here.add(srv["name"])
            pids = _load_pids()
            pids[srv["name"]] = pid
            _save_pids(pids)
            servers_to_wait[srv["name"]] = srv["port"]

    if servers_to_wait:
        _wait_for_ports(servers_to_wait)

    opener = _no_redirect_opener()

    # Per-variant checks (golden has no manifest/expected-issues)
    variant_servers = [s for s in servers if s["name"] != "golden"]

    # Collect all ports for uniqueness check
    all_ports = [srv["port"] for srv in variant_servers]
    duplicate_ports: set[int] = {p for p in all_ports if all_ports.count(p) > 1}

    # Table data
    rows = []

    for srv in variant_servers:
        name = srv["name"]
        port = srv["port"]
        manifest_path = srv.get("manifest_path", srv["directory"] / "manifest.json")
        expected_issues_path = srv["directory"] / "expected-issues.json"

        manifest_ok = "PASS"
        expected_ok = "PASS"
        http_ok = "PASS"
        errors: list[str] = []

        # --- Manifest validation ---
        if not manifest_path.exists():
            manifest_ok = "FAIL"
            errors.append("missing manifest.json")
            manifest_data = None
        else:
            try:
                manifest_data = json.loads(manifest_path.read_text())
            except json.JSONDecodeError as exc:
                manifest_ok = "FAIL"
                errors.append(f"manifest JSON parse error: {exc}")
                manifest_data = None

            if manifest_data is not None:
                v = validator_cls(manifest_schema)
                schema_errors = list(v.iter_errors(manifest_data))
                if schema_errors:
                    manifest_ok = "FAIL"
                    for se in schema_errors:
                        errors.append(f"manifest schema: {se.message}")

                # Consistency: name == directory name
                if manifest_data.get("name") != name:
                    manifest_ok = "FAIL"
                    errors.append(
                        f"manifest name '{manifest_data.get('name')}' != dir name '{name}'"
                    )

                # Consistency: port != 3000
                if manifest_data.get("port") == GOLDEN_PORT:
                    manifest_ok = "FAIL"
                    errors.append(f"port {GOLDEN_PORT} is reserved for golden")

                # Consistency: port not duplicated
                if port in duplicate_ports:
                    manifest_ok = "FAIL"
                    errors.append(f"port {port} duplicated across variants")

        # --- Expected-issues validation ---
        if not expected_issues_path.exists():
            expected_ok = "FAIL"
            errors.append("missing expected-issues.json")
        else:
            try:
                expected_data = json.loads(expected_issues_path.read_text())
            except json.JSONDecodeError as exc:
                expected_ok = "FAIL"
                errors.append(f"expected-issues JSON parse error: {exc}")
                expected_data = None

            if expected_data is not None:
                v2 = validator_cls(expected_issues_schema)
                ei_errors = list(v2.iter_errors(expected_data))
                if ei_errors:
                    expected_ok = "FAIL"
                    for se in ei_errors:
                        errors.append(f"expected-issues schema: {se.message}")

        # --- HTTP check ---
        if manifest_data is not None and "urlUnderTest" in manifest_data:
            url = manifest_data["urlUnderTest"]
        else:
            url = f"http://localhost:{port}/"

        # Optional: manifest may declare a non-200 expected status (e.g. v18-status-mismatch)
        expected_status = 200
        if manifest_data is not None and "expectedHttpStatus" in manifest_data:
            expected_status = int(manifest_data["expectedHttpStatus"])

        try:
            req = urllib.request.Request(url)
            try:
                resp = opener.open(req, timeout=10)
                code = resp.status
            except urllib.error.HTTPError as exc:
                code = exc.code
            if code != expected_status:
                http_ok = "FAIL"
                errors.append(f"HTTP {code} (expected {expected_status}) from {url}")
        except urllib.error.URLError as exc:
            http_ok = "FAIL"
            errors.append(f"URL error: {exc.reason}")
        except Exception as exc:
            http_ok = "FAIL"
            errors.append(f"HTTP check exception: {exc}")

        overall = "PASS" if all(s == "PASS" for s in [manifest_ok, expected_ok, http_ok]) else "FAIL"
        rows.append({
            "name": name,
            "port": port,
            "manifest": manifest_ok,
            "expected_issues": expected_ok,
            "http": http_ok,
            "overall": overall,
            "errors": errors,
        })

    # Also check golden HTTP
    golden_http = "PASS"
    golden_errors: list[str] = []
    try:
        req = urllib.request.Request(f"http://localhost:{GOLDEN_PORT}/")
        resp = opener.open(req, timeout=10)
        code = resp.status
        if code != 200:
            golden_http = "FAIL"
            golden_errors.append(f"HTTP {code}")
    except Exception as exc:
        golden_http = "FAIL"
        golden_errors.append(str(exc))

    # Print table
    print()
    header = f"{'VARIANT':<30} {'PORT':<6} {'MANIFEST':<10} {'EXP-ISSUES':<12} {'HTTP':<6} {'OVERALL'}"
    print(header)
    print("-" * len(header))

    # Golden row
    print(
        f"{'golden':<30} {GOLDEN_PORT:<6} {'n/a':<10} {'n/a':<12} {golden_http:<6} {golden_http}"
    )
    if golden_errors:
        for e in golden_errors:
            print(f"  ERROR: {e}")

    total = len(rows) + 1  # +1 for golden
    passed = (1 if golden_http == "PASS" else 0)

    for row in rows:
        print(
            f"{row['name']:<30} {row['port']:<6} {row['manifest']:<10} "
            f"{row['expected_issues']:<12} {row['http']:<6} {row['overall']}"
        )
        for e in row["errors"]:
            print(f"  ERROR: {e}")
        if row["overall"] == "PASS":
            passed += 1

    print()
    overall_result = "PASS" if passed == total else "FAIL"
    print(f"TESTBED CHECK: {overall_result} ({passed}/{total} ok)")

    # Stop only servers this run started
    if started_here:
        pids = _load_pids()
        remaining_pids: dict[str, int] = {}
        for name, pid in pids.items():
            if name in started_here:
                try:
                    os.kill(pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
            else:
                remaining_pids[name] = pid
        if remaining_pids:
            _save_pids(remaining_pids)
        elif PIDS_FILE.exists():
            PIDS_FILE.unlink(missing_ok=True)

    return 0 if overall_result == "PASS" else 1


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

def main() -> int:
    parser = argparse.ArgumentParser(
        prog="run-all.py",
        description="Manage page-pair-diff testbed servers.",
    )
    parser.add_argument(
        "command",
        nargs="?",
        choices=["start", "stop", "check"],
        help="Command to run",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Alias for the 'check' command",
    )
    args = parser.parse_args()

    command = args.command
    if args.check:
        command = "check"

    if command is None:
        parser.print_help()
        return 1

    if command == "start":
        return cmd_start()
    elif command == "stop":
        return cmd_stop()
    elif command == "check":
        return cmd_check()
    else:
        parser.print_help()
        return 1


if __name__ == "__main__":
    sys.exit(main())
