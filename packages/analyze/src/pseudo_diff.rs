//! Pseudo-element (`::before`/`::after`) diff (port-parity U10).
//!
//! Consumes both bundles' `pseudoElements` maps (U9) plus two alignment
//! primitives factored out of `style_diff.rs` and reused verbatim here —
//! never a second implementation:
//!   - `style_diff::matched_old_to_new_id` (the matcher's Matched-band
//!     old-node-id -> new-node-id map), for tier "node" owner alignment.
//!   - `style_diff::build_ancestor_pairing` (the ancestor style channel's
//!     descendant-set + style-similarity old<->new ancestor pairing), for
//!     tier "ancestor" owner alignment. It shares the exact same key space as
//!     `computedStyles`'s ancestor entries (`AncestorDescriptor::id`, e.g.
//!     `anc_3`) — the same ids capture uses as `pseudoElements` map keys for
//!     tier "ancestor" entries.
//!
//! Tier "selector" (decorative leaves with no semantic-node/ancestor
//! identity) aligns iff the exact `pseudoElements` map key string exists on
//! both sides — there is no fallback tier for these owners, so an unaligned
//! tier-"selector" owner with a painted pseudo still emits (at demoted
//! confidence) rather than going silent (design brief U10 step 3).
//!
//! Painted-pseudo property diffs run through the SAME canonicalization
//! ladder (`style_diff::normalize_value_with_page_url` /
//! `canonicalize_for_compare` / C2 / C3) the leaf/ancestor style channels use.
//!
//! New-side-only painted pseudos are deliberately never surfaced by this pass
//! (a `pseudo_element_added` type is deferred — design brief U10 step 4):
//! the owner loop below only ever iterates the OLD bundle's `pseudoElements`
//! map, so an owner/slot painted only on the new side is simply never
//! visited.
//!
//! DETERMINISM: BTreeMap iteration only (sorted owner keys); each owner's
//! `::before` slot is processed before its `::after` slot; float rounding via
//! `round4` before comparison/formatting.

use std::collections::{BTreeMap, BTreeSet};

use crate::config::{base_confidence, PSEUDO_DIFF_PROPERTIES, PSEUDO_SELECTOR_UNMATCHED_DEMOTION};
use crate::contract::{
    Anchors, CaptureBundle, CaptureDeterminism, Issue, IssueCategory, IssueSeverity, IssueType,
    Locator, PseudoElementEntry, PseudoOwnerTier, PseudoStyles, RunWarning, SemanticNode,
};
use crate::issue::compute_issue_id;
use crate::matching::MatchOutcome;
use crate::scoring::{compute_confidence, SeverityResolver};
use crate::style_diff;

// ---------------------------------------------------------------------------
// PseudoSlot
// ---------------------------------------------------------------------------

/// Which pseudo-element a captured entry's slot refers to (port-parity U10).
///
/// `pub`: shared with `explain.rs`'s `--selector "...::before"` /
/// `"...::after"` pseudo locator — never a second `::before`/`::after` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PseudoSlot {
    Before,
    After,
}

impl PseudoSlot {
    /// The `::before` / `::after` label. Used verbatim as the
    /// `compute_issue_id` `style_property` slot (alone for
    /// `pseudo_element_missing`, prefixed `"<label>.<property>"` for pseudo
    /// `style_changed`) and embedded in evidence's `"pseudo"` field.
    pub fn label(&self) -> &'static str {
        match self {
            PseudoSlot::Before => "::before",
            PseudoSlot::After => "::after",
        }
    }

    /// The captured style for this slot on one entry, if painted.
    pub fn style<'a>(&self, entry: &'a PseudoElementEntry) -> Option<&'a PseudoStyles> {
        match self {
            PseudoSlot::Before => entry.before.as_ref(),
            PseudoSlot::After => entry.after.as_ref(),
        }
    }
}

/// Fixed emission order within an owner: `::before` then `::after`.
const SLOTS: [PseudoSlot; 2] = [PseudoSlot::Before, PseudoSlot::After];

fn owner_tier_str(tier: &PseudoOwnerTier) -> &'static str {
    match tier {
        PseudoOwnerTier::Node => "node",
        PseudoOwnerTier::Ancestor => "ancestor",
        PseudoOwnerTier::Selector => "selector",
    }
}

fn round4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}

fn node_to_anchors(node: &SemanticNode) -> Anchors {
    Anchors {
        text: node.anchors.text.clone(),
        role: node.anchors.role.clone(),
        href: node.anchors.href.clone(),
        alt: node.anchors.alt.clone(),
        aria_label: node.anchors.aria_label.clone(),
        nearest_heading: node.anchors.nearest_heading.clone(),
        landmark: node.anchors.landmark.clone(),
        ordinal_in_landmark: node.anchors.ordinal_in_landmark,
    }
}

fn pseudo_bbox_to_locator(bbox: &[f64; 4]) -> [i32; 4] {
    [
        bbox[0].round() as i32,
        bbox[1].round() as i32,
        bbox[2].round() as i32,
        bbox[3].round() as i32,
    ]
}

