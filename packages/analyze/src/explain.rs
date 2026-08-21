//! `matchy explain` — hermetic computed-style / bbox triage probe.
//!
//! Locates a node by anchor string, node id, or CSS selector across two frozen
//! `CaptureBundle`s and prints a per-side computed-style + bbox table, highlighting
//! differences.  No browser, no network, no taxonomy or scoring — surfaces only data
//! already in the bundles.

use std::collections::BTreeMap;

use crate::contract::{
    CaptureBundle, HitTestEntry, HitTestOutcome, HitTestPoint, HitTestStatus, PseudoElementEntry,
    SemanticNode,
};
use crate::hit_test_diff::{format_miss_winners, tally_points};
use crate::pseudo_diff::{pseudo_style_map, PseudoSlot};

// ---------------------------------------------------------------------------
// Public API types
// ---------------------------------------------------------------------------

/// The three ways to point at a node.
#[derive(Debug, Clone)]
pub enum Locator {
    /// `key=value` where key ∈ {text, role, href, nearestHeading}.
    /// Substring match on the corresponding `NodeAnchors` / `SemanticNode` field.
    Anchor { key: String, value: String },
    /// Exact match on `SemanticNode.id`.
    NodeId(String),
    /// Exact match on `SemanticNode.css_selector` first; falls back to substring.
    Selector(String),
}

impl Locator {
    /// Parse `--anchor "text=Get started"` into `Locator::Anchor`.
    /// Returns `Err` if the string has no `=`.
    pub fn parse_anchor(s: &str) -> Result<Self, String> {
        let (key, value) = s
            .split_once('=')
            .ok_or_else(|| format!("invalid --anchor syntax '{}': expected key=value", s))?;
        let key = key.to_string();
        if !matches!(key.as_str(), "text" | "role" | "href" | "nearestHeading") {
            return Err(format!(
                "unknown anchor key '{}': expected text, role, href, or nearestHeading",
                key
            ));
        }
        Ok(Locator::Anchor {
            key,
            value: value.to_string(),
        })
    }
}

/// Resolution status for one side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionStatus {
    Resolved,
    NotFound,
}

/// Per-side resolution result.
#[derive(Debug, Clone)]
pub struct SideResult {
    pub status: ResolutionStatus,
    pub node_id: Option<String>,
    /// Computed-style props for the resolved node (empty if not found).
    pub computed_styles: BTreeMap<String, String>,
    /// Bbox as four synthetic props: bbox.x, bbox.y, bbox.w, bbox.h
    pub bbox: Option<[i32; 4]>,
    /// Port-parity U7: the resolved node's raw hit-test entry, if the bundle
    /// carries the `hitTests` channel for this node. `None` when the node
    /// wasn't resolved or the channel is absent for it.
    pub hit_test: Option<HitTestEntry>,
    /// Port-parity U10: `"::before"`/`"::after"` labels this resolved node
    /// owns a painted pseudo-element for (tier "node" only — the only tier
    /// reachable via a plain `--anchor`/`--node` node locator), sorted.
    /// Empty for the ordinary case (no owned pseudos) and always empty on the
    /// pseudo-locator path (`explain_pseudo`) itself.
    pub owned_pseudo_slots: Vec<String>,
}

/// One side's hit-test view for the `explain` output (port-parity U7).
#[derive(Debug, Clone)]
pub struct HitTestSideExplain {
    /// Adjusted (or, single-sided, raw) hit fraction, round4-formatted (e.g. "1.0000").
    pub fraction: String,
    /// "hits/denominator" over the same denominator used for `fraction`.
    pub raw_hits: String,
    /// Top-3 miss winners for this side, "sel (xN); sel2 (xM)" (empty when none).
    pub miss_winners: String,
}

/// The hit-test section of an `explain` report: present iff at least one side
/// has sampled hit-test data for the located node. Each side is `None` when
/// that side has no comparable data — rendered as `<absent>`.
#[derive(Debug, Clone)]
pub struct HitTestExplain {
    pub old: Option<HitTestSideExplain>,
    pub new: Option<HitTestSideExplain>,
}

/// One property row in the explain output.
#[derive(Debug, Clone)]
pub struct PropRow {
    pub property: String,
    pub old_value: String,
    pub new_value: String,
    pub changed: bool,
}

/// The structured result returned by [`explain`].
#[derive(Debug, Clone)]
pub struct ExplainReport {
    pub old: SideResult,
    pub new: SideResult,
    /// Property rows in alphabetical order.
    pub rows: Vec<PropRow>,
    /// Human-readable asymmetry description when only one side resolved.
    pub asymmetry_message: Option<String>,
    /// Port-parity U7: hit-test section, present iff either side has sampled
    /// hit-test data for the located node.
    pub hit_test: Option<HitTestExplain>,
}

// ---------------------------------------------------------------------------
// Core pure function
// ---------------------------------------------------------------------------

/// Locate a node by `locator` in each bundle independently, gather their
/// computed-style + bbox values, and produce a property comparison table.
///
/// `props`: when `Some`, show exactly those properties (even if absent/unchanged).
/// When `None`, show only properties that differ between the two sides (diff-only default).
///
/// Determinism: all maps are `BTreeMap`; property rows sorted alphabetically; node
/// resolution tie-breaks on (seq_index, id) to mirror `region_link`'s total order.
pub fn explain(
    old_bundle: &CaptureBundle,
    new_bundle: &CaptureBundle,
    locator: &Locator,
    props: Option<&[String]>,
) -> ExplainReport {
    let old_side = resolve_side(old_bundle, locator);
    let new_side = resolve_side(new_bundle, locator);

    // Build the property rows.
    let rows = build_rows(&old_side, &new_side, props);

    // Build asymmetry message.
    let asymmetry_message = match (&old_side.status, &new_side.status) {
        (ResolutionStatus::Resolved, ResolutionStatus::NotFound) => Some(format!(
            "node present in OLD only (id: {}) — element may have been removed in new",
            old_side.node_id.as_deref().unwrap_or("?")
        )),
        (ResolutionStatus::NotFound, ResolutionStatus::Resolved) => Some(format!(
            "node present in NEW only (id: {}) — element may have been added in new",
            new_side.node_id.as_deref().unwrap_or("?")
        )),
        _ => None,
    };

    let hit_test = build_hit_test_section(old_side.hit_test.as_ref(), new_side.hit_test.as_ref());

    ExplainReport {
        old: old_side,
        new: new_side,
        rows,
        asymmetry_message,
        hit_test,
    }
}

// ---------------------------------------------------------------------------
// Pseudo-element locator (port-parity U10)
// ---------------------------------------------------------------------------

/// Detect and strip a trailing `::before`/`::after` suffix from a
/// `--selector` locator string. Returns `(owner_part, slot)` — `owner_part`
/// is the selector with the suffix removed, passed straight to
/// `resolve_pseudo_owner`. `None` when the string carries neither suffix
/// (the ordinary node-selector path applies).
pub fn parse_pseudo_selector(s: &str) -> Option<(String, PseudoSlot)> {
    if let Some(owner) = s.strip_suffix("::before") {
        return Some((owner.to_string(), PseudoSlot::Before));
    }
    s.strip_suffix("::after")
        .map(|owner| (owner.to_string(), PseudoSlot::After))
}

