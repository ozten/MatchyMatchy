#!/usr/bin/env python3
"""
migrate-golden-ids.py -- id-migration pre-pass for the U2 issue-id derivation fix.

Context (port-parity plan, U14): `packages/analyze/src/issue.rs`'s `compute_issue_id`
and `resolve_id_collisions` were rewritten (U2) so issue ids survive re-captures:
`ordinalInLandmark` was dropped from the hash unconditionally, `nearestHeading` became
conditional (identity-grade only when text/href/alt/ariaLabel are all absent), and
collision suffixing switched from a bbox-pixel sort to document order
(`seqIndexOld` ascending, `None` last, then `seqIndexNew`, then array order).

Every issue id in every committed golden changes at least once as a mechanical
consequence -- not because the tool's *behavior* changed. If we simply re-recorded
goldens with the new build, that mechanical churn would swamp the real triage: any
genuine regression hiding inside 2,500 renamed ids would be undetectable by eye.

This script recomputes each existing golden's issue ids under the NEW derivation,
using only fields already present in the golden JSON (type, viewport, anchors minus
ordinalInLandmark, the conditional nearestHeading rule, and the styleProperty hash
slot recovered from `remediation.property` for style-category issues), and rewrites
every reference to those ids in place: `issues[].id`, `clusters[].issueIds`,
`agentSummary.topFixes`, `suppressed.ids`, `regions[].memberIssueIds`,
`outOfScope.ids` -- and anywhere else an old id string appears, via a generic
recursive string-replace keyed on the mapping (cluster/region ids are NOT touched:
they hash `type + kind + shared_key`, independent of member issue ids, per
`clustering.rs::sha12`, so they are stable across this migration by construction).

After this pre-pass runs, `testbed/goldens/*.diffresult.json` hold "triage baseline"
documents: byte-identical to the pre-U2 goldens in every field EXCEPT issue ids, which
now reflect the NEW derivation. Diffing a freshly re-recorded golden (built from the
current code) against this triage baseline should show *zero* id-only drift; any
remaining diff is genuine schema/severity/score/detector drift for U14 to triage. If
id-only drift is nonzero, this script's replication of `compute_issue_id` /
`resolve_id_collisions` has a bug relative to `issue.rs` -- fix the script, not the
golden.

MUST mirror packages/analyze/src/issue.rs::compute_issue_id and ::resolve_id_collisions
byte-for-byte. Read that file before touching this one.

Usage:
    python3 testbed/migrate-golden-ids.py [--dry-run] [FILES...]

    With no FILES, operates on testbed/goldens/*.diffresult.json.
    --dry-run prints the per-file summary without writing anything.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import sys
from pathlib import Path
from urllib.parse import urlsplit

SCRIPT_DIR = Path(__file__).resolve().parent
GOLDENS_DIR = SCRIPT_DIR / "goldens"

UNIT_SEP = "\x1f"


# ---------------------------------------------------------------------------
# id_stable_url -- mirrors issue.rs::id_stable_url
# ---------------------------------------------------------------------------
def id_stable_url(href: str) -> str:
    """Normalize an href to scheme://host[:port]/path, dropping query+fragment.

    Mirrors issue.rs::id_stable_url: on successful absolute-URL parse, returns
    scheme+host[:port]+path. On parse failure (relative/malformed hrefs), truncates
    at the first '?' or '#', whichever comes first; unchanged if neither is present.
    """
    parts = urlsplit(href)
    # Rust's url::Url::parse requires an absolute URI (scheme present). A bare
    # scheme with no netloc (mailto:, tel:) still parses in the `url` crate, but
    # none of the current fixture data uses such schemes; treat scheme-without-
    # netloc as a parse success too, mirroring url::Url's permissiveness, with an
    # empty host.
    if parts.scheme:
        host = parts.hostname or ""
        path = parts.path or ""
        if parts.port:
            return f"{parts.scheme}://{host}:{parts.port}{path}"
        return f"{parts.scheme}://{host}{path}"
    # Relative or unparseable: strip from first '?' or '#'.
    q = href.find("?")
    f = href.find("#")
    candidates = [c for c in (q, f) if c != -1]
    if not candidates:
        return href
    cut = min(candidates)
    return href[:cut]


# ---------------------------------------------------------------------------
# compute_issue_id -- mirrors issue.rs::compute_issue_id
# ---------------------------------------------------------------------------
def compute_issue_id(issue_type: str, viewport: str, anchors: dict, style_property: str | None) -> str:
    """Replicates issue.rs::compute_issue_id.

    Canonical = fields joined by U+001F, in exact order:
      type, viewport, text, role, href (id_stable_url-normalized), alt, ariaLabel,
      landmark, [nearestHeading -- conditional], styleProperty.

    ordinalInLandmark is unconditionally excluded (not present anywhere below).
    nearestHeading is identity-grade ONLY when text/href/alt/ariaLabel are ALL
    absent/empty; otherwise its slot is empty even if the value is present.
    """
    text = anchors.get("text") or ""
    role = anchors.get("role") or ""
    href = anchors.get("href")
    href_stable = id_stable_url(href) if href else ""
    alt = anchors.get("alt") or ""
    aria_label = anchors.get("ariaLabel") or ""
    landmark = anchors.get("landmark") or ""
    nearest_heading = anchors.get("nearestHeading") or ""

    has_strong_or_medium_anchor = bool(text) or bool(href_stable) or bool(alt) or bool(aria_label)
    nearest_heading_slot = "" if has_strong_or_medium_anchor else nearest_heading

    fields = [
        issue_type,
        viewport,
        text,
        role,
        href_stable,
        alt,
        aria_label,
        landmark,
        nearest_heading_slot,
        style_property or "",
    ]
    canonical = UNIT_SEP.join(fields)
    hex_str = hashlib.sha256(canonical.encode("utf-8")).hexdigest()
    return f"issue_{hex_str[:12]}"


# ---------------------------------------------------------------------------
# resolve_id_collisions -- mirrors issue.rs::resolve_id_collisions
# ---------------------------------------------------------------------------
def _seq_sort_key(v):
    # Some(n) -> (0, n); None -> (1, 0) -- Some always precedes None.
    if v is None:
        return (1, 0)
    return (0, v)


def resolve_id_collisions(records: list[dict]) -> None:
    """Assigns final (possibly suffixed) ids to `records` in place.

    Each record must have: 'base_id', 'seq_index_old', 'seq_index_new', and
    'orig_index' (position in the input array -- the best available stand-in for
    Rust's pre-output-sort insertion order; see module docstring for the residual
    approximation this implies when seqIndexOld AND seqIndexNew are BOTH tied
    within a colliding group).

    Sets 'final_id' on each record.
    """
    groups: dict[str, list[dict]] = {}
    for rec in records:
        groups.setdefault(rec["base_id"], []).append(rec)

    for base_id, group in groups.items():
        if len(group) <= 1:
            group[0]["final_id"] = base_id
            continue

        group.sort(
            key=lambda r: (
                _seq_sort_key(r["seq_index_old"]),
                _seq_sort_key(r["seq_index_new"]),
                r["orig_index"],
            )
        )
        for suffix_idx, rec in enumerate(group):
            if suffix_idx == 0:
                rec["final_id"] = base_id
            else:
                rec["final_id"] = f"{base_id}-{suffix_idx + 1}"


# ---------------------------------------------------------------------------
# Per-golden migration
# ---------------------------------------------------------------------------
def style_property_of(issue: dict) -> str | None:
    if issue.get("category") != "style":
        return None
    remediation = issue.get("remediation")
    if not remediation:
        return None
    prop = remediation.get("property")
    if not prop:
        return None
    return prop


def build_id_mapping(diff_result: dict) -> dict[str, str]:
    """Returns {old_id: new_id} for every issue in diff_result['issues']."""
    issues = diff_result.get("issues", [])
    records = []
    for idx, issue in enumerate(issues):
        anchors = issue.get("locator", {}).get("anchors", {})
        style_property = style_property_of(issue)
        base_id = compute_issue_id(issue["type"], issue["viewport"], anchors, style_property)
        locator = issue.get("locator", {})
        records.append(
            {
                "old_id": issue["id"],
                "base_id": base_id,
                "seq_index_old": locator.get("seqIndexOld"),
                "seq_index_new": locator.get("seqIndexNew"),
                "orig_index": idx,
            }
        )

    resolve_id_collisions(records)

    mapping: dict[str, str] = {}
    for rec in records:
        mapping[rec["old_id"]] = rec["final_id"]
    return mapping


def apply_mapping(node, mapping: dict[str, str]):
    """Recursively rewrites every string in `node` that is a key in `mapping`."""
    if isinstance(node, dict):
        return {k: apply_mapping(v, mapping) for k, v in node.items()}
    if isinstance(node, list):
        return [apply_mapping(v, mapping) for v in node]
    if isinstance(node, str):
        return mapping.get(node, node)
    return node


def migrate_file(path: Path, dry_run: bool) -> tuple[int, int, int]:
    """Returns (total_issues, ids_rewritten, collision_groups)."""
    original = json.loads(path.read_text())
    mapping = build_id_mapping(original)

    ids_rewritten = sum(1 for old, new in mapping.items() if old != new)
    total_issues = len(original.get("issues", []))

    # Count colliding groups (base ids shared by >1 issue) for the summary.
    base_id_counts: dict[str, int] = {}
    for issue in original.get("issues", []):
        anchors = issue.get("locator", {}).get("anchors", {})
        sp = style_property_of(issue)
        base = compute_issue_id(issue["type"], issue["viewport"], anchors, sp)
        base_id_counts[base] = base_id_counts.get(base, 0) + 1
    collision_groups = sum(1 for c in base_id_counts.values() if c > 1)

    migrated = apply_mapping(copy.deepcopy(original), mapping)

    if not dry_run:
        # ensure_ascii=False: goldens store raw UTF-8 (em dashes, curly quotes, …);
        # escaping them to \uXXXX would be JSON-equivalent but a needless byte-diff
        # noise source against the pre-migration file, obscuring the id-only intent
        # of this pass.
        path.write_text(json.dumps(migrated, indent=2, ensure_ascii=False) + "\n")

    return total_issues, ids_rewritten, collision_groups


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("files", nargs="*", help="golden files to migrate (default: all testbed/goldens/*.diffresult.json)")
    parser.add_argument("--dry-run", action="store_true", help="print summary without writing")
    args = parser.parse_args()

    if args.files:
        targets = [Path(f) for f in args.files]
    else:
        targets = sorted(GOLDENS_DIR.glob("*.diffresult.json"))

    if not targets:
        print("No golden files found.")
        return 1

    print(f"{'GOLDEN':<45}  {'ISSUES':>7}  {'REWRITTEN':>9}  {'COLLISION GROUPS':>16}")
    print("-" * 85)
    total_rewritten = 0
    for path in targets:
        total_issues, ids_rewritten, collision_groups = migrate_file(path, args.dry_run)
        total_rewritten += ids_rewritten
        print(f"{path.name:<45}  {total_issues:>7}  {ids_rewritten:>9}  {collision_groups:>16}")

    print("-" * 85)
    mode = "DRY-RUN (nothing written)" if args.dry_run else "WRITTEN in place"
    print(f"{mode}: {total_rewritten} id(s) rewritten across {len(targets)} file(s).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