/// Build a `{property: value}` string map from a captured `PseudoStyles`
/// (the curated field set; `content` always present, the rest best-effort).
///
/// `pub(crate)`: also reused by `explain.rs`'s pseudo `--selector` path —
/// never a second implementation.
pub(crate) fn pseudo_style_map(p: &PseudoStyles) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert("content".to_string(), p.content.clone());
    if let Some(v) = &p.position {
        m.insert("position".to_string(), v.clone());
    }
    if let Some(v) = &p.width {
        m.insert("width".to_string(), v.clone());
    }
    if let Some(v) = &p.height {
        m.insert("height".to_string(), v.clone());
    }
    if let Some(v) = &p.background_color {
        m.insert("background-color".to_string(), v.clone());
    }
    if let Some(v) = &p.background_image {
        m.insert("background-image".to_string(), v.clone());
    }
    if let Some(v) = &p.border {
        m.insert("border".to_string(), v.clone());
    }
    if let Some(v) = &p.border_radius {
        m.insert("border-radius".to_string(), v.clone());
    }
    if let Some(v) = &p.top {
        m.insert("top".to_string(), v.clone());
    }
    if let Some(v) = &p.right {
        m.insert("right".to_string(), v.clone());
    }
    if let Some(v) = &p.bottom {
        m.insert("bottom".to_string(), v.clone());
    }
    if let Some(v) = &p.left {
        m.insert("left".to_string(), v.clone());
    }
    if let Some(v) = &p.z_index {
        m.insert("z-index".to_string(), v.clone());
    }
    if let Some(v) = &p.display {
        m.insert("display".to_string(), v.clone());
    }
    if let Some(v) = &p.opacity {
        m.insert("opacity".to_string(), v.clone());
    }
    m
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Derive `pseudo_element_missing` / pseudo `style_changed` issues, plus any
/// `pseudo_budget_truncated` run warning, from both bundles' `pseudoElements`
/// maps.
///
/// Returns `(issues, warnings)` — empty/empty when either side lacks the
/// `pseudoElements` channel entirely (the `capability_mismatch` warning for
/// that condition is `orchestrate.rs`'s concern, computed independently from
/// the same two bundles; this function never emits it).
pub fn pseudo_issues(
    old_bundle: &CaptureBundle,
    new_bundle: &CaptureBundle,
    match_outcome: &MatchOutcome,
    viewport: &str,
    profile: &SeverityResolver,
    env_mismatch: bool,
) -> (Vec<Issue>, Vec<RunWarning>) {
    let mut issues: Vec<Issue> = Vec::new();
    let mut warnings: Vec<RunWarning> = Vec::new();

    let old_pseudo = match old_bundle.pseudo_elements.as_ref() {
        Some(m) if !m.is_empty() => m,
        _ => return (issues, warnings),
    };
    let new_pseudo = match new_bundle.pseudo_elements.as_ref() {
        Some(m) if !m.is_empty() => m,
        _ => return (issues, warnings),
    };

    let old_page_url = old_bundle.page.final_url.as_str();
    let new_page_url = new_bundle.page.final_url.as_str();

    // Tier "node" alignment: the matcher's Matched-band map (§ module doc).
    let old_to_new_id = style_diff::matched_old_to_new_id(old_bundle, new_bundle, match_outcome);

    // Tier "ancestor" alignment: the ancestor style channel's own pairing.
    let old_node_ids: BTreeSet<String> =
        old_bundle.page.nodes.iter().map(|n| n.id.clone()).collect();
    let new_node_ids: BTreeSet<String> =
        new_bundle.page.nodes.iter().map(|n| n.id.clone()).collect();
    let ancestor_pairs = style_diff::build_ancestor_pairing(
        old_bundle,
        new_bundle,
        &old_to_new_id,
        &old_node_ids,
        &new_node_ids,
        old_page_url,
        new_page_url,
    );
    let old_anc_to_new_anc: BTreeMap<String, String> = ancestor_pairs
        .into_iter()
        .map(|p| (p.old_id, p.new_id))
        .collect();

    let old_nodes_by_id: BTreeMap<&str, &SemanticNode> = old_bundle
        .page
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n))
        .collect();
    let new_nodes_by_id: BTreeMap<&str, &SemanticNode> = new_bundle
        .page
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n))
        .collect();

    // Truncation guard (design brief step 5): a pass-wide flag, independent
    // of whether this pass ends up emitting any tier-ancestor/selector
    // missing issues at all — asymmetric budget drops must not be silently
    // undetectable in the run's warnings.
    let truncated = old_bundle.pseudo_truncated.is_some() || new_bundle.pseudo_truncated.is_some();
    if truncated {
        warnings.push(pseudo_budget_truncated_warning(old_bundle, new_bundle));
    }

    // Deterministic iteration: BTreeMap key order (sorted owner keys); each
    // owner's ::before is processed before its ::after (SLOTS order, inside
    // the emit helpers below).
    for (old_key, old_entry) in old_pseudo {
        match &old_entry.owner_tier {
            PseudoOwnerTier::Node => {
                let old_node_id = match &old_entry.owner_node_id {
                    Some(id) => id.as_str(),
                    None => continue, // malformed entry — defensive; never emitted by capture
                };
                let old_node = match old_nodes_by_id.get(old_node_id) {
                    Some(n) => *n,
                    None => continue,
                };
                // Owner itself unmatched: "tier-node ... unaligned -> NOTHING"
                // (the node's own missing_* issue already covers it — no
                // double count).
                let new_node_id = match old_to_new_id.get(old_node_id) {
                    Some(id) => id.as_str(),
                    None => continue,
                };
                let new_node = new_nodes_by_id.get(new_node_id).copied();
                let aligned_new_entry = new_pseudo
                    .get_key_value(new_node_id)
                    .map(|(k, v)| (k.as_str(), v));

                let anchors = node_to_anchors(old_node);
                let css_selector_old = old_node.css_selector.clone();
                let css_selector_new = new_node.and_then(|n| n.css_selector.clone());
                let node_bbox_old = Some(old_node.bbox);
                let node_bbox_new = new_node.map(|n| n.bbox);
                let seq_index_old = Some(old_node.seq_index);
                let seq_index_new = new_node.map(|n| n.seq_index);

                emit_for_aligned_owner(EmitCtx {
                    issues: &mut issues,
                    old_entry,
                    aligned_new_entry,
                    tier: &PseudoOwnerTier::Node,
                    anchors,
                    css_selector_old,
                    css_selector_new,
                    node_bbox_old,
                    node_bbox_new,
                    seq_index_old,
                    seq_index_new,
                    viewport,
                    profile,
                    env_mismatch,
                    old_bundle,
                    new_bundle,
                    old_page_url,
                    new_page_url,
                    truncated,
                });
            }
            PseudoOwnerTier::Ancestor => {
                // Unaligned tier-ancestor owner -> NOTHING (same rationale as
                // tier-node: the ancestor's own style-channel silence, or its
                // absence from the pairing entirely, already reflects it).
                let new_anc_id = match old_anc_to_new_anc.get(old_key) {
                    Some(id) => id.as_str(),
                    None => continue,
                };
                let aligned_new_entry = new_pseudo
                    .get_key_value(new_anc_id)
                    .map(|(k, v)| (k.as_str(), v));
                let anchors = Anchors {
                    landmark: old_entry.landmark.clone(),
                    ..Anchors::null()
                };
                let css_selector_old = old_entry.owner_selector.clone();
                let css_selector_new =
                    aligned_new_entry.and_then(|(_, e)| e.owner_selector.clone());

                emit_for_aligned_owner(EmitCtx {
                    issues: &mut issues,
                    old_entry,
                    aligned_new_entry,
                    tier: &PseudoOwnerTier::Ancestor,
                    anchors,
                    css_selector_old,
                    css_selector_new,
                    node_bbox_old: None,
                    node_bbox_new: None,
                    seq_index_old: None,
                    seq_index_new: None,
                    viewport,
                    profile,
                    env_mismatch,
                    old_bundle,
                    new_bundle,
                    old_page_url,
                    new_page_url,
                    truncated,
                });
            }
            PseudoOwnerTier::Selector => {
                match new_pseudo.get_key_value(old_key.as_str()) {
                    Some((new_key, new_entry_ref)) => {
                        let anchors = Anchors {
                            landmark: old_entry.landmark.clone(),
                            ..Anchors::null()
                        };
                        let css_selector_old = old_entry.owner_selector.clone();
                        let css_selector_new = new_entry_ref.owner_selector.clone();

                        emit_for_aligned_owner(EmitCtx {
                            issues: &mut issues,
                            old_entry,
                            aligned_new_entry: Some((new_key.as_str(), new_entry_ref)),
                            tier: &PseudoOwnerTier::Selector,
                            anchors,
                            css_selector_old,
                            css_selector_new,
                            node_bbox_old: None,
                            node_bbox_new: None,
                            seq_index_old: None,
                            seq_index_new: None,
                            viewport,
                            profile,
                            env_mismatch,
                            old_bundle,
                            new_bundle,
                            old_page_url,
                            new_page_url,
                            truncated,
                        });
                    }
                    None => {
                        // Design brief U10 step 3 (the motivating
                        // "attribute-dropped port" case): no fallback tier
                        // exists for a decorative leaf, so silence here would
                        // blind the feature to its own acceptance case.
                        emit_unaligned_selector(
                            &mut issues,
                            old_entry,
                            viewport,
                            profile,
                            env_mismatch,
                            old_bundle,
                            new_bundle,
                            truncated,
                        );
                    }
                }
            }
        }
    }

    (issues, warnings)
}