/// Resolve a pseudo owner within one bundle by the (suffix-stripped) selector
/// string, per the design brief: try an exact `pseudoElements` map-key match
/// first (covers tier-"ancestor" `anc_N` keys and a node id typed directly),
/// then an exact `ownerSelector` match (covers tier-"ancestor"/tier-"selector"
/// owners, whose map key carries a landmark prefix the bare CSS selector does
/// not), then fall back to the existing node-selector locator (covers
/// tier-"node" owners typed as their own CSS selector) followed by a
/// pseudo-entry lookup keyed on the resolved node id.
fn resolve_pseudo_owner<'a>(
    bundle: &'a CaptureBundle,
    owner_part: &str,
) -> Option<(&'a str, &'a PseudoElementEntry)> {
    let pseudo = bundle.pseudo_elements.as_ref()?;

    // (1) exact ownerKey (map key) match.
    if let Some((k, v)) = pseudo.get_key_value(owner_part) {
        return Some((k.as_str(), v));
    }

    // (2) exact ownerSelector match — BTreeMap iteration order (sorted keys)
    // for determinism on the (hypothetical) case of more than one owner
    // sharing a selector string.
    for (k, v) in pseudo.iter() {
        if v.owner_selector.as_deref() == Some(owner_part) {
            return Some((k.as_str(), v));
        }
    }

    // (3) fall back to the existing node-selector locator, then this node's
    // own pseudo entry (tier "node").
    let node = find_node(
        &bundle.page.nodes,
        &Locator::Selector(owner_part.to_string()),
    )?;
    pseudo
        .get_key_value(node.id.as_str())
        .map(|(k, v)| (k.as_str(), v))
}

fn pseudo_to_side_result(
    owner: Option<(&str, &PseudoElementEntry)>,
    slot: PseudoSlot,
) -> SideResult {
    let style = owner.and_then(|(_, e)| slot.style(e));
    match style {
        None => SideResult {
            status: ResolutionStatus::NotFound,
            node_id: None,
            computed_styles: BTreeMap::new(),
            bbox: None,
            hit_test: None,
            owned_pseudo_slots: Vec::new(),
        },
        Some(s) => SideResult {
            status: ResolutionStatus::Resolved,
            node_id: owner.map(|(k, _)| k.to_string()),
            computed_styles: pseudo_style_map(s),
            bbox: s.bbox.map(|b| {
                [
                    b[0].round() as i32,
                    b[1].round() as i32,
                    b[2].round() as i32,
                    b[3].round() as i32,
                ]
            }),
            hit_test: None,
            owned_pseudo_slots: Vec::new(),
        },
    }
}

/// `explain --selector "...::before"` / `"...::after"` (port-parity U10):
/// locate the pseudo's OWNER independently on each side
/// (`resolve_pseudo_owner`), then diff the requested slot's captured style
/// map with the same diff-only default + `--props` filtering the node path
/// uses (`build_rows` / `diff_prop_rows` — shared, not a second
/// implementation). One-side-present is not an error (asymmetry reported;
/// the CLI layer still exits 0, same convention as the node path).
pub fn explain_pseudo(
    old_bundle: &CaptureBundle,
    new_bundle: &CaptureBundle,
    owner_part: &str,
    slot: PseudoSlot,
    props: Option<&[String]>,
) -> ExplainReport {
    let old_owner = resolve_pseudo_owner(old_bundle, owner_part);
    let new_owner = resolve_pseudo_owner(new_bundle, owner_part);

    let old_side = pseudo_to_side_result(old_owner, slot);
    let new_side = pseudo_to_side_result(new_owner, slot);

    let rows = build_rows(&old_side, &new_side, props);

    let asymmetry_message = match (&old_side.status, &new_side.status) {
        (ResolutionStatus::Resolved, ResolutionStatus::NotFound) => Some(format!(
            "pseudo {} present in OLD only (owner: {}) — element/rule may have been removed in new",
            slot.label(),
            old_side.node_id.as_deref().unwrap_or("?")
        )),
        (ResolutionStatus::NotFound, ResolutionStatus::Resolved) => Some(format!(
            "pseudo {} present in NEW only (owner: {}) — element/rule may have been added in new",
            slot.label(),
            new_side.node_id.as_deref().unwrap_or("?")
        )),
        _ => None,
    };

    ExplainReport {
        old: old_side,
        new: new_side,
        rows,
        asymmetry_message,
        hit_test: None,
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Resolve the locator against one bundle independently.
fn resolve_side(bundle: &CaptureBundle, locator: &Locator) -> SideResult {
    let node = find_node(&bundle.page.nodes, locator);

    match node {
        None => SideResult {
            status: ResolutionStatus::NotFound,
            node_id: None,
            computed_styles: BTreeMap::new(),
            bbox: None,
            hit_test: None,
            owned_pseudo_slots: Vec::new(),
        },
        Some(n) => {
            let computed_styles = bundle
                .computed_styles
                .get(&n.id)
                .cloned()
                .unwrap_or_default();
            let bbox = Some(n.bbox);
            let hit_test = bundle
                .hit_tests
                .as_ref()
                .and_then(|m| m.get(&n.id))
                .cloned();
            let owned_pseudo_slots = owned_pseudo_slots_for_node(bundle, &n.id);
            SideResult {
                status: ResolutionStatus::Resolved,
                node_id: Some(n.id.clone()),
                computed_styles,
                bbox,
                hit_test,
                owned_pseudo_slots,
            }
        }
    }
}

/// Port-parity U10: `"::before"`/`"::after"` labels (sorted) a tier-"node"
/// pseudo owner keyed on `node_id` paints, if any. Only tier "node" is
/// reachable via a plain `--anchor`/`--node` node locator — tier
/// "ancestor"/"selector" owners are not `SemanticNode`s and have no bearing
/// here.
fn owned_pseudo_slots_for_node(bundle: &CaptureBundle, node_id: &str) -> Vec<String> {
    bundle
        .pseudo_elements
        .as_ref()
        .and_then(|m| m.get(node_id))
        .map(|entry| {
            let mut v = Vec::new();
            if entry.before.is_some() {
                v.push(PseudoSlot::Before.label().to_string());
            }
            if entry.after.is_some() {
                v.push(PseudoSlot::After.label().to_string());
            }
            v
        })
        .unwrap_or_default()
}

/// A side's hit-test entry, restricted to the case `explain` can render:
/// `status == Sampled` with a non-empty points array.
fn sampled_points(entry: Option<&HitTestEntry>) -> Option<&[HitTestPoint]> {
    entry
        .filter(|e| e.status == HitTestStatus::Sampled)
        .and_then(|e| e.points.as_deref())
        .filter(|p| !p.is_empty())
}

/// Independent (no partner side) per-point tally: excludes `clipped`/`offViewport`
/// but cannot apply the joint both-side-miss drop (there's no partner to compare
/// against), so the "adjusted" fraction here is simply the raw fraction over the
/// excluded-aware denominator.
fn raw_side_explain(points: &[HitTestPoint]) -> HitTestSideExplain {
    let mut hits = 0u32;
    let mut denom = 0u32;
    let mut winners: BTreeMap<String, u32> = BTreeMap::new();
    for p in points {
        match p.o {
            HitTestOutcome::Hit => {
                hits += 1;
                denom += 1;
            }
            HitTestOutcome::Miss => {
                denom += 1;
                if let Some(w) = &p.winner {
                    *winners.entry(w.clone()).or_insert(0) += 1;
                }
            }
            HitTestOutcome::Clipped | HitTestOutcome::OffViewport => {}
        }
    }
    let fraction = if denom == 0 {
        0.0
    } else {
        hits as f64 / denom as f64
    };
    HitTestSideExplain {
        fraction: format!("{:.4}", round4(fraction)),
        raw_hits: format!("{}/{}", hits, denom),
        miss_winners: format_miss_winners(&winners),
    }
}

fn round4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}

/// Build the hit-test section: joint (adjusted, exclusion-aware) when both
/// sides have comparable sampled data; independent raw view when only one
/// side does; `None` (no section at all) when neither side has any.
fn build_hit_test_section(
    old_entry: Option<&HitTestEntry>,
    new_entry: Option<&HitTestEntry>,
) -> Option<HitTestExplain> {
    let old_points = sampled_points(old_entry);
    let new_points = sampled_points(new_entry);

    if old_points.is_none() && new_points.is_none() {
        return None;
    }

    match (old_points, new_points) {
        (Some(op), Some(np)) => match tally_points(op, np) {
            Some(tally) => Some(HitTestExplain {
                old: Some(HitTestSideExplain {
                    fraction: format!(
                        "{:.4}",
                        round4(if tally.denominator == 0 {
                            0.0
                        } else {
                            tally.hits_old as f64 / tally.denominator as f64
                        })
                    ),
                    raw_hits: format!("{}/{}", tally.hits_old, tally.denominator),
                    miss_winners: format_miss_winners(&tally.old_miss_winners),
                }),
                new: Some(HitTestSideExplain {
                    fraction: format!(
                        "{:.4}",
                        round4(if tally.denominator == 0 {
                            0.0
                        } else {
                            tally.hits_new as f64 / tally.denominator as f64
                        })
                    ),
                    raw_hits: format!("{}/{}", tally.hits_new, tally.denominator),
                    miss_winners: format_miss_winners(&tally.new_miss_winners),
                }),
            }),
            // Grid-size mismatch between sides (never emitted by capture, but
            // explain must never panic on hand-built/adversarial bundles) —
            // fall back to independent raw views for both.
            None => Some(HitTestExplain {
                old: Some(raw_side_explain(op)),
                new: Some(raw_side_explain(np)),
            }),
        },
        (Some(op), None) => Some(HitTestExplain {
            old: Some(raw_side_explain(op)),
            new: None,
        }),
        (None, Some(np)) => Some(HitTestExplain {
            old: None,
            new: Some(raw_side_explain(np)),
        }),
        (None, None) => unreachable!("checked above"),
    }
}

/// Find a node in `nodes` matching the locator.
/// When multiple nodes match, pick the one with the lowest (seq_index, id) — total order.
fn find_node<'a>(nodes: &'a [SemanticNode], locator: &Locator) -> Option<&'a SemanticNode> {
    let mut candidates: Vec<&SemanticNode> =
        nodes.iter().filter(|n| node_matches(n, locator)).collect();

    if candidates.is_empty() {
        return None;
    }

    // Total-order tie-break: (seq_index ASC, id ASC)
    candidates.sort_by(|a, b| a.seq_index.cmp(&b.seq_index).then_with(|| a.id.cmp(&b.id)));
    candidates.into_iter().next()
}

/// Return true if `node` matches the locator.
fn node_matches(node: &SemanticNode, locator: &Locator) -> bool {
    match locator {
        Locator::NodeId(id) => &node.id == id,

        Locator::Selector(sel) => {
            // Exact match first; fall back to substring.
            if let Some(css) = &node.css_selector {
                if css == sel {
                    return true;
                }
                // Substring fallback
                css.contains(sel.as_str())
            } else {
                false
            }
        }

        Locator::Anchor { key, value } => match key.as_str() {
            "text" => {
                // Check anchors.text first, then node.text
                let anchors_text = node.anchors.text.as_deref().unwrap_or("");
                let node_text = node.text.as_deref().unwrap_or("");
                anchors_text.contains(value.as_str()) || node_text.contains(value.as_str())
            }
            "role" => {
                let anchors_role = node.anchors.role.as_deref().unwrap_or("");
                let node_role = node.role.as_deref().unwrap_or("");
                anchors_role.contains(value.as_str()) || node_role.contains(value.as_str())
            }
            "href" => {
                let anchors_href = node.anchors.href.as_deref().unwrap_or("");
                let node_href = node.href.as_deref().unwrap_or("");
                anchors_href.contains(value.as_str()) || node_href.contains(value.as_str())
            }
            "nearestHeading" => {
                let heading = node.anchors.nearest_heading.as_deref().unwrap_or("");
                heading.contains(value.as_str())
            }
            _ => false,
        },
    }
}

/// Build the sorted property rows from the two resolved sides.
fn build_rows(
    old_side: &SideResult,
    new_side: &SideResult,
    props: Option<&[String]>,
) -> Vec<PropRow> {
    let old_map = side_style_map(old_side);
    let new_map = side_style_map(new_side);
    diff_prop_rows(
        &old_map,
        &new_map,
        &["bbox.x", "bbox.y", "bbox.w", "bbox.h"],
        props,
    )
}

/// Materialize a node `SideResult`'s computed styles + bbox pseudo-props
/// (`bbox.x`/`bbox.y`/`bbox.w`/`bbox.h`) into one flat string map, so both the
/// node path and the pseudo path (`explain_pseudo`, port-parity U10) can share
/// the SAME diff/`--props` logic (`diff_prop_rows`) — never a second
/// implementation.
fn side_style_map(side: &SideResult) -> BTreeMap<String, String> {
    let mut m = side.computed_styles.clone();
    if let Some(bbox) = side.bbox {
        m.insert("bbox.x".to_string(), bbox[0].to_string());
        m.insert("bbox.y".to_string(), bbox[1].to_string());
        m.insert("bbox.w".to_string(), bbox[2].to_string());
        m.insert("bbox.h".to_string(), bbox[3].to_string());
    }
    m
}