// ---------------------------------------------------------------------------
// Aligned-owner emission (rules 1 & 2)
// ---------------------------------------------------------------------------

struct EmitCtx<'a> {
    issues: &'a mut Vec<Issue>,
    old_entry: &'a PseudoElementEntry,
    aligned_new_entry: Option<(&'a str, &'a PseudoElementEntry)>,
    tier: &'a PseudoOwnerTier,
    anchors: Anchors,
    css_selector_old: Option<String>,
    css_selector_new: Option<String>,
    node_bbox_old: Option<[i32; 4]>,
    node_bbox_new: Option<[i32; 4]>,
    seq_index_old: Option<u32>,
    seq_index_new: Option<u32>,
    viewport: &'a str,
    profile: &'a SeverityResolver,
    env_mismatch: bool,
    old_bundle: &'a CaptureBundle,
    new_bundle: &'a CaptureBundle,
    old_page_url: &'a str,
    new_page_url: &'a str,
    truncated: bool,
}

/// Emit issues for one aligned owner across both pseudo slots (`::before`
/// then `::after`): old painted + new absent -> `pseudo_element_missing`
/// (rule 1); old painted + new painted -> per-property `style_changed`
/// (rule 2); old absent -> nothing regardless of the new side (rules 3/4 —
/// new-only painted is the deferred `pseudo_element_added` case).
fn emit_for_aligned_owner(ctx: EmitCtx<'_>) {
    let EmitCtx {
        issues,
        old_entry,
        aligned_new_entry,
        tier,
        anchors,
        css_selector_old,
        css_selector_new,
        node_bbox_old,
        node_bbox_new,
        seq_index_old,
        seq_index_new,
        viewport,
        profile,
        env_mismatch,
        old_bundle,
        new_bundle,
        old_page_url,
        new_page_url,
        truncated,
    } = ctx;

    let tier_str = owner_tier_str(tier);
    let is_node_tier = *tier == PseudoOwnerTier::Node;

    for slot in SLOTS {
        let old_style = match slot.style(old_entry) {
            Some(s) => s,
            None => continue, // old not painted for this slot: nothing to do
        };
        let new_style = aligned_new_entry.and_then(|(_, e)| slot.style(e));

        let bbox_old = old_style
            .bbox
            .map(|b| pseudo_bbox_to_locator(&b))
            .or(if is_node_tier { node_bbox_old } else { None });

        match new_style {
            None => {
                // Rule 1: old painted, new absent.
                let bbox_new = if is_node_tier { node_bbox_new } else { None };

                let mut severity =
                    profile.severity_for(&IssueType::PseudoElementMissing, &IssueCategory::Style);
                if truncated && !is_node_tier {
                    severity = IssueSeverity::Info;
                }

                let confidence = compute_confidence(
                    base_confidence::PSEUDO_ELEMENT_MISSING,
                    env_mismatch,
                    &old_bundle.determinism,
                    &new_bundle.determinism,
                );

                let id = compute_issue_id(
                    &IssueType::PseudoElementMissing,
                    viewport,
                    &anchors,
                    Some(slot.label()),
                );

                let evidence = serde_json::json!({
                    "old": serde_json::to_value(old_style).unwrap_or(serde_json::Value::Null),
                    "new": serde_json::json!({}),
                    "pseudo": slot.label(),
                    "ownerTier": tier_str,
                });
                let remediation =
                    build_missing_remediation(slot.label(), &anchors, css_selector_old.as_deref());
                let message = build_missing_message(slot.label(), &anchors);

                issues.push(Issue {
                    id,
                    issue_type: IssueType::PseudoElementMissing,
                    category: IssueCategory::Style,
                    severity,
                    confidence,
                    viewport: viewport.to_string(),
                    locale: new_bundle.page.lang.clone(),
                    goal: Some("G4".to_string()),
                    message,
                    locator: Locator {
                        anchors: anchors.clone(),
                        css_selector_old: css_selector_old.clone(),
                        css_selector_new: css_selector_new.clone(),
                        bbox_old,
                        bbox_new,
                        seq_index_old,
                        seq_index_new,
                    },
                    evidence,
                    remediation: Some(remediation),
                });
            }
            Some(new_style) => {
                // Rule 2: both painted -> property-level diff.
                let bbox_new = new_style
                    .bbox
                    .map(|b| pseudo_bbox_to_locator(&b))
                    .or(if is_node_tier { node_bbox_new } else { None });

                let match_evidence = serde_json::json!({
                    "stage": "pseudo",
                    "ownerTier": tier_str,
                    "pseudo": slot.label(),
                });

                let mut prop_issues = diff_pseudo_props(
                    old_style,
                    new_style,
                    slot.label(),
                    &anchors,
                    css_selector_old.as_deref(),
                    css_selector_new.as_deref(),
                    bbox_old,
                    bbox_new,
                    seq_index_old,
                    seq_index_new,
                    &match_evidence,
                    viewport,
                    &new_bundle.page.lang,
                    profile,
                    env_mismatch,
                    &old_bundle.determinism,
                    &new_bundle.determinism,
                    old_page_url,
                    new_page_url,
                );
                issues.append(&mut prop_issues);
            }
        }
    }
}

/// Rule 3: an old-side tier-"selector" owner with no key-matched new owner —
/// the motivating "attribute-dropped port" defect. Emits `pseudo_element_missing`
/// at demoted confidence (`PSEUDO_SELECTOR_UNMATCHED_DEMOTION`), with the
/// alignment failure recorded in evidence.
#[allow(clippy::too_many_arguments)]
fn emit_unaligned_selector(
    issues: &mut Vec<Issue>,
    old_entry: &PseudoElementEntry,
    viewport: &str,
    profile: &SeverityResolver,
    env_mismatch: bool,
    old_bundle: &CaptureBundle,
    new_bundle: &CaptureBundle,
    truncated: bool,
) {
    let anchors = Anchors {
        landmark: old_entry.landmark.clone(),
        ..Anchors::null()
    };
    let css_selector_old = old_entry.owner_selector.clone();

    for slot in SLOTS {
        let old_style = match slot.style(old_entry) {
            Some(s) => s,
            None => continue,
        };
        let bbox_old = old_style.bbox.map(|b| pseudo_bbox_to_locator(&b));

        let mut severity =
            profile.severity_for(&IssueType::PseudoElementMissing, &IssueCategory::Style);
        // Tier "selector" is a tier-b/c owner: the truncation guard applies
        // regardless of alignment outcome.
        if truncated {
            severity = IssueSeverity::Info;
        }

        let base_conf = compute_confidence(
            base_confidence::PSEUDO_ELEMENT_MISSING,
            env_mismatch,
            &old_bundle.determinism,
            &new_bundle.determinism,
        );
        let confidence = round4(base_conf * PSEUDO_SELECTOR_UNMATCHED_DEMOTION);

        let id = compute_issue_id(
            &IssueType::PseudoElementMissing,
            viewport,
            &anchors,
            Some(slot.label()),
        );

        let evidence = serde_json::json!({
            "old": serde_json::to_value(old_style).unwrap_or(serde_json::Value::Null),
            "new": serde_json::json!({}),
            "pseudo": slot.label(),
            "ownerTier": "selector",
            "alignmentTier": "selector-unmatched",
        });
        let remediation =
            build_missing_remediation(slot.label(), &anchors, css_selector_old.as_deref());
        let message = build_missing_message(slot.label(), &anchors);

        issues.push(Issue {
            id,
            issue_type: IssueType::PseudoElementMissing,
            category: IssueCategory::Style,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_bundle.page.lang.clone(),
            goal: Some("G4".to_string()),
            message,
            locator: Locator {
                anchors: anchors.clone(),
                css_selector_old: css_selector_old.clone(),
                css_selector_new: None,
                bbox_old,
                bbox_new: None,
                seq_index_old: None,
                seq_index_new: None,
            },
            evidence,
            remediation: Some(remediation),
        });
    }
}

// ---------------------------------------------------------------------------
// Rule 2: property-level diff through the shared canonicalization ladder
// ---------------------------------------------------------------------------

/// Diff one aligned, both-painted pseudo pair's curated properties
/// (`PSEUDO_DIFF_PROPERTIES`, fixed order) through the SAME canonicalization
/// ladder `style_diff`'s leaf/ancestor channels use — reused via the
/// `pub(crate)` primitives there, never re-implemented.
///
/// Deliberate design choice: pseudo `background-image` gradient changes are
/// NOT routed through the leaf/ancestor channels' gradient classification
/// (`background_gradient_lost`/`background_gradient_changed`) — that logic
/// is tightly coupled to the bare (non-pseudo-prefixed) issue-id slot and
/// evidence shape it was written for. Reported here as plain `style_changed`
/// instead, per the design brief's explicit fallback ("otherwise plain
/// style_changed").
#[allow(clippy::too_many_arguments)]
fn diff_pseudo_props(
    old_style: &PseudoStyles,
    new_style: &PseudoStyles,
    slot_label: &str,
    old_anchors: &Anchors,
    css_selector_old: Option<&str>,
    css_selector_new: Option<&str>,
    bbox_old: Option<[i32; 4]>,
    bbox_new: Option<[i32; 4]>,
    seq_index_old: Option<u32>,
    seq_index_new: Option<u32>,
    match_evidence: &serde_json::Value,
    viewport: &str,
    locale: &Option<String>,
    profile: &SeverityResolver,
    env_mismatch: bool,
    old_det: &CaptureDeterminism,
    new_det: &CaptureDeterminism,
    old_page_url: &str,
    new_page_url: &str,
) -> Vec<Issue> {
    let old_map = pseudo_style_map(old_style);
    let new_map = pseudo_style_map(new_style);
    let mut issues = Vec::new();

    for prop in PSEUDO_DIFF_PROPERTIES {
        let old_v = match old_map.get(*prop) {
            Some(v) => v.as_str(),
            None => continue,
        };
        let new_v = match new_map.get(*prop) {
            Some(v) => v.as_str(),
            None => continue,
        };

        let old_norm = style_diff::normalize_value_with_page_url(prop, old_v, old_page_url);
        let new_norm = style_diff::normalize_value_with_page_url(prop, new_v, new_page_url);
        if old_norm == new_norm {
            continue; // Equal after normalization -> no issue
        }
        if style_diff::values_equal_c2(&old_norm, &new_norm) {
            continue; // C2: sub-pixel numeric epsilon
        }
        if style_diff::values_equal_c3(&old_norm, &new_norm) {
            continue; // C3: url() filename-tail equivalence
        }

        let old_canon = style_diff::canonicalize_for_compare(prop, &old_norm, None);
        let new_canon = style_diff::canonicalize_for_compare(prop, &new_norm, None);
        if old_canon == new_canon {
            continue; // C4: semantic equivalence (incl. the "content" quote-normalization arm)
        }
        if style_diff::values_equal_c2(&old_canon, &new_canon) {
            continue;
        }

        let prefixed_prop = format!("{}.{}", slot_label, prop);
        let base = base_confidence::STYLE_CHANGED;
        let confidence = compute_confidence(base, env_mismatch, old_det, new_det);
        let id = compute_issue_id(
            &IssueType::StyleChanged,
            viewport,
            old_anchors,
            Some(&prefixed_prop),
        );
        // The resolver's property lookup receives the BARE property name —
        // pseudo-prefixing only affects the id slot and remediation.property.
        let sev =
            profile.severity_for_property(&IssueType::StyleChanged, &IssueCategory::Style, prop);
        let evidence = style_diff::build_prop_evidence(prop, old_v, new_v, match_evidence, None);
        let remediation = style_diff::build_remediation(&prefixed_prop, old_v, new_v, old_anchors);
        let message = style_diff::build_message(&prefixed_prop, old_v, new_v, old_anchors);

        issues.push(Issue {
            id,
            issue_type: IssueType::StyleChanged,
            category: IssueCategory::Style,
            severity: sev,
            confidence,
            viewport: viewport.to_string(),
            locale: locale.clone(),
            goal: Some("G1".to_string()),
            message,
            locator: style_diff::build_locator(
                old_anchors.clone(),
                css_selector_old,
                css_selector_new,
                bbox_old,
                bbox_new,
                seq_index_old,
                seq_index_new,
            ),
            evidence,
            remediation: Some(remediation),
        });
    }

    issues
}

// ---------------------------------------------------------------------------
// Message / remediation / warning builders
// ---------------------------------------------------------------------------

fn missing_near(anchors: &Anchors) -> Option<&str> {
    anchors
        .nearest_heading
        .as_deref()
        .filter(|s| !s.is_empty())
        .or_else(|| anchors.landmark.as_deref().filter(|s| !s.is_empty()))
}

fn build_missing_remediation(
    slot_label: &str,
    anchors: &Anchors,
    css_selector_old: Option<&str>,
) -> serde_json::Value {
    let near = missing_near(anchors);
    let mut grep_targets: Vec<serde_json::Value> = Vec::new();
    if let Some(href) = anchors.href.as_deref() {
        if !href.is_empty() {
            grep_targets.push(serde_json::Value::String(format!("\"{}\"", href)));
        }
    }
    if let Some(text) = anchors.text.as_deref() {
        if !text.is_empty() {
            grep_targets.push(serde_json::Value::String(text.to_string()));
        }
    }
    if let Some(sel) = css_selector_old {
        if !sel.is_empty() {
            grep_targets.push(serde_json::Value::String(sel.to_string()));
        }
    }
    if grep_targets.is_empty() {
        if let Some(nh) = near {
            grep_targets.push(serde_json::Value::String(nh.to_string()));
        }
    }

    serde_json::json!({
        "action": "restore_pseudo_element",
        "findBy": {
            "grep": grep_targets,
            "near": near
        },
        "pseudo": slot_label,
        "note": "The tool does not name the source component. Use the grep targets to locate the owning selector/rule in source or CMS."
    })
}

fn build_missing_message(slot_label: &str, anchors: &Anchors) -> String {
    let near_part = match missing_near(anchors) {
        Some(n) => format!(" near \"{}\"", n),
        None => String::new(),
    };
    format!("{} pseudo-element is missing{}", slot_label, near_part)
}