/// Build property rows from two already-materialized `{property: value}`
/// maps — shared by the node path (`build_rows`, via `side_style_map`) and
/// the pseudo `--selector "...::before"`/`"...::after"` path
/// (`explain_pseudo`, port-parity U10). Diff-only default when `props` is
/// `None` (union of both maps' keys plus `extra_diff_candidates`, filtered to
/// differing values); explicit sorted list otherwise. Absent key -> `<absent>`.
fn diff_prop_rows(
    old_map: &BTreeMap<String, String>,
    new_map: &BTreeMap<String, String>,
    extra_diff_candidates: &[&str],
    props: Option<&[String]>,
) -> Vec<PropRow> {
    const ABSENT: &str = "<absent>";

    let get_val = |map: &BTreeMap<String, String>, prop: &str| -> String {
        map.get(prop).cloned().unwrap_or_else(|| ABSENT.to_string())
    };

    let keys: Vec<String> = if let Some(explicit) = props {
        // Explicit --props: always show these in order (sorted for determinism).
        let mut sorted = explicit.to_vec();
        sorted.sort();
        sorted
    } else {
        // Diff-only default: union of both sides' keys + extra candidates,
        // filtered to those that differ.
        let mut all_keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for k in extra_diff_candidates {
            all_keys.insert(k.to_string());
        }
        for k in old_map.keys() {
            all_keys.insert(k.clone());
        }
        for k in new_map.keys() {
            all_keys.insert(k.clone());
        }
        all_keys
            .into_iter()
            .filter(|k| get_val(old_map, k) != get_val(new_map, k))
            .collect()
    };

    keys.into_iter()
        .map(|prop| {
            let old_value = get_val(old_map, &prop);
            let new_value = get_val(new_map, &prop);
            let changed = old_value != new_value;
            PropRow {
                property: prop,
                old_value,
                new_value,
                changed,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Formatting helpers (used by the CLI handler in matchy.rs)
// ---------------------------------------------------------------------------

/// Format the `ExplainReport` as a human-readable table string.
///
/// Output is byte-deterministic (properties sorted alphabetically, BTreeMap iteration).
pub fn format_report(report: &ExplainReport, locator_str: &str) -> String {
    let mut out = String::new();

    // Header line
    out.push_str(&format!("matchy explain — locator: {}\n", locator_str));
    out.push('\n');

    // Per-side resolution summary
    match (&report.old.status, &report.new.status) {
        (ResolutionStatus::Resolved, ResolutionStatus::Resolved) => {
            out.push_str(&format!(
                "old: resolved  node={}\n",
                report.old.node_id.as_deref().unwrap_or("?")
            ));
            push_owned_pseudo_note(&mut out, &report.old);
            out.push_str(&format!(
                "new: resolved  node={}\n",
                report.new.node_id.as_deref().unwrap_or("?")
            ));
            push_owned_pseudo_note(&mut out, &report.new);
        }
        (ResolutionStatus::Resolved, ResolutionStatus::NotFound) => {
            out.push_str(&format!(
                "old: resolved  node={}\n",
                report.old.node_id.as_deref().unwrap_or("?")
            ));
            push_owned_pseudo_note(&mut out, &report.old);
            out.push_str("new: NOT FOUND\n");
        }
        (ResolutionStatus::NotFound, ResolutionStatus::Resolved) => {
            out.push_str("old: NOT FOUND\n");
            out.push_str(&format!(
                "new: resolved  node={}\n",
                report.new.node_id.as_deref().unwrap_or("?")
            ));
            push_owned_pseudo_note(&mut out, &report.new);
        }
        (ResolutionStatus::NotFound, ResolutionStatus::NotFound) => {
            out.push_str("old: NOT FOUND\n");
            out.push_str("new: NOT FOUND\n");
        }
    }

    if let Some(msg) = &report.asymmetry_message {
        out.push('\n');
        out.push_str(&format!("NOTE: {}\n", msg));
    }

    out.push('\n');

    push_hit_test_section(&mut out, report.hit_test.as_ref());

    if report.rows.is_empty() {
        out.push_str("(no properties to display)\n");
        return out;
    }

    // Compute column widths for alignment.
    let prop_w = report
        .rows
        .iter()
        .map(|r| r.property.len())
        .max()
        .unwrap_or(8)
        .max(8); // "property"
    let old_w = report
        .rows
        .iter()
        .map(|r| r.old_value.len())
        .max()
        .unwrap_or(3)
        .max(3); // "old"
    let new_w = report
        .rows
        .iter()
        .map(|r| r.new_value.len())
        .max()
        .unwrap_or(3)
        .max(3); // "new"

    // Header row
    out.push_str(&format!(
        "{:<prop_w$}  {:<old_w$}  {:<new_w$}  {}\n",
        "property",
        "old",
        "new",
        "changed?",
        prop_w = prop_w,
        old_w = old_w,
        new_w = new_w
    ));
    // Separator
    out.push_str(&format!(
        "{:-<prop_w$}  {:-<old_w$}  {:-<new_w$}  --------\n",
        "",
        "",
        "",
        prop_w = prop_w,
        old_w = old_w,
        new_w = new_w
    ));

    for row in &report.rows {
        let changed_str = if row.changed { "CHANGED" } else { "" };
        out.push_str(&format!(
            "{:<prop_w$}  {:<old_w$}  {:<new_w$}  {}\n",
            row.property,
            row.old_value,
            row.new_value,
            changed_str,
            prop_w = prop_w,
            old_w = old_w,
            new_w = new_w
        ));
    }

    out
}

/// Port-parity U10: when a resolved node owns painted pseudo-elements, append
/// a short note line so users know to re-query with `--selector
/// "...::before"` / `"...::after"` — deliberately NOT a dump of their styles
/// by default.
fn push_owned_pseudo_note(out: &mut String, side: &SideResult) {
    if side.owned_pseudo_slots.is_empty() {
        return;
    }
    out.push_str(&format!(
        "    pseudo-elements: {}\n",
        side.owned_pseudo_slots.join(", ")
    ));
}

/// Append the hit-test (clickable-area) section, when present. Deterministic
/// formatting: fixed field order, `<absent>` for a side with no comparable data.
fn push_hit_test_section(out: &mut String, hit_test: Option<&HitTestExplain>) {
    let Some(ht) = hit_test else { return };

    out.push_str("hit-test (clickable-area):\n");
    push_hit_test_side(out, "old", ht.old.as_ref());
    push_hit_test_side(out, "new", ht.new.as_ref());
    out.push('\n');
}

fn push_hit_test_side(out: &mut String, side: &str, view: Option<&HitTestSideExplain>) {
    match view {
        None => out.push_str(&format!("  {}: <absent>\n", side)),
        Some(v) => {
            let winners = if v.miss_winners.is_empty() {
                "(none)"
            } else {
                v.miss_winners.as_str()
            };
            out.push_str(&format!(
                "  {}: fraction={} rawHits={} missWinners={}\n",
                side, v.fraction, v.raw_hits, winners
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        A11yInfo, CaptureDeterminism, Environment, NetworkInfo, NodeAnchors, PageModel,
        Screenshots, SemanticNode, StepStatus, StyleCandidates, ViewportConfig,
    };
    use std::collections::BTreeMap;

    fn make_det() -> CaptureDeterminism {
        CaptureDeterminism {
            animations_disabled: StepStatus::Ran,
            reduced_motion: StepStatus::Ran,
            time_frozen: StepStatus::Ran,
            random_stubbed: StepStatus::Ran,
            fonts_ready: StepStatus::Ran,
            images_decoded: StepStatus::Ran,
            lazy_load_pass: StepStatus::Ran,
            settled: StepStatus::Ran,
            settle: None,
            hit_test_probe: None,
            quiescence: None,
            settle_scroll_ineffective: None,
            settle_growth_capped: None,
            clicked: vec![],
            hidden: vec![],
            masked: vec![],
            retried_without_time_freeze: false,
            integrity: None,
        }
    }

    fn make_node(
        id: &str,
        text: Option<&str>,
        role: Option<&str>,
        href: Option<&str>,
        css_selector: Option<&str>,
        nearest_heading: Option<&str>,
        bbox: [i32; 4],
        seq_index: u32,
    ) -> SemanticNode {
        SemanticNode {
            id: id.to_string(),
            kind: "button".to_string(),
            role: role.map(str::to_string),
            text: text.map(str::to_string),
            acc_name: None,
            href: href.map(str::to_string),
            image_alt: None,
            bbox,
            seq_index,
            anchors: NodeAnchors {
                text: text.map(str::to_string),
                role: role.map(str::to_string),
                href: href.map(str::to_string),
                alt: None,
                aria_label: None,
                nearest_heading: nearest_heading.map(str::to_string),
                landmark: None,
                ordinal_in_landmark: None,
            },
            css_selector: css_selector.map(str::to_string),
            raw_href: None,
            src: None,
            natural_width: None,
            natural_height: None,
            loaded: None,
            heading_level: None,
            has_onclick: None,
        }
    }

    fn make_bundle(
        url: &str,
        nodes: Vec<SemanticNode>,
        computed_styles: BTreeMap<String, BTreeMap<String, String>>,
    ) -> CaptureBundle {
        CaptureBundle {
            schema_version: "1.0".to_string(),
            captured_at: "2026-01-01T00:00:00Z".to_string(),
            viewport: ViewportConfig {
                name: "desktop".to_string(),
                width: 1440,
                height: 900,
                dsf: 1.0,
            },
            environment: Environment {
                os: "linux".to_string(),
                chromium_build: "1234".to_string(),
                playwright: "1.60.0".to_string(),
                dsf: 1.0,
            },
            determinism: make_det(),
            page: PageModel {
                url: url.to_string(),
                final_url: url.to_string(),
                redirect_chain: vec![],
                status_code: 200,
                title: None,
                meta_description: None,
                canonical: None,
                lang: Some("en".to_string()),
                page_height: 2000,
                nodes,
                landmarks: vec![],
                landmark_rects: None,
                network: NetworkInfo { requests: vec![] },
                console: vec![],
                a11y: A11yInfo { violations: vec![] },
                link_probes: vec![],
            },
            computed_styles,
            screenshots: Screenshots {
                full_page: "desktop/old.png".to_string(),
                viewport: "desktop/old-vp.png".to_string(),
            },
            style_candidates: StyleCandidates::default(),
            hit_tests: None,
            pseudo_elements: None,
            pseudo_truncated: None,
        }
    }

    // -------------------------------------------------------------------------
    // Helper: Build two bundles with a CTA node that has a style change
    // -------------------------------------------------------------------------
    fn make_cta_bundles() -> (CaptureBundle, CaptureBundle) {
        let cta_old = make_node(
            "node_cta_old",
            Some("Get started"),
            Some("button"),
            None,
            Some(".hero .cta"),
            Some("Hero"),
            [100, 200, 150, 50],
            3,
        );
        let cta_new = make_node(
            "node_cta_new",
            Some("Get started"),
            Some("button"),
            None,
            Some(".hero .cta"),
            Some("Hero"),
            [100, 200, 150, 50],
            3,
        );
        let other_old = make_node(
            "node_other",
            Some("Sign in"),
            None,
            None,
            None,
            None,
            [0, 0, 100, 40],
            0,
        );

        let mut old_styles: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut cta_old_style: BTreeMap<String, String> = BTreeMap::new();
        cta_old_style.insert(
            "background-image".to_string(),
            "linear-gradient(90deg, #0070f3, #00c6ff)".to_string(),
        );
        cta_old_style.insert("color".to_string(), "#ffffff".to_string());
        cta_old_style.insert("font-family".to_string(), "Inter, sans-serif".to_string());
        old_styles.insert("node_cta_old".to_string(), cta_old_style);

        let mut new_styles: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut cta_new_style: BTreeMap<String, String> = BTreeMap::new();
        cta_new_style.insert("background-image".to_string(), "none".to_string());
        cta_new_style.insert("color".to_string(), "#ffffff".to_string()); // unchanged
        cta_new_style.insert("font-family".to_string(), "Inter, sans-serif".to_string()); // unchanged
        new_styles.insert("node_cta_new".to_string(), cta_new_style);

        let old_bundle = make_bundle(
            "http://old.example.com/",
            vec![other_old, cta_old],
            old_styles,
        );
        let new_bundle = make_bundle("http://new.example.com/", vec![cta_new], new_styles);

        (old_bundle, new_bundle)
    }

    // -------------------------------------------------------------------------
    // Test A1: anchor text=Get started resolves CTA; diff-only includes changed prop,
    //          excludes unchanged props.
    // -------------------------------------------------------------------------
    #[test]
    fn test_anchor_text_diff_only() {
        let (old_bundle, new_bundle) = make_cta_bundles();
        let locator = Locator::parse_anchor("text=Get started").unwrap();
        let report = explain(&old_bundle, &new_bundle, &locator, None);

        // Both sides resolved
        assert_eq!(report.old.status, ResolutionStatus::Resolved);
        assert_eq!(report.new.status, ResolutionStatus::Resolved);
        assert_eq!(report.old.node_id.as_deref(), Some("node_cta_old"));
        assert_eq!(report.new.node_id.as_deref(), Some("node_cta_new"));

        // background-image changed: must appear
        let bg_row = report
            .rows
            .iter()
            .find(|r| r.property == "background-image")
            .expect("background-image must be in rows");
        assert!(bg_row.changed, "background-image must be marked changed");
        assert!(
            bg_row.old_value.contains("gradient"),
            "old background-image must contain gradient"
        );
        assert_eq!(bg_row.new_value, "none");

        // color and font-family are unchanged — must NOT appear in diff-only output
        let unchanged_props: Vec<&str> = report
            .rows
            .iter()
            .filter(|r| !r.changed)
            .map(|r| r.property.as_str())
            .collect();
        assert!(
            !unchanged_props.contains(&"color"),
            "color must not appear in diff-only output"
        );
        assert!(
            !unchanged_props.contains(&"font-family"),
            "font-family must not appear in diff-only output"
        );

        // All rows in diff-only mode must be changed
        for row in &report.rows {
            // bbox props may differ too (different nodes) - that's expected
            assert!(
                row.changed,
                "diff-only must only include changed rows, got unchanged: {}",
                row.property
            );
        }
    }

    // -------------------------------------------------------------------------
    // Test A2: --props color,font-family restricts output to exactly those props,
    //          even when both are unchanged.
    // -------------------------------------------------------------------------
    #[test]
    fn test_explicit_props_shows_unchanged() {
        let (old_bundle, new_bundle) = make_cta_bundles();
        let locator = Locator::parse_anchor("text=Get started").unwrap();
        let props = vec!["color".to_string(), "font-family".to_string()];
        let report = explain(&old_bundle, &new_bundle, &locator, Some(&props));

        // Both sides resolved
        assert_eq!(report.old.status, ResolutionStatus::Resolved);
        assert_eq!(report.new.status, ResolutionStatus::Resolved);

        // Exactly 2 rows: color and font-family
        assert_eq!(
            report.rows.len(),
            2,
            "must have exactly 2 rows for explicit props"
        );
        let prop_names: Vec<&str> = report.rows.iter().map(|r| r.property.as_str()).collect();
        assert!(prop_names.contains(&"color"), "color must be present");
        assert!(
            prop_names.contains(&"font-family"),
            "font-family must be present"
        );

        // background-image must NOT appear (not in --props)
        assert!(
            !prop_names.contains(&"background-image"),
            "background-image must not appear when not in --props"
        );

        // Both must be marked NOT changed
        for row in &report.rows {
            assert!(
                !row.changed,
                "color/font-family must not be changed; got changed=true for {}",
                row.property
            );
        }
    }

    // -------------------------------------------------------------------------
    // Test A3: --node <id> resolves the same CTA node as the anchor.
    // -------------------------------------------------------------------------
    #[test]
    fn test_node_id_resolves_cta() {
        let (old_bundle, new_bundle) = make_cta_bundles();
        let locator = Locator::NodeId("node_cta_old".to_string());
        let report = explain(&old_bundle, &new_bundle, &locator, None);

        // Old side resolves to the CTA node
        assert_eq!(report.old.status, ResolutionStatus::Resolved);
        assert_eq!(report.old.node_id.as_deref(), Some("node_cta_old"));

        // New side: node_cta_new has a different id — not found by this locator
        // That is expected: --node uses exact id match
        assert_eq!(report.new.status, ResolutionStatus::NotFound);
    }

    // -------------------------------------------------------------------------
    // Test A4: --selector ".hero .cta" resolves the CTA node on both sides (both have that css_selector).
    // -------------------------------------------------------------------------
    #[test]
    fn test_selector_resolves_cta() {
        let (old_bundle, new_bundle) = make_cta_bundles();
        let locator = Locator::Selector(".hero .cta".to_string());
        let report = explain(&old_bundle, &new_bundle, &locator, None);

        // Both sides have css_selector = ".hero .cta"
        assert_eq!(report.old.status, ResolutionStatus::Resolved);
        assert_eq!(report.new.status, ResolutionStatus::Resolved);
        assert_eq!(report.old.node_id.as_deref(), Some("node_cta_old"));
        assert_eq!(report.new.node_id.as_deref(), Some("node_cta_new"));
    }

    // -------------------------------------------------------------------------
    // Test A5: node present in new only → reports asymmetry, no panic.
    // -------------------------------------------------------------------------
    #[test]
    fn test_one_side_only_no_panic() {
        let (old_bundle, new_bundle) = make_cta_bundles();
        // Use a text that only exists in the new bundle (new has "Get started",
        // old also has it — so use a node_id that only exists in new).
        let locator = Locator::NodeId("node_cta_new".to_string());
        let report = explain(&old_bundle, &new_bundle, &locator, None);

        // Old side should NOT resolve (no node with id=node_cta_new in old)
        assert_eq!(report.old.status, ResolutionStatus::NotFound);
        // New side resolves
        assert_eq!(report.new.status, ResolutionStatus::Resolved);
        assert_eq!(report.new.node_id.as_deref(), Some("node_cta_new"));

        // Asymmetry message must be present
        assert!(report.asymmetry_message.is_some());
        let msg = report.asymmetry_message.as_deref().unwrap();
        assert!(
            msg.contains("NEW only"),
            "asymmetry message must mention NEW only"
        );
    }

    // -------------------------------------------------------------------------
    // Test A6: locator matching no nodes → both sides NotFound, no panic.
    // -------------------------------------------------------------------------
    #[test]
    fn test_not_found_on_both_sides() {
        let (old_bundle, new_bundle) = make_cta_bundles();
        let locator = Locator::parse_anchor("text=DOES_NOT_EXIST_ANYWHERE").unwrap();
        let report = explain(&old_bundle, &new_bundle, &locator, None);

        assert_eq!(report.old.status, ResolutionStatus::NotFound);
        assert_eq!(report.new.status, ResolutionStatus::NotFound);
        assert!(report.asymmetry_message.is_none());
        assert!(report.rows.is_empty());
    }

    // -------------------------------------------------------------------------
    // Test A7: anchor key validation.
    // -------------------------------------------------------------------------
    #[test]
    fn test_anchor_key_validation() {
        assert!(Locator::parse_anchor("text=foo").is_ok());
        assert!(Locator::parse_anchor("role=button").is_ok());
        assert!(Locator::parse_anchor("href=/about").is_ok());
        assert!(Locator::parse_anchor("nearestHeading=Hero").is_ok());
        assert!(Locator::parse_anchor("badkey=foo").is_err());
        assert!(Locator::parse_anchor("no-equals").is_err());
    }

    // -------------------------------------------------------------------------
    // Test A8: tie-break determinism — multiple matches sorted by (seq_index, id).
    // -------------------------------------------------------------------------
    #[test]
    fn test_tie_break_determinism() {
        // Two nodes both with text="Get started"; lower seq_index wins.
        let node_a = make_node(
            "node_b", // id sorts after node_a but seq_index is higher
            Some("Get started"),
            None,
            None,
            None,
            None,
            [0, 300, 100, 40],
            5,
        );
        let node_b = make_node(
            "node_a", // id sorts first
            Some("Get started"),
            None,
            None,
            None,
            None,
            [0, 100, 100, 40],
            2, // lower seq_index → wins
        );
        let bundle = make_bundle("http://example.com/", vec![node_a, node_b], BTreeMap::new());

        let locator = Locator::parse_anchor("text=Get started").unwrap();
        let side = resolve_side(&bundle, &locator);

        assert_eq!(side.status, ResolutionStatus::Resolved);
        // node_b has seq_index=2 < node_a's seq_index=5, so node_b (id=node_a) wins
        assert_eq!(side.node_id.as_deref(), Some("node_a"));
    }

    // -------------------------------------------------------------------------
    // Test A9: format_report output is byte-deterministic and contains expected content.
    // -------------------------------------------------------------------------
    #[test]
    fn test_format_report_deterministic() {
        let (old_bundle, new_bundle) = make_cta_bundles();
        let locator = Locator::parse_anchor("text=Get started").unwrap();
        let report = explain(&old_bundle, &new_bundle, &locator, None);

        let out1 = format_report(&report, "text=Get started");
        let out2 = format_report(&report, "text=Get started");
        assert_eq!(out1, out2, "format_report must be byte-deterministic");

        // Must contain the column headers
        assert!(out1.contains("property"), "must contain 'property' header");
        assert!(out1.contains("old"), "must contain 'old' header");
        assert!(out1.contains("new"), "must contain 'new' header");
        assert!(out1.contains("changed?"), "must contain 'changed?' header");

        // Must mark the background-image row as CHANGED
        assert!(out1.contains("CHANGED"), "must mark changed rows");
        assert!(
            out1.contains("background-image"),
            "must show background-image"
        );
    }

    // -------------------------------------------------------------------------
    // port-parity U7: hit-test section
    // -------------------------------------------------------------------------

    fn hit_pt(o: HitTestOutcome, winner: Option<&str>) -> HitTestPoint {
        HitTestPoint {
            o,
            winner: winner.map(str::to_string),
        }
    }

    fn sampled_entry(points: Vec<HitTestPoint>) -> HitTestEntry {
        HitTestEntry {
            status: HitTestStatus::Sampled,
            skip_reason: None,
            grid_size: Some(5),
            points: Some(points),
        }
    }

    fn make_bundle_with_hit_tests(
        url: &str,
        nodes: Vec<SemanticNode>,
        hit_tests: BTreeMap<String, HitTestEntry>,
    ) -> CaptureBundle {
        let mut bundle = make_bundle(url, nodes, BTreeMap::new());
        bundle.hit_tests = Some(hit_tests);
        bundle
    }

    /// Both sides have comparable sampled data -> the joint (adjusted) view is
    /// used, including winners on both old and new sides.
    #[test]
    fn test_hit_test_section_joint_view_both_sides() {
        let node_old = make_node(
            "n_cta",
            Some("Get started"),
            None,
            None,
            None,
            None,
            [0, 0, 100, 40],
            0,
        );
        let node_new = make_node(
            "n_cta",
            Some("Get started"),
            None,
            None,
            None,
            None,
            [0, 0, 100, 40],
            0,
        );

        let old_points = (0..25)
            .map(|_| hit_pt(HitTestOutcome::Hit, None))
            .collect::<Vec<_>>();
        let mut new_points = (0..3)
            .map(|_| hit_pt(HitTestOutcome::Hit, None))
            .collect::<Vec<_>>();
        new_points.extend((0..22).map(|_| hit_pt(HitTestOutcome::Miss, Some("img.overlay"))));

        let mut old_ht = BTreeMap::new();
        old_ht.insert("n_cta".to_string(), sampled_entry(old_points));
        let mut new_ht = BTreeMap::new();
        new_ht.insert("n_cta".to_string(), sampled_entry(new_points));

        let old_bundle =
            make_bundle_with_hit_tests("http://old.example.com/", vec![node_old], old_ht);
        let new_bundle =
            make_bundle_with_hit_tests("http://new.example.com/", vec![node_new], new_ht);

        let locator = Locator::parse_anchor("text=Get started").unwrap();
        let report = explain(&old_bundle, &new_bundle, &locator, None);

        let ht = report
            .hit_test
            .clone()
            .expect("hit_test section must be present");
        let old_view = ht.old.expect("old side present");
        let new_view = ht.new.expect("new side present");
        assert_eq!(old_view.fraction, "1.0000");
        assert_eq!(old_view.raw_hits, "25/25");
        assert_eq!(new_view.fraction, "0.1200");
        assert_eq!(new_view.raw_hits, "3/25");
        assert_eq!(new_view.miss_winners, "img.overlay (x22)");

        let out = format_report(&report, "text=Get started");
        assert!(out.contains("hit-test (clickable-area):"));
        assert!(out.contains("fraction=1.0000"));
        assert!(out.contains("fraction=0.1200"));
        assert!(out.contains("img.overlay (x22)"));
    }

    /// Only one side has hit-test data -> the other side prints `<absent>`,
    /// and the present side's fraction is the raw (unpaired) fraction.
    #[test]
    fn test_hit_test_section_one_side_absent() {
        let node_old = make_node(
            "n_cta",
            Some("Get started"),
            None,
            None,
            None,
            None,
            [0, 0, 100, 40],
            0,
        );
        let node_new = make_node(
            "n_cta_new",
            Some("Get started"),
            None,
            None,
            None,
            None,
            [0, 0, 100, 40],
            0,
        );

        let old_points = (0..25)
            .map(|_| hit_pt(HitTestOutcome::Hit, None))
            .collect::<Vec<_>>();
        let mut old_ht = BTreeMap::new();
        old_ht.insert("n_cta".to_string(), sampled_entry(old_points));

        let old_bundle =
            make_bundle_with_hit_tests("http://old.example.com/", vec![node_old], old_ht);
        // New bundle has no hitTests channel at all.
        let new_bundle = make_bundle("http://new.example.com/", vec![node_new], BTreeMap::new());

        let locator = Locator::parse_anchor("text=Get started").unwrap();
        let report = explain(&old_bundle, &new_bundle, &locator, None);

        let ht = report
            .hit_test
            .clone()
            .expect("hit_test section must be present");
        assert!(ht.old.is_some());
        assert!(ht.new.is_none());
        assert_eq!(ht.old.unwrap().raw_hits, "25/25");

        let out = format_report(&report, "text=Get started");
        assert!(out.contains("new: <absent>"));
    }

    /// Neither side has hit-test data -> no section at all (no panic, no
    /// spurious header).
    #[test]
    fn test_hit_test_section_absent_on_both_no_section() {
        let (old_bundle, new_bundle) = make_cta_bundles();
        let locator = Locator::parse_anchor("text=Get started").unwrap();
        let report = explain(&old_bundle, &new_bundle, &locator, None);

        assert!(report.hit_test.is_none());
        let out = format_report(&report, "text=Get started");
        assert!(!out.contains("hit-test (clickable-area):"));
    }

    // -------------------------------------------------------------------------
    // Pseudo-element locator (port-parity U10)
    // -------------------------------------------------------------------------

    use crate::contract::{PseudoElementEntry, PseudoOwnerTier, PseudoStyles};

    fn make_bundle_with_pseudo(
        url: &str,
        nodes: Vec<SemanticNode>,
        pseudo_elements: BTreeMap<String, PseudoElementEntry>,
    ) -> CaptureBundle {
        let mut bundle = make_bundle(url, nodes, BTreeMap::new());
        bundle.pseudo_elements = Some(pseudo_elements);
        bundle
    }

    fn tick_style(background_color: &str) -> PseudoStyles {
        PseudoStyles {
            content: "\"\"".to_string(),
            position: Some("absolute".to_string()),
            width: Some("10px".to_string()),
            height: Some("10px".to_string()),
            background_color: Some(background_color.to_string()),
            background_image: None,
            border: None,
            border_radius: None,
            top: Some("0px".to_string()),
            right: None,
            bottom: None,
            left: Some("0px".to_string()),
            z_index: None,
            display: Some("block".to_string()),
            opacity: Some("1".to_string()),
            bbox: Some([10.0, 20.0, 10.0, 10.0]),
        }
    }

    fn selector_entry(selector: &str, before: Option<PseudoStyles>) -> PseudoElementEntry {
        selector_entry_full(selector, before, None)
    }

    fn selector_entry_full(
        selector: &str,
        before: Option<PseudoStyles>,
        after: Option<PseudoStyles>,
    ) -> PseudoElementEntry {
        PseudoElementEntry {
            owner_tier: PseudoOwnerTier::Selector,
            owner_node_id: None,
            owner_selector: Some(selector.to_string()),
            landmark: Some("main".to_string()),
            before,
            after,
        }
    }

    /// `--selector '[section-style="overlap"]::after'`-style locator resolves
    /// via the exact `ownerSelector` match (tier "selector") — the acceptance
    /// case for the golden-page owner key, which carries a landmark prefix the
    /// bare selector does not.
    #[test]
    fn test_explain_pseudo_resolves_via_owner_selector_match() {
        let sel = "[section-style=\"overlap\"]";
        let key = format!("main::{}", sel);

        let mut old_pseudo = BTreeMap::new();
        old_pseudo.insert(
            key.clone(),
            selector_entry_full(sel, None, Some(tick_style("rgb(255, 0, 0)"))),
        );
        let mut new_pseudo = BTreeMap::new();
        new_pseudo.insert(
            key,
            selector_entry_full(sel, None, Some(tick_style("rgb(0, 0, 255)"))),
        );

        let old_bundle = make_bundle_with_pseudo("http://old.example.com/", vec![], old_pseudo);
        let new_bundle = make_bundle_with_pseudo("http://new.example.com/", vec![], new_pseudo);

        let (owner_part, slot) =
            parse_pseudo_selector(&format!("{}::after", sel)).expect("::after suffix must parse");
        let report = explain_pseudo(&old_bundle, &new_bundle, &owner_part, slot, None);

        assert_eq!(report.old.status, ResolutionStatus::Resolved);
        assert_eq!(report.new.status, ResolutionStatus::Resolved);
        assert_eq!(report.asymmetry_message, None);
        let row = report
            .rows
            .iter()
            .find(|r| r.property == "background-color")
            .expect("background-color row present (diff-only default)");
        assert_eq!(row.old_value, "rgb(255, 0, 0)");
        assert_eq!(row.new_value, "rgb(0, 0, 255)");
        assert!(row.changed);
    }

    /// One-side-present (old painted, new absent) is not an error: exit-0
    /// convention preserved, asymmetry reported.
    #[test]
    fn test_explain_pseudo_one_side_present_asymmetry_reported() {
        let sel = "[data-hr-corner-top]";
        let key = format!("main::{}", sel);

        let mut old_pseudo = BTreeMap::new();
        old_pseudo.insert(
            key.clone(),
            selector_entry(sel, Some(tick_style("rgb(255, 0, 0)"))),
        );
        let mut new_pseudo = BTreeMap::new();
        new_pseudo.insert(key, selector_entry(sel, None));

        let old_bundle = make_bundle_with_pseudo("http://old.example.com/", vec![], old_pseudo);
        let new_bundle = make_bundle_with_pseudo("http://new.example.com/", vec![], new_pseudo);

        let (owner_part, slot) =
            parse_pseudo_selector(&format!("{}::before", sel)).expect("::before suffix must parse");
        let report = explain_pseudo(&old_bundle, &new_bundle, &owner_part, slot, None);

        assert_eq!(report.old.status, ResolutionStatus::Resolved);
        assert_eq!(report.new.status, ResolutionStatus::NotFound);
        let msg = report.asymmetry_message.expect("asymmetry message present");
        assert!(msg.contains("::before"));
        assert!(msg.contains("OLD only"));

        // Exit-0 convention: both_not_found is false here, mirroring the
        // node-locator path's contract.
        let both_not_found = report.old.status == ResolutionStatus::NotFound
            && report.new.status == ResolutionStatus::NotFound;
        assert!(!both_not_found);
    }

    /// `--props` filtering applies on the pseudo path too.
    #[test]
    fn test_explain_pseudo_props_filter() {
        let sel = "[data-hr-corner-top]";
        let key = format!("main::{}", sel);
        let mut old_pseudo = BTreeMap::new();
        old_pseudo.insert(
            key.clone(),
            selector_entry(sel, Some(tick_style("rgb(255, 0, 0)"))),
        );
        let mut new_pseudo = BTreeMap::new();
        new_pseudo.insert(key, selector_entry(sel, Some(tick_style("rgb(255, 0, 0)"))));

        let old_bundle = make_bundle_with_pseudo("http://old.example.com/", vec![], old_pseudo);
        let new_bundle = make_bundle_with_pseudo("http://new.example.com/", vec![], new_pseudo);

        let (owner_part, slot) =
            parse_pseudo_selector(&format!("{}::before", sel)).expect("::before suffix must parse");
        let props = vec!["content".to_string(), "width".to_string()];
        let report = explain_pseudo(&old_bundle, &new_bundle, &owner_part, slot, Some(&props));

        let props_shown: Vec<&str> = report.rows.iter().map(|r| r.property.as_str()).collect();
        assert_eq!(props_shown, vec!["content", "width"]);
    }

    /// A node that owns pseudos gets a short note line on the ordinary
    /// `--anchor`/`--node` path — not a dump of their styles by default.
    #[test]
    fn test_node_locator_appends_owned_pseudo_note() {
        let node_old = make_node(
            "n_cta",
            Some("Get started"),
            None,
            None,
            None,
            None,
            [0, 0, 100, 40],
            0,
        );
        let node_new = make_node(
            "n_cta",
            Some("Get started"),
            None,
            None,
            None,
            None,
            [0, 0, 100, 40],
            0,
        );
        let mut pseudo = BTreeMap::new();
        pseudo.insert(
            "n_cta".to_string(),
            PseudoElementEntry {
                owner_tier: PseudoOwnerTier::Node,
                owner_node_id: Some("n_cta".to_string()),
                owner_selector: None,
                landmark: Some("main".to_string()),
                before: Some(tick_style("rgb(255, 0, 0)")),
                after: Some(tick_style("rgb(0, 0, 255)")),
            },
        );

        let old_bundle = make_bundle_with_pseudo("http://old.example.com/", vec![node_old], pseudo);
        let new_bundle = make_bundle("http://new.example.com/", vec![node_new], BTreeMap::new());

        let locator = Locator::parse_anchor("text=Get started").unwrap();
        let report = explain(&old_bundle, &new_bundle, &locator, None);
        assert_eq!(
            report.old.owned_pseudo_slots,
            vec!["::before".to_string(), "::after".to_string()]
        );
        assert!(report.new.owned_pseudo_slots.is_empty());

        let out = format_report(&report, "text=Get started");
        assert!(out.contains("pseudo-elements: ::before, ::after"));
        // Not a dump: the pseudo styles themselves must not appear as rows.
        assert!(!out.contains("background-color"));
    }

    /// `parse_pseudo_selector` round-trips and leaves ordinary selectors alone.
    #[test]
    fn test_parse_pseudo_selector() {
        assert_eq!(
            parse_pseudo_selector("#hr-corner::before"),
            Some(("#hr-corner".to_string(), PseudoSlot::Before))
        );
        assert_eq!(
            parse_pseudo_selector("#hr-corner::after"),
            Some(("#hr-corner".to_string(), PseudoSlot::After))
        );
        assert_eq!(parse_pseudo_selector("#hr-corner"), None);
    }
}