fn pseudo_budget_truncated_warning(
    old_bundle: &CaptureBundle,
    new_bundle: &CaptureBundle,
) -> RunWarning {
    let dropped_old = old_bundle
        .pseudo_truncated
        .as_ref()
        .map(|t| t.dropped_count.to_string())
        .unwrap_or_else(|| "0".to_string());
    let dropped_new = new_bundle
        .pseudo_truncated
        .as_ref()
        .map(|t| t.dropped_count.to_string())
        .unwrap_or_else(|| "0".to_string());

    let mut context: BTreeMap<String, String> = BTreeMap::new();
    context.insert("droppedCountOld".to_string(), dropped_old);
    context.insert("droppedCountNew".to_string(), dropped_new);

    RunWarning {
        code: "pseudo_budget_truncated".to_string(),
        message: "the pseudo-element capture budget was exceeded on at least one side; \
                   tier-ancestor/selector pseudo_element_missing issues from this pass are \
                   demoted to info severity to avoid false positives from asymmetric drop order"
            .to_string(),
        context: Some(serde_json::to_value(context).unwrap_or(serde_json::Value::Null)),
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
        PseudoTruncated, Screenshots, StepStatus, StyleCandidates, ViewportConfig,
    };
    use crate::matching::{MatchBand, MatchOutcome, MatchStage, MatchedPair};
    use crate::scoring::{ParityProfile, SeverityResolver};

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

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

    fn make_env() -> Environment {
        Environment {
            os: "linux".to_string(),
            chromium_build: "1234".to_string(),
            playwright: "1.60.0".to_string(),
            dsf: 1.0,
        }
    }

    fn make_viewport_cfg() -> ViewportConfig {
        ViewportConfig {
            name: "desktop".to_string(),
            width: 1440,
            height: 900,
            dsf: 1.0,
        }
    }

    fn make_page(url: &str, nodes: Vec<SemanticNode>) -> PageModel {
        PageModel {
            url: url.to_string(),
            final_url: url.to_string(),
            redirect_chain: vec![],
            status_code: 200,
            title: None,
            meta_description: None,
            canonical: None,
            lang: Some("en".to_string()),
            page_height: 4000,
            nodes,
            landmarks: vec![],
            landmark_rects: None,
            network: NetworkInfo { requests: vec![] },
            console: vec![],
            a11y: A11yInfo { violations: vec![] },
            link_probes: vec![],
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_bundle_full(
        url: &str,
        nodes: Vec<SemanticNode>,
        pseudo_elements: Option<BTreeMap<String, PseudoElementEntry>>,
        pseudo_truncated: Option<PseudoTruncated>,
    ) -> CaptureBundle {
        CaptureBundle {
            schema_version: "1.1".to_string(),
            captured_at: "2026-01-01T00:00:00Z".to_string(),
            viewport: make_viewport_cfg(),
            environment: make_env(),
            determinism: make_det(),
            page: make_page(url, nodes),
            computed_styles: BTreeMap::new(),
            screenshots: Screenshots {
                full_page: "desktop/old.png".to_string(),
                viewport: "desktop/old-vp.png".to_string(),
            },
            style_candidates: StyleCandidates::default(),
            hit_tests: None,
            pseudo_elements,
            pseudo_truncated,
        }
    }

    fn make_bundle(
        url: &str,
        nodes: Vec<SemanticNode>,
        pseudo_elements: Option<BTreeMap<String, PseudoElementEntry>>,
    ) -> CaptureBundle {
        make_bundle_full(url, nodes, pseudo_elements, None)
    }

    fn make_node(id: &str, seq_index: u32, landmark: Option<&str>) -> SemanticNode {
        SemanticNode {
            id: id.to_string(),
            kind: "div".to_string(),
            role: None,
            text: None,
            acc_name: None,
            href: None,
            image_alt: None,
            bbox: [10, 20, 100, 30],
            seq_index,
            anchors: NodeAnchors {
                text: None,
                role: None,
                href: None,
                alt: None,
                aria_label: None,
                nearest_heading: None,
                landmark: landmark.map(str::to_string),
                ordinal_in_landmark: Some(1),
            },
            css_selector: Some(format!("#{}", id)),
            raw_href: None,
            src: None,
            natural_width: None,
            natural_height: None,
            loaded: None,
            heading_level: None,
            has_onclick: None,
        }
    }

    fn make_matched_pair(old_idx: usize, new_idx: usize) -> MatchedPair {
        let mut signals = BTreeMap::new();
        signals.insert("text".to_string(), 1.0_f64);
        MatchedPair {
            old_idx,
            new_idx,
            score: 1.0,
            stage: MatchStage::Identity,
            band: MatchBand::Matched,
            signals,
        }
    }

    fn make_outcome(pairs: Vec<MatchedPair>) -> MatchOutcome {
        MatchOutcome {
            pairs,
            missing_old: vec![],
            added_new: vec![],
        }
    }

    fn content_structure() -> SeverityResolver {
        SeverityResolver::from_profile(ParityProfile::ContentStructure)
    }

    fn strict_visual() -> SeverityResolver {
        SeverityResolver::from_profile(ParityProfile::StrictVisual)
    }

    fn tick_style(content: &str) -> PseudoStyles {
        PseudoStyles {
            content: content.to_string(),
            position: Some("absolute".to_string()),
            width: Some("10px".to_string()),
            height: Some("10px".to_string()),
            background_color: Some("rgb(255, 0, 0)".to_string()),
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

    fn selector_entry(
        selector: &str,
        landmark: Option<&str>,
        before: Option<PseudoStyles>,
        after: Option<PseudoStyles>,
    ) -> PseudoElementEntry {
        PseudoElementEntry {
            owner_tier: PseudoOwnerTier::Selector,
            owner_node_id: None,
            owner_selector: Some(selector.to_string()),
            landmark: landmark.map(str::to_string),
            before,
            after,
        }
    }

    fn node_entry(before: Option<PseudoStyles>, after: Option<PseudoStyles>) -> PseudoElementEntry {
        node_entry_for("n_cta", before, after)
    }

    fn node_entry_for(
        node_id: &str,
        before: Option<PseudoStyles>,
        after: Option<PseudoStyles>,
    ) -> PseudoElementEntry {
        PseudoElementEntry {
            owner_tier: PseudoOwnerTier::Node,
            owner_node_id: Some(node_id.to_string()),
            owner_selector: None,
            landmark: Some("main".to_string()),
            before,
            after,
        }
    }

    fn selector_key(landmark: &str, selector: &str) -> String {
        format!("{}::{}", landmark, selector)
    }

    // -----------------------------------------------------------------------
    // Motivating corner-tick: tier-selector aligned, old painted, new absent.
    // -----------------------------------------------------------------------

    #[test]
    fn test_motivating_corner_tick_selector_aligned_missing_on_new() {
        let key = selector_key("main", "[data-hr-corner-top]");
        let mut old_pseudo = BTreeMap::new();
        old_pseudo.insert(
            key.clone(),
            selector_entry(
                "[data-hr-corner-top]",
                Some("main"),
                Some(tick_style("\"\"")),
                None,
            ),
        );
        let mut new_pseudo = BTreeMap::new();
        // Owner still aligned (same key) but no longer paints ::before.
        new_pseudo.insert(
            key.clone(),
            selector_entry("[data-hr-corner-top]", Some("main"), None, None),
        );

        let old_bundle = make_bundle("http://old.example.com/", vec![], Some(old_pseudo));
        let new_bundle = make_bundle("http://new.example.com/", vec![], Some(new_pseudo));
        let outcome = make_outcome(vec![]);

        let (issues, warnings) = pseudo_issues(
            &old_bundle,
            &new_bundle,
            &outcome,
            "desktop",
            &content_structure(),
            false,
        );

        assert!(warnings.is_empty());
        assert_eq!(issues.len(), 1, "exactly one pseudo_element_missing issue");
        let issue = &issues[0];
        assert_eq!(issue.issue_type, IssueType::PseudoElementMissing);
        assert_eq!(issue.category, IssueCategory::Style);
        assert_eq!(issue.severity, IssueSeverity::Warning);
        assert_eq!(issue.goal, Some("G4".to_string()));
        assert_eq!(issue.confidence, base_confidence::PSEUDO_ELEMENT_MISSING);
        assert_eq!(issue.evidence["pseudo"], "::before");
        assert_eq!(issue.evidence["ownerTier"], "selector");
        assert_eq!(issue.evidence["new"], serde_json::json!({}));
        assert_eq!(issue.evidence["old"]["content"], "\"\"");
        assert!(issue.evidence.get("alignmentTier").is_none());
        assert_eq!(
            issue.locator.css_selector_old.as_deref(),
            Some("[data-hr-corner-top]")
        );
    }

    // -----------------------------------------------------------------------
    // Aligned ::after background-color change -> style_changed.
    // -----------------------------------------------------------------------

    #[test]
    fn test_aligned_after_background_color_change() {
        let key = selector_key("main", "[data-hr-corner-top]");
        let mut old_style = tick_style("\"\"");
        old_style.background_color = Some("rgb(255, 0, 0)".to_string());
        let mut new_style = tick_style("\"\"");
        new_style.background_color = Some("rgb(0, 0, 255)".to_string());

        let mut old_pseudo = BTreeMap::new();
        old_pseudo.insert(
            key.clone(),
            selector_entry("[data-hr-corner-top]", Some("main"), None, Some(old_style)),
        );
        let mut new_pseudo = BTreeMap::new();
        new_pseudo.insert(
            key.clone(),
            selector_entry("[data-hr-corner-top]", Some("main"), None, Some(new_style)),
        );

        let old_bundle = make_bundle("http://old.example.com/", vec![], Some(old_pseudo));
        let new_bundle = make_bundle("http://new.example.com/", vec![], Some(new_pseudo));
        let outcome = make_outcome(vec![]);

        let (issues, _warnings) = pseudo_issues(
            &old_bundle,
            &new_bundle,
            &outcome,
            "desktop",
            &content_structure(),
            false,
        );

        assert_eq!(issues.len(), 1);
        let issue = &issues[0];
        assert_eq!(issue.issue_type, IssueType::StyleChanged);
        assert_eq!(issue.category, IssueCategory::Style);
        assert_eq!(issue.goal, Some("G1".to_string()));
        let remediation = issue.remediation.as_ref().expect("remediation present");
        assert_eq!(remediation["property"], "::after.background-color");
        assert_eq!(remediation["from"], "rgb(0, 0, 255)");
        assert_eq!(remediation["to"], "rgb(255, 0, 0)");
    }

    // -----------------------------------------------------------------------
    // Owner missing entirely on new -> zero pseudo issues.
    // -----------------------------------------------------------------------

    #[test]
    fn test_owner_missing_entirely_on_new_zero_issues() {
        let old_node = make_node("n_cta", 0, Some("main"));
        let mut old_pseudo = BTreeMap::new();
        old_pseudo.insert(
            "n_cta".to_string(),
            node_entry(Some(tick_style("\"\"")), None),
        );

        let new_node = make_node("n_other", 0, Some("main"));
        let mut new_pseudo = BTreeMap::new();
        new_pseudo.insert(
            "n_other".to_string(),
            node_entry(Some(tick_style("\"\"")), None),
        );

        let old_bundle = make_bundle("http://old.example.com/", vec![old_node], Some(old_pseudo));
        let new_bundle = make_bundle("http://new.example.com/", vec![new_node], Some(new_pseudo));
        // No matched pair at all: old n_cta has no counterpart.
        let outcome = make_outcome(vec![]);

        let (issues, _warnings) = pseudo_issues(
            &old_bundle,
            &new_bundle,
            &outcome,
            "desktop",
            &content_structure(),
            false,
        );
        assert!(
            issues.is_empty(),
            "unaligned tier-node owner must emit zero pseudo issues (owner's own missing_* covers it)"
        );
    }

    // -----------------------------------------------------------------------
    // ::before both sides + ::after old-only, same owner -> exactly one issue.
    // -----------------------------------------------------------------------

    #[test]
    fn test_before_both_sides_after_old_only_exactly_one_issue() {
        let old_node = make_node("n_cta", 0, Some("main"));
        let new_node = make_node("n_cta", 0, Some("main"));

        let mut old_pseudo = BTreeMap::new();
        old_pseudo.insert(
            "n_cta".to_string(),
            node_entry(Some(tick_style("\"\"")), Some(tick_style("\"x\""))),
        );
        let mut new_pseudo = BTreeMap::new();
        // ::before painted identically on new (no diff); ::after absent on new.
        new_pseudo.insert(
            "n_cta".to_string(),
            node_entry(Some(tick_style("\"\"")), None),
        );

        let old_bundle = make_bundle("http://old.example.com/", vec![old_node], Some(old_pseudo));
        let new_bundle = make_bundle("http://new.example.com/", vec![new_node], Some(new_pseudo));
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let (issues, _warnings) = pseudo_issues(
            &old_bundle,
            &new_bundle,
            &outcome,
            "desktop",
            &content_structure(),
            false,
        );

        assert_eq!(issues.len(), 1, "only ::after's absence should fire");
        assert_eq!(issues[0].evidence["pseudo"], "::after");

        // Distinct id from any ::before-slot id for the SAME owner/anchors.
        let before_id = compute_issue_id(
            &IssueType::PseudoElementMissing,
            "desktop",
            &issues[0].locator.anchors,
            Some("::before"),
        );
        assert_ne!(issues[0].id, before_id);
    }

    // -----------------------------------------------------------------------
    // New-only painted pseudo -> zero issues (deferred pseudo_element_added).
    // -----------------------------------------------------------------------

    #[test]
    fn test_new_only_painted_pseudo_zero_issues() {
        let old_node = make_node("n_cta", 0, Some("main"));
        let new_node = make_node("n_cta", 0, Some("main"));
        // A second, unrelated matched pair with an IDENTICAL aligned pseudo on
        // both sides — keeps the old-side `pseudoElements` channel non-empty
        // (so `pseudo_issues` doesn't short-circuit on "channel absent")
        // without itself contributing any issue.
        let old_other = make_node("n_other", 1, Some("main"));
        let new_other = make_node("n_other", 1, Some("main"));

        let mut old_pseudo = BTreeMap::new();
        old_pseudo.insert(
            "n_other".to_string(),
            node_entry_for("n_other", Some(tick_style("\"\"")), None),
        );
        let mut new_pseudo = BTreeMap::new();
        new_pseudo.insert(
            "n_other".to_string(),
            node_entry_for("n_other", Some(tick_style("\"\"")), None),
        );
        // n_cta paints ::before on new only — never present in old_pseudo at all.
        new_pseudo.insert(
            "n_cta".to_string(),
            node_entry(Some(tick_style("\"\"")), None),
        );

        let old_bundle = make_bundle(
            "http://old.example.com/",
            vec![old_node, old_other],
            Some(old_pseudo),
        );
        let new_bundle = make_bundle(
            "http://new.example.com/",
            vec![new_node, new_other],
            Some(new_pseudo),
        );
        let outcome = make_outcome(vec![make_matched_pair(0, 0), make_matched_pair(1, 1)]);

        let (issues, _warnings) = pseudo_issues(
            &old_bundle,
            &new_bundle,
            &outcome,
            "desktop",
            &content_structure(),
            false,
        );
        assert!(
            issues.is_empty(),
            "new-only painted pseudo on n_cta must never surface (never visited: not in old_pseudo)"
        );
    }

    // -----------------------------------------------------------------------
    // Attribute-dropped tier-selector: demoted-confidence pseudo_element_missing.
    // -----------------------------------------------------------------------

    #[test]
    fn test_attribute_dropped_tier_selector_demoted_confidence() {
        let mut old_pseudo = BTreeMap::new();
        old_pseudo.insert(
            selector_key("main", "[data-hr-corner-top]"),
            selector_entry(
                "[data-hr-corner-top]",
                Some("main"),
                Some(tick_style("\"\"")),
                None,
            ),
        );
        // New side: the attribute (and therefore the owner key) is gone
        // entirely — no key match at all.
        let new_pseudo: BTreeMap<String, PseudoElementEntry> = BTreeMap::new();
        let mut new_pseudo_nonempty = new_pseudo;
        new_pseudo_nonempty.insert(
            selector_key("main", ".unrelated"),
            selector_entry(".unrelated", Some("main"), Some(tick_style("\"\"")), None),
        );

        let old_bundle = make_bundle("http://old.example.com/", vec![], Some(old_pseudo));
        let new_bundle = make_bundle("http://new.example.com/", vec![], Some(new_pseudo_nonempty));
        let outcome = make_outcome(vec![]);

        let (issues, _warnings) = pseudo_issues(
            &old_bundle,
            &new_bundle,
            &outcome,
            "desktop",
            &content_structure(),
            false,
        );

        assert_eq!(issues.len(), 1);
        let issue = &issues[0];
        assert_eq!(issue.issue_type, IssueType::PseudoElementMissing);
        assert_eq!(issue.evidence["alignmentTier"], "selector-unmatched");
        let expected =
            round4(base_confidence::PSEUDO_ELEMENT_MISSING * PSEUDO_SELECTOR_UNMATCHED_DEMOTION);
        assert_eq!(issue.confidence, expected);
        assert!(issue.confidence < base_confidence::PSEUDO_ELEMENT_MISSING);
    }

    // -----------------------------------------------------------------------
    // Asymmetric truncation -> tier-b/c demoted to Info + warning present.
    // -----------------------------------------------------------------------

    #[test]
    fn test_asymmetric_truncation_demotes_and_warns() {
        let key = selector_key("main", "[data-hr-corner-top]");
        let mut old_pseudo = BTreeMap::new();
        old_pseudo.insert(
            key.clone(),
            selector_entry(
                "[data-hr-corner-top]",
                Some("main"),
                Some(tick_style("\"\"")),
                None,
            ),
        );
        let mut new_pseudo = BTreeMap::new();
        new_pseudo.insert(
            key.clone(),
            selector_entry("[data-hr-corner-top]", Some("main"), None, None),
        );

        let old_bundle = make_bundle("http://old.example.com/", vec![], Some(old_pseudo));
        // New side recorded a truncation (asymmetric: old did not).
        let new_bundle = make_bundle_full(
            "http://new.example.com/",
            vec![],
            Some(new_pseudo),
            Some(PseudoTruncated { dropped_count: 12 }),
        );
        let outcome = make_outcome(vec![]);

        let (issues, warnings) = pseudo_issues(
            &old_bundle,
            &new_bundle,
            &outcome,
            "desktop",
            &content_structure(),
            false,
        );

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, IssueSeverity::Info);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "pseudo_budget_truncated");
        let ctx = warnings[0].context.as_ref().expect("context present");
        assert_eq!(ctx["droppedCountOld"], "0");
        assert_eq!(ctx["droppedCountNew"], "12");
    }

    /// Tier-node emissions keep their severity under truncation.
    #[test]
    fn test_truncation_does_not_demote_tier_node() {
        let old_node = make_node("n_cta", 0, Some("main"));
        let new_node = make_node("n_cta", 0, Some("main"));
        let mut old_pseudo = BTreeMap::new();
        old_pseudo.insert(
            "n_cta".to_string(),
            node_entry(Some(tick_style("\"\"")), None),
        );
        let mut new_pseudo = BTreeMap::new();
        new_pseudo.insert("n_cta".to_string(), node_entry(None, None));

        let old_bundle = make_bundle_full(
            "http://old.example.com/",
            vec![old_node],
            Some(old_pseudo),
            Some(PseudoTruncated { dropped_count: 5 }),
        );
        let new_bundle = make_bundle("http://new.example.com/", vec![new_node], Some(new_pseudo));
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let (issues, warnings) = pseudo_issues(
            &old_bundle,
            &new_bundle,
            &outcome,
            "desktop",
            &content_structure(),
            false,
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(
            issues[0].severity,
            IssueSeverity::Warning,
            "tier-node emissions keep their profile severity under truncation"
        );
        assert_eq!(warnings.len(), 1);
    }

    // -----------------------------------------------------------------------
    // strict-visual profile -> the same missing pseudo emits at error.
    // -----------------------------------------------------------------------

    #[test]
    fn test_strict_visual_profile_missing_pseudo_at_error() {
        let old_node = make_node("n_cta", 0, Some("main"));
        let new_node = make_node("n_cta", 0, Some("main"));
        let mut old_pseudo = BTreeMap::new();
        old_pseudo.insert(
            "n_cta".to_string(),
            node_entry(Some(tick_style("\"\"")), None),
        );
        let mut new_pseudo = BTreeMap::new();
        new_pseudo.insert("n_cta".to_string(), node_entry(None, None));

        let old_bundle = make_bundle("http://old.example.com/", vec![old_node], Some(old_pseudo));
        let new_bundle = make_bundle("http://new.example.com/", vec![new_node], Some(new_pseudo));
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let (issues, _warnings) = pseudo_issues(
            &old_bundle,
            &new_bundle,
            &outcome,
            "desktop",
            &strict_visual(),
            false,
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, IssueSeverity::Error);
    }

    // -----------------------------------------------------------------------
    // Old bundle without pseudo channel -> zero pseudo issues, no crash.
    // -----------------------------------------------------------------------

    #[test]
    fn test_old_bundle_without_pseudo_channel_zero_issues() {
        let old_node = make_node("n_cta", 0, Some("main"));
        let new_node = make_node("n_cta", 0, Some("main"));
        let mut new_pseudo = BTreeMap::new();
        new_pseudo.insert(
            "n_cta".to_string(),
            node_entry(Some(tick_style("\"\"")), None),
        );

        let old_bundle = make_bundle("http://old.example.com/", vec![old_node], None);
        let new_bundle = make_bundle("http://new.example.com/", vec![new_node], Some(new_pseudo));
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let (issues, warnings) = pseudo_issues(
            &old_bundle,
            &new_bundle,
            &outcome,
            "desktop",
            &content_structure(),
            false,
        );
        assert!(issues.is_empty());
        assert!(
            warnings.is_empty(),
            "capability_mismatch is orchestrate.rs's concern, not pseudo_diff's"
        );
    }

    #[test]
    fn test_both_bundles_without_pseudo_channel_zero_issues() {
        let old_node = make_node("n_cta", 0, Some("main"));
        let new_node = make_node("n_cta", 0, Some("main"));
        let old_bundle = make_bundle("http://old.example.com/", vec![old_node], None);
        let new_bundle = make_bundle("http://new.example.com/", vec![new_node], None);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let (issues, warnings) = pseudo_issues(
            &old_bundle,
            &new_bundle,
            &outcome,
            "desktop",
            &content_structure(),
            false,
        );
        assert!(issues.is_empty());
        assert!(warnings.is_empty());
    }

    // -----------------------------------------------------------------------
    // Per-property severity map applies to the BARE property name.
    // -----------------------------------------------------------------------

    #[test]
    fn test_severity_map_applies_to_bare_property_name() {
        let mut old_style = tick_style("\"\"");
        old_style.background_color = Some("rgb(255, 0, 0)".to_string());
        let mut new_style = tick_style("\"\"");
        new_style.background_color = Some("rgb(0, 0, 255)".to_string());

        let key = selector_key("main", "[data-hr-corner-top]");
        let mut old_pseudo = BTreeMap::new();
        old_pseudo.insert(
            key.clone(),
            selector_entry("[data-hr-corner-top]", Some("main"), None, Some(old_style)),
        );
        let mut new_pseudo = BTreeMap::new();
        new_pseudo.insert(
            key.clone(),
            selector_entry("[data-hr-corner-top]", Some("main"), None, Some(new_style)),
        );

        let old_bundle = make_bundle("http://old.example.com/", vec![], Some(old_pseudo));
        let new_bundle = make_bundle("http://new.example.com/", vec![], Some(new_pseudo));
        let outcome = make_outcome(vec![]);

        // Build a resolver with a user override on the BARE "background-color"
        // property; if the resolver were (incorrectly) queried with the
        // pseudo-prefixed string, this override would never apply and the
        // severity would fall through to the profile default (Warning).
        let mut user_props = BTreeMap::new();
        user_props.insert("background-color".to_string(), IssueSeverity::Critical);
        let (resolver, denied) = SeverityResolver::with_user_map(
            ParityProfile::ContentStructure,
            BTreeMap::new(),
            user_props,
        );
        assert!(denied.is_empty());

        let (issues, _warnings) = pseudo_issues(
            &old_bundle,
            &new_bundle,
            &outcome,
            "desktop",
            &resolver,
            false,
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(
            issues[0].severity,
            IssueSeverity::Critical,
            "bare-property severity map must apply to pseudo style_changed issues"
        );
    }

    // -----------------------------------------------------------------------
    // Id stability across re-analysis.
    // -----------------------------------------------------------------------

    #[test]
    fn test_id_stability_across_reanalysis() {
        let key = selector_key("main", "[data-hr-corner-top]");
        let mut old_pseudo = BTreeMap::new();
        old_pseudo.insert(
            key.clone(),
            selector_entry(
                "[data-hr-corner-top]",
                Some("main"),
                Some(tick_style("\"\"")),
                None,
            ),
        );
        let mut new_pseudo = BTreeMap::new();
        new_pseudo.insert(
            key.clone(),
            selector_entry("[data-hr-corner-top]", Some("main"), None, None),
        );

        let old_bundle = make_bundle("http://old.example.com/", vec![], Some(old_pseudo));
        let new_bundle = make_bundle("http://new.example.com/", vec![], Some(new_pseudo));
        let outcome = make_outcome(vec![]);

        let (issues1, _) = pseudo_issues(
            &old_bundle,
            &new_bundle,
            &outcome,
            "desktop",
            &content_structure(),
            false,
        );
        let (issues2, _) = pseudo_issues(
            &old_bundle,
            &new_bundle,
            &outcome,
            "desktop",
            &content_structure(),
            false,
        );
        assert_eq!(issues1.len(), 1);
        assert_eq!(issues2.len(), 1);
        assert_eq!(issues1[0].id, issues2[0].id);
    }

    // -----------------------------------------------------------------------
    // Deterministic output order across map insertion orders.
    // -----------------------------------------------------------------------

    #[test]
    fn test_deterministic_order_across_insertion_orders() {
        let key_a = selector_key("main", "[data-hr-corner-top]");
        let key_b = selector_key("main", "[data-hr-corner-bottom]");

        let mut old_pseudo_1 = BTreeMap::new();
        old_pseudo_1.insert(
            key_a.clone(),
            selector_entry(
                "[data-hr-corner-top]",
                Some("main"),
                Some(tick_style("\"\"")),
                None,
            ),
        );
        old_pseudo_1.insert(
            key_b.clone(),
            selector_entry(
                "[data-hr-corner-bottom]",
                Some("main"),
                Some(tick_style("\"\"")),
                None,
            ),
        );

        // Same content, reversed insertion order — BTreeMap normalizes this,
        // but assert explicitly that output order is insertion-independent.
        let mut old_pseudo_2 = BTreeMap::new();
        old_pseudo_2.insert(
            key_b.clone(),
            selector_entry(
                "[data-hr-corner-bottom]",
                Some("main"),
                Some(tick_style("\"\"")),
                None,
            ),
        );
        old_pseudo_2.insert(
            key_a.clone(),
            selector_entry(
                "[data-hr-corner-top]",
                Some("main"),
                Some(tick_style("\"\"")),
                None,
            ),
        );

        let new_pseudo: BTreeMap<String, PseudoElementEntry> = BTreeMap::new();
        let mut new_pseudo_nonempty = new_pseudo;
        new_pseudo_nonempty.insert(
            selector_key("main", ".unrelated"),
            selector_entry(".unrelated", Some("main"), Some(tick_style("\"\"")), None),
        );

        let old_bundle_1 = make_bundle("http://old.example.com/", vec![], Some(old_pseudo_1));
        let old_bundle_2 = make_bundle("http://old.example.com/", vec![], Some(old_pseudo_2));
        let new_bundle = make_bundle("http://new.example.com/", vec![], Some(new_pseudo_nonempty));
        let outcome = make_outcome(vec![]);

        let (issues1, _) = pseudo_issues(
            &old_bundle_1,
            &new_bundle,
            &outcome,
            "desktop",
            &content_structure(),
            false,
        );
        let (issues2, _) = pseudo_issues(
            &old_bundle_2,
            &new_bundle,
            &outcome,
            "desktop",
            &content_structure(),
            false,
        );

        let ids1: Vec<&str> = issues1.iter().map(|i| i.id.as_str()).collect();
        let ids2: Vec<&str> = issues2.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids1, ids2);
        assert_eq!(issues1.len(), 2);
    }
}
