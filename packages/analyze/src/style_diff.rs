//! Computed-style diff module (M4 §3).
//!
//! Entry point: `style_issues(old_bundle, new_bundle, match_outcome, viewport, profile, env_mismatch)`
//!
//! Channels:
//!   1. Leaf channel: Matched pairs with computed styles on both sides.
//!   2. Ancestor channel: Paired ancestors via descendant-set grouping.
//!
//! DETERMINISM: BTreeMap everywhere; greedy assignment is total-ordered;
//! float sims are rounded to 4 decimals before ordering; no HashMap.

use std::collections::BTreeMap;

use crate::config::{
    base_confidence, ANCESTOR_MIN_SIMILARITY, MIN_PAIRING_SCORE_FOR_STYLE, STYLE_DIFF_PROPERTIES,
};
use crate::contract::{
    AncestorDescriptor, Anchors, CaptureBundle, Issue, IssueCategory, IssueType, Locator,
};
use crate::issue::compute_issue_id;
use crate::matching::{norm_href, MatchBand, MatchOutcome};
use crate::scoring::{compute_confidence, SeverityResolver};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Derive style issues from matcher output and computed styles.
pub fn style_issues(
    old_bundle: &CaptureBundle,
    new_bundle: &CaptureBundle,
    match_outcome: &MatchOutcome,
    viewport: &str,
    profile: &SeverityResolver,
    env_mismatch: bool,
) -> Vec<Issue> {
    let mut issues: Vec<Issue> = Vec::new();

    // Capture each side's final_url for url() normalization via norm_href.
    let old_page_url = old_bundle.page.final_url.as_str();
    let new_page_url = new_bundle.page.final_url.as_str();

    // Collect the set of all SemanticNode ids (leaf channel owns these for ancestor skip).
    let old_node_ids: std::collections::BTreeSet<String> =
        old_bundle.page.nodes.iter().map(|n| n.id.clone()).collect();
    let new_node_ids: std::collections::BTreeSet<String> =
        new_bundle.page.nodes.iter().map(|n| n.id.clone()).collect();

    // -----------------------------------------------------------------------
    // 1. Leaf channel
    // -----------------------------------------------------------------------
    // Process ALL pairs (Matched and Uncertain).
    // Uncertain-pairing gate (bug p1-04): pairs with band != Matched OR
    // score < MIN_PAIRING_SCORE_FOR_STYLE emit issues at Info severity with
    // evidence.match.uncertainPairing = true, excluded from style score.
    // Sort by (old_seq_index, old_id) for deterministic issue ordering.
    let mut all_pairs: Vec<&crate::matching::MatchedPair> = match_outcome.pairs.iter().collect();
    all_pairs.sort_by(|a, b| {
        let oa = &old_bundle.page.nodes[a.old_idx];
        let ob = &old_bundle.page.nodes[b.old_idx];
        oa.seq_index
            .cmp(&ob.seq_index)
            .then_with(|| oa.id.cmp(&ob.id))
    });

    // Build old_id -> new_id map for ancestor channel (only Matched pairs)
    let mut old_to_new_id: BTreeMap<String, String> = BTreeMap::new();

    for pair in &all_pairs {
        let old_node = &old_bundle.page.nodes[pair.old_idx];
        let new_node = &new_bundle.page.nodes[pair.new_idx];

        // Only use Matched pairs for the ancestor channel map.
        if pair.band == MatchBand::Matched {
            old_to_new_id.insert(old_node.id.clone(), new_node.id.clone());
        }

        let old_styles = match old_bundle.computed_styles.get(&old_node.id) {
            Some(s) => s,
            None => continue,
        };
        let new_styles = match new_bundle.computed_styles.get(&new_node.id) {
            Some(s) => s,
            None => continue,
        };

        // Determine whether this is an uncertain pairing.
        let is_uncertain =
            pair.band != MatchBand::Matched || pair.score < MIN_PAIRING_SCORE_FOR_STYLE;

        let match_evidence = pair_match_evidence(pair, old_bundle, new_bundle, is_uncertain);

        let base = base_confidence::STYLE_CHANGED;
        let confidence = compute_confidence(
            base,
            env_mismatch,
            &old_bundle.determinism,
            &new_bundle.determinism,
        );

        let old_anchors = node_to_anchors(old_node);

        let mut leaf_issues = diff_styles(
            old_styles,
            new_styles,
            &old_anchors,
            old_node.css_selector.as_deref(),
            new_node.css_selector.as_deref(),
            Some(old_node.bbox),
            Some(new_node.bbox),
            Some(old_node.seq_index),
            Some(new_node.seq_index),
            &match_evidence,
            confidence,
            viewport,
            &new_bundle.page.lang,
            profile,
            env_mismatch,
            &old_bundle.determinism,
            &new_bundle.determinism,
            old_page_url,
            new_page_url,
            is_uncertain,
        );
        issues.append(&mut leaf_issues);
    }

    // -----------------------------------------------------------------------
    // 2. Ancestor channel
    // -----------------------------------------------------------------------
    let mut ancestor_issues = ancestor_channel_issues(
        old_bundle,
        new_bundle,
        &old_to_new_id,
        &old_node_ids,
        &new_node_ids,
        viewport,
        profile,
        env_mismatch,
        old_page_url,
        new_page_url,
    );
    issues.append(&mut ancestor_issues);

    issues
}

// ---------------------------------------------------------------------------
// Ancestor channel (§3.2)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn ancestor_channel_issues(
    old_bundle: &CaptureBundle,
    new_bundle: &CaptureBundle,
    old_to_new_id: &BTreeMap<String, String>,
    old_node_ids: &std::collections::BTreeSet<String>,
    new_node_ids: &std::collections::BTreeSet<String>,
    viewport: &str,
    profile: &SeverityResolver,
    env_mismatch: bool,
    old_page_url: &str,
    new_page_url: &str,
) -> Vec<Issue> {
    // Build reverse map: new_id → old_id
    let mut new_to_old_id: BTreeMap<String, String> = BTreeMap::new();
    for (old_id, new_id) in old_to_new_id {
        new_to_old_id.insert(new_id.clone(), old_id.clone());
    }

    // old ancestor id → descriptor (quick lookup)
    let old_ancestors: BTreeMap<String, &AncestorDescriptor> = old_bundle
        .style_candidates
        .ancestors
        .iter()
        .map(|a| (a.id.clone(), a))
        .collect();
    let new_ancestors: BTreeMap<String, &AncestorDescriptor> = new_bundle
        .style_candidates
        .ancestors
        .iter()
        .map(|a| (a.id.clone(), a))
        .collect();

    // For each old ancestor: compute D(a) = sorted set of matched-new-node ids
    // whose OLD chain contains `a`, projected to new ids.
    // Skip any chain entry whose id is a SemanticNode id.
    let mut old_anc_to_desc: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (node_id, chain) in &old_bundle.style_candidates.chains {
        // Only consider nodes that are matched
        let new_node_id = match old_to_new_id.get(node_id) {
            Some(id) => id.clone(),
            None => continue,
        };
        for anc_id in chain {
            // Skip if this ancestor id is actually a SemanticNode id (leaf channel owns it)
            if old_node_ids.contains(anc_id) {
                continue;
            }
            old_anc_to_desc
                .entry(anc_id.clone())
                .or_default()
                .push(new_node_id.clone());
        }
    }
    // Sort each descendant set for stable key
    for v in old_anc_to_desc.values_mut() {
        v.sort();
        v.dedup();
    }

    // For each new ancestor: compute D(b) = sorted set of matched-new-node ids
    // whose NEW chain contains `b`.
    let mut new_anc_to_desc: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (node_id, chain) in &new_bundle.style_candidates.chains {
        // Only matched new nodes
        if !new_to_old_id.contains_key(node_id) {
            continue;
        }
        for anc_id in chain {
            // Skip if this ancestor id is actually a SemanticNode id
            if new_node_ids.contains(anc_id) {
                continue;
            }
            new_anc_to_desc
                .entry(anc_id.clone())
                .or_default()
                .push(node_id.clone());
        }
    }
    for v in new_anc_to_desc.values_mut() {
        v.sort();
        v.dedup();
    }

    // Group old ancestors by D(a) key (ids joined by \x1f)
    let mut old_by_desc_key: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (anc_id, desc_set) in &old_anc_to_desc {
        if desc_set.is_empty() {
            continue;
        }
        let key = desc_set.join("\x1f");
        old_by_desc_key.entry(key).or_default().push(anc_id.clone());
    }
    for v in old_by_desc_key.values_mut() {
        v.sort();
    }

    // Group new ancestors by D(b) key
    let mut new_by_desc_key: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (anc_id, desc_set) in &new_anc_to_desc {
        if desc_set.is_empty() {
            continue;
        }
        let key = desc_set.join("\x1f");
        new_by_desc_key.entry(key).or_default().push(anc_id.clone());
    }
    for v in new_by_desc_key.values_mut() {
        v.sort();
    }

    let mut issues: Vec<Issue> = Vec::new();

    // For each key present on both sides, run style-similarity assignment
    for (desc_key, old_group) in &old_by_desc_key {
        let new_group = match new_by_desc_key.get(desc_key) {
            Some(g) => g,
            None => continue,
        };

        // Compute similarity matrix and pair greedily
        // Collect candidates: (sim, old_id, new_id)
        let mut candidates: Vec<(f64, String, String)> = Vec::new();
        for old_id in old_group {
            let old_desc = match old_ancestors.get(old_id) {
                Some(d) => d,
                None => continue,
            };
            let old_styles = match old_bundle.computed_styles.get(old_id.as_str()) {
                Some(s) => s,
                None => continue,
            };
            for new_id in new_group {
                let new_desc = match new_ancestors.get(new_id) {
                    Some(d) => d,
                    None => continue,
                };
                let new_styles = match new_bundle.computed_styles.get(new_id.as_str()) {
                    Some(s) => s,
                    None => continue,
                };
                let (sim, _base, _bonus) = style_similarity(
                    old_styles,
                    new_styles,
                    old_desc.tag.as_str(),
                    new_desc.tag.as_str(),
                    old_page_url,
                    new_page_url,
                );
                candidates.push((sim, old_id.clone(), new_id.clone()));
            }
        }

        // Sort: (sim desc, old_id asc, new_id asc) — sim rounded to 4 decimals
        candidates.sort_by(|a, b| {
            let sa = round4(a.0);
            let sb = round4(b.0);
            sb.partial_cmp(&sa)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });

        // Greedy pairing — each id used once
        let mut used_old: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut used_new: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

        for (sim, old_id, new_id) in &candidates {
            if *sim < ANCESTOR_MIN_SIMILARITY {
                break; // Sorted descending, so all remaining are below floor
            }
            if used_old.contains(old_id) || used_new.contains(new_id) {
                continue;
            }
            used_old.insert(old_id.clone());
            used_new.insert(new_id.clone());

            let old_desc = match old_ancestors.get(old_id.as_str()) {
                Some(d) => d,
                None => continue,
            };
            let new_desc = match new_ancestors.get(new_id.as_str()) {
                Some(d) => d,
                None => continue,
            };
            let old_styles = match old_bundle.computed_styles.get(old_id.as_str()) {
                Some(s) => s,
                None => continue,
            };
            let new_styles = match new_bundle.computed_styles.get(new_id.as_str()) {
                Some(s) => s,
                None => continue,
            };

            // Recompute base/bonus for evidence signals (sim stored in candidates is uncapped).
            let (_, sim_base, sim_bonus) = style_similarity(
                old_styles,
                new_styles,
                old_desc.tag.as_str(),
                new_desc.tag.as_str(),
                old_page_url,
                new_page_url,
            );
            let match_evidence = serde_json::json!({
                "stage": "ancestor",
                "score": round4(sim.min(1.0)),
                "signals": {
                    "descendantSet": 1.0,
                    "styleSim": round4(sim_base),
                    "tagBonus": sim_bonus
                }
            });

            let base = base_confidence::STYLE_CHANGED;
            let confidence = compute_confidence(
                base,
                env_mismatch,
                &old_bundle.determinism,
                &new_bundle.determinism,
            );

            let old_anchors = old_desc.anchors.clone();

            let mut anc_issues = diff_styles(
                old_styles,
                new_styles,
                &old_anchors,
                old_desc.css_selector.as_deref(),
                new_desc.css_selector.as_deref(),
                Some(old_desc.bbox),
                Some(new_desc.bbox),
                None, // ancestors have no seqIndex
                None,
                &match_evidence,
                confidence,
                viewport,
                &new_bundle.page.lang,
                profile,
                env_mismatch,
                &old_bundle.determinism,
                &new_bundle.determinism,
                old_page_url,
                new_page_url,
                false, // ancestor channel: similarity >= ANCESTOR_MIN_SIMILARITY, always confident
            );
            issues.append(&mut anc_issues);
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// Style similarity (§3.2)
// ---------------------------------------------------------------------------

/// Returns `(uncapped_sim, base_ratio, tag_bonus)`.
///
/// `uncapped_sim = base_ratio + tag_bonus` — intentionally NOT capped at 1.0 so that a
/// perfect pair (base 1.0 + 0.05 = 1.05) sorts strictly above a one-prop-differing pair
/// (e.g. 25/26 + 0.05 ≈ 1.0115) and they are never tied.  Callers must use
/// `min(uncapped_sim, 1.0)` when the value is presented in output.
fn style_similarity(
    old_styles: &BTreeMap<String, String>,
    new_styles: &BTreeMap<String, String>,
    old_tag: &str,
    new_tag: &str,
    old_page_url: &str,
    new_page_url: &str,
) -> (f64, f64, f64) {
    let mut equal_count = 0u32;
    let mut present_count = 0u32;

    for prop in STYLE_DIFF_PROPERTIES {
        let old_v = old_styles.get(*prop);
        let new_v = new_styles.get(*prop);
        // absent on one side — not counted
        if let (Some(o), Some(n)) = (old_v, new_v) {
            present_count += 1;
            let old_norm = normalize_value_with_page_url(prop, o, old_page_url);
            let new_norm = normalize_value_with_page_url(prop, n, new_page_url);
            if old_norm == new_norm
                || values_equal_c2(&old_norm, &new_norm)
                || values_equal_c3(&old_norm, &new_norm)
            {
                equal_count += 1;
            }
        }
    }

    let tag_bonus = if old_tag == new_tag {
        0.05_f64
    } else {
        0.0_f64
    };
    let base = if present_count == 0 {
        0.0_f64
    } else {
        equal_count as f64 / present_count as f64
    };

    // Return uncapped — callers cap to 1.0 only for display/output.
    (base + tag_bonus, base, tag_bonus)
}

// ---------------------------------------------------------------------------
// Canonicalization for equality comparison (WP-C)
// ---------------------------------------------------------------------------

/// Canonicalize an already-normalized CSS property value for equality comparison.
///
/// This step runs AFTER `normalize_value_with_page_url` and BEFORE the equality
/// check in `diff_styles`. Its job is to collapse semantically-equivalent
/// representations to a single canonical form so that they compare equal.
///
/// Rules:
/// 1. `border` / `outline`: if any whitespace-delimited token is exactly `none`
///    (the border-style component), return `"none"` — a border with style none
///    never paints regardless of width or color.  Token match is whole-token only.
/// 2. `text-align`: `start` → `left`, `end` → `right` (whole-value).
///    Constraint: capture does not record writing direction; `start`/`left` and
///    `end`/`right` are equated assuming LTR, the overwhelmingly common case.
/// 3. `line-height`: if value is exactly `normal` and `own_font_size` parses as
///    `<f>px`, return `format!("{:.4}px", f * 1.2)` (UA default ratio).
///
/// All other properties: returned unchanged.
/// The function is pure, deterministic, and allocation-light.
fn canonicalize_for_compare(prop: &str, value: &str, own_font_size: Option<&str>) -> String {
    match prop {
        "border" | "outline" => {
            // Rule 1: if any whole token is exactly "none", the border never paints.
            for token in value.split_ascii_whitespace() {
                if token == "none" {
                    return "none".to_string();
                }
            }
            value.to_string()
        }
        "text-align" => {
            // Rule 2: LTR normalization.  start → left, end → right.
            match value {
                "start" => "left".to_string(),
                "end" => "right".to_string(),
                _ => value.to_string(),
            }
        }
        "line-height" => {
            // Rule 3: resolve `normal` using the UA ratio 1.2 × font-size.
            if value == "normal" {
                if let Some(fs_str) = own_font_size {
                    // Expect the form "<number>px"
                    if let Some(num_str) = fs_str.strip_suffix("px") {
                        if let Ok(fs) = num_str.trim().parse::<f64>() {
                            return format!("{:.4}px", fs * 1.2);
                        }
                    }
                }
            }
            value.to_string()
        }
        _ => value.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Core style diff (used by both channels)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn diff_styles(
    old_styles: &BTreeMap<String, String>,
    new_styles: &BTreeMap<String, String>,
    old_anchors: &Anchors,
    css_selector_old: Option<&str>,
    css_selector_new: Option<&str>,
    bbox_old: Option<[i32; 4]>,
    bbox_new: Option<[i32; 4]>,
    seq_index_old: Option<u32>,
    seq_index_new: Option<u32>,
    match_evidence: &serde_json::Value,
    _confidence: f64,
    viewport: &str,
    locale: &Option<String>,
    profile: &SeverityResolver,
    env_mismatch: bool,
    old_det: &crate::contract::CaptureDeterminism,
    new_det: &crate::contract::CaptureDeterminism,
    old_page_url: &str,
    new_page_url: &str,
    is_uncertain: bool,
) -> Vec<Issue> {
    let mut issues: Vec<Issue> = Vec::new();
    use crate::contract::IssueSeverity;

    // Iterate diff property list in fixed order (STYLE_DIFF_PROPERTIES is a &[&str] slice)
    for prop in STYLE_DIFF_PROPERTIES {
        let old_v = match old_styles.get(*prop) {
            Some(v) => v.as_str(),
            None => continue,
        };
        let new_v = match new_styles.get(*prop) {
            Some(v) => v.as_str(),
            None => continue,
        };

        let old_norm = normalize_value_with_page_url(prop, old_v, old_page_url);
        let new_norm = normalize_value_with_page_url(prop, new_v, new_page_url);

        if old_norm == new_norm {
            continue; // Equal after normalization → no issue
        }

        // C2: sub-pixel numeric epsilon (M6 calibration) — treat as equal
        // when all non-numeric tokens match and every numeric pair differs by
        // less than STYLE_NUMERIC_EPSILON with identical unit.
        if values_equal_c2(&old_norm, &new_norm) {
            continue;
        }

        // C3: url() filename-tail comparison (M6 calibration) — treat as equal
        // when the url-insensitive forms are equal and at least one url() token
        // was present (same asset, different CDN host or path prefix).
        if values_equal_c3(&old_norm, &new_norm) {
            continue;
        }

        // C4: canonicalize for semantic equivalence (WP-C).
        // `own_font_size` is taken from each side's own style map for the
        // line-height rule; evidence/messages keep using old_norm/new_norm.
        {
            let old_fs = old_styles.get("font-size").map(String::as_str);
            let new_fs = new_styles.get("font-size").map(String::as_str);
            let old_canon = canonicalize_for_compare(prop, &old_norm, old_fs);
            let new_canon = canonicalize_for_compare(prop, &new_norm, new_fs);
            if old_canon == new_canon {
                continue; // Semantically equivalent → no issue
            }
            // Also run C2 epsilon on the canonical forms (needed for line-height:
            // real UA "normal" ratios vary slightly, e.g. 22.6094px vs 22.608px).
            if values_equal_c2(&old_canon, &new_canon) {
                continue;
            }
        }

        // Classification
        if *prop == "background-image" {
            // Parse both sides for gradients
            let old_grads = extract_gradients(old_v);
            let new_grads = extract_gradients(new_v);
            let old_has_gradient = !old_grads.is_empty();
            let new_has_gradient = !new_grads.is_empty();

            let (issue_type, goal) = if old_has_gradient && !new_has_gradient {
                (IssueType::BackgroundGradientLost, "G4")
            } else if old_has_gradient && new_has_gradient {
                (IssueType::BackgroundGradientChanged, "G4")
            } else {
                // No gradient classification — fall through to style_changed
                let base = base_confidence::STYLE_CHANGED;
                let conf = compute_confidence(base, env_mismatch, old_det, new_det);
                let id =
                    compute_issue_id(&IssueType::StyleChanged, viewport, old_anchors, Some(prop));
                let sev = profile.severity_for_property(
                    &IssueType::StyleChanged,
                    &IssueCategory::Style,
                    prop,
                );
                let evidence = build_prop_evidence(prop, old_v, new_v, match_evidence, None);
                let remediation = build_remediation(prop, old_v, new_v, old_anchors);
                let message = build_message(prop, old_v, new_v, old_anchors);
                issues.push(Issue {
                    id,
                    issue_type: IssueType::StyleChanged,
                    category: IssueCategory::Style,
                    severity: sev,
                    confidence: conf,
                    viewport: viewport.to_string(),
                    locale: locale.clone(),
                    goal: Some("G1".to_string()),
                    message,
                    locator: build_locator(
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
                continue;
            };

            // Gradient issue (suppresses generic style_changed)
            let base = base_confidence::GRADIENT;
            let conf = compute_confidence(base, env_mismatch, old_det, new_det);
            let id = compute_issue_id(&issue_type, viewport, old_anchors, Some(prop));
            let sev = profile.severity_for_property(&issue_type, &IssueCategory::Style, prop);

            let grad_evidence = serde_json::json!({
                "old": gradients_to_json(&old_grads),
                "new": gradients_to_json(&new_grads)
            });

            let evidence =
                build_prop_evidence(prop, old_v, new_v, match_evidence, Some(grad_evidence));
            let remediation = build_remediation(prop, old_v, new_v, old_anchors);
            let message = build_gradient_message(&issue_type, old_anchors);
            issues.push(Issue {
                id,
                issue_type,
                category: IssueCategory::Style,
                severity: sev,
                confidence: conf,
                viewport: viewport.to_string(),
                locale: locale.clone(),
                goal: Some(goal.to_string()),
                message,
                locator: build_locator(
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
        } else {
            // Generic style_changed
            let base = base_confidence::STYLE_CHANGED;
            let conf = compute_confidence(base, env_mismatch, old_det, new_det);
            let id = compute_issue_id(&IssueType::StyleChanged, viewport, old_anchors, Some(prop));
            let sev = profile.severity_for_property(
                &IssueType::StyleChanged,
                &IssueCategory::Style,
                prop,
            );

            let evidence = build_prop_evidence(prop, old_v, new_v, match_evidence, None);
            let remediation = build_remediation(prop, old_v, new_v, old_anchors);
            let message = build_message(prop, old_v, new_v, old_anchors);
            issues.push(Issue {
                id,
                issue_type: IssueType::StyleChanged,
                category: IssueCategory::Style,
                severity: sev,
                confidence: conf,
                viewport: viewport.to_string(),
                locale: locale.clone(),
                goal: Some("G1".to_string()),
                message,
                locator: build_locator(
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
    }

    // Uncertain-pairing gate (bug p1-04): when the pairing is uncertain
    // (band != Matched OR score < MIN_PAIRING_SCORE_FOR_STYLE), override
    // severity to Info for all emitted issues. The evidence.match already
    // has `uncertainPairing: true` injected by pair_match_evidence.
    if is_uncertain {
        for issue in &mut issues {
            issue.severity = IssueSeverity::Info;
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// Value normalization
// ---------------------------------------------------------------------------

/// Normalize a CSS property value for comparison, using `page_final_url` for url() resolution.
///
/// - Trim + collapse internal whitespace.
/// - For color properties: parse to canonical rgb()/rgba() form.
/// - Rewrite same-site url() tokens via `norm_href` so path-prefix-mounted pages compare equal.
fn normalize_value_with_page_url(prop: &str, value: &str, page_final_url: &str) -> String {
    let trimmed = collapse_whitespace(value);
    // Color properties
    let after_color = if is_color_property(prop) {
        if let Some(canon) = normalize_color(&trimmed) {
            canon
        } else {
            trimmed
        }
    } else {
        trimmed
    };
    // url() normalization: skip if no page URL available
    if page_final_url.is_empty() {
        return after_color;
    }
    normalize_url_origins(&after_color, page_final_url)
}

/// Extract the origin (scheme+host+port, default ports normalized) from a URL string.
/// Returns an empty string on parse failure.
pub fn extract_origin(url_str: &str) -> String {
    let parsed = match url::Url::parse(url_str) {
        Ok(u) => u,
        Err(_) => return String::new(),
    };
    // Only handle http/https
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return String::new();
    }
    let host = match parsed.host_str() {
        Some(h) => h,
        None => return String::new(),
    };
    // Normalize default ports: http:80, https:443
    let explicit_port = parsed.port();
    let default_port: Option<u16> = match scheme {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    };
    let non_default_port = explicit_port.and_then(|p| {
        if Some(p) == default_port {
            None
        } else {
            Some(p)
        }
    });
    if let Some(port) = non_default_port {
        format!("{}://{}:{}", scheme, host, port)
    } else {
        format!("{}://{}", scheme, host)
    }
}

/// Return the directory prefix of a page URL's path: everything up to and including
/// the last `/`.  For `http://localhost:3000/` this is `/`; for
/// `http://localhost:3014/products/connect/branded-call/` this is
/// `/products/connect/branded-call/`.  Returns `/` on any parse failure (safe default).
fn page_dir(page_final_url: &str) -> String {
    match url::Url::parse(page_final_url) {
        Ok(u) => {
            let path = u.path();
            // rfind gives the position of the last '/'
            match path.rfind('/') {
                Some(pos) => path[..=pos].to_string(),
                None => "/".to_string(),
            }
        }
        Err(_) => "/".to_string(),
    }
}

/// Scan `value` for url("..."), url('...'), url(...) tokens.
///
/// For each token whose inner content is a same-site URL (same host as
/// `page_final_url`), apply two-step normalisation so that path-prefix-mounted
/// pages compare equal:
///
/// 1. `norm_href` strips the origin, yielding an absolute path like `/assets/x.svg`
///    or `/products/connect/branded-call/assets/x.svg`.
/// 2. If that absolute path starts with the page's own directory prefix (e.g.
///    `/products/connect/branded-call/`), strip it, leaving `assets/x.svg`.
///    Both sides then produce the same page-relative form.
///    If the path does NOT start with the page dir the absolute path is kept
///    as-is — this is a genuine author-controlled root-anchored reference.
///
/// External/third-party URLs pass through unchanged.  Never panics.
pub fn normalize_url_origins(value: &str, page_final_url: &str) -> String {
    if page_final_url.is_empty() {
        return value.to_string();
    }
    let dir = page_dir(page_final_url);
    let mut result = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Case-insensitive search for "url("
        if i + 4 <= bytes.len()
            && bytes[i].eq_ignore_ascii_case(&b'u')
            && bytes[i + 1].eq_ignore_ascii_case(&b'r')
            && bytes[i + 2].eq_ignore_ascii_case(&b'l')
            && bytes[i + 3] == b'('
        {
            let inner_start = i + 4; // position right after "url("
                                     // Find matching close paren (paren-aware)
            if let Some(close_pos) = find_matching_paren(value, i + 3) {
                let inner_raw = &value[inner_start..close_pos];
                // Strip optional surrounding quotes from the inner value
                let inner = strip_url_quotes(inner_raw);
                // Determine quote style of original token
                let quote_char = {
                    let b = inner_raw.trim_start().as_bytes();
                    if !b.is_empty() && (b[0] == b'"' || b[0] == b'\'') {
                        Some(b[0] as char)
                    } else {
                        None
                    }
                };
                // Step 1: norm_href — same-site → absolute path+query; external → full URL.
                // Returns input unchanged for non-http/https (data:, etc.) — safe passthrough.
                let normed = norm_href(inner, page_final_url);

                // Step 2: if same-site (norm_href changed the value and result is root-relative),
                // additionally strip the page directory prefix.
                let final_normed = if normed != inner && normed.starts_with('/') {
                    if normed.starts_with(dir.as_str()) && dir != "/" {
                        // Strip the page-dir prefix, leaving a page-relative path.
                        normed[dir.len()..].to_string()
                    } else if dir == "/" {
                        // Page is at root: strip leading slash for consistency.
                        normed[1..].to_string()
                    } else {
                        // Asset is not under the page dir — keep absolute path.
                        normed
                    }
                } else {
                    normed.clone()
                };

                if final_normed != inner {
                    result.push_str("url(");
                    if let Some(q) = quote_char {
                        result.push(q);
                        result.push_str(&final_normed);
                        result.push(q);
                    } else {
                        result.push_str(&final_normed);
                    }
                    result.push(')');
                } else {
                    // No change — copy original token verbatim
                    result.push_str(&value[i..=close_pos]);
                }
                i = close_pos + 1;
                continue;
            }
            // Couldn't find close paren — copy char verbatim
            result.push(value.as_bytes()[i] as char);
            i += 1;
        } else {
            result.push(value.as_bytes()[i] as char);
            i += 1;
        }
    }
    result
}

/// Strip leading/trailing quotes (single or double) from a url() inner string.
fn strip_url_quotes(s: &str) -> &str {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

// ---------------------------------------------------------------------------
// C2: sub-pixel numeric epsilon comparison (M6 calibration)
// ---------------------------------------------------------------------------

/// A token in a CSS property value: either a number+unit pair or a non-numeric fragment.
#[derive(Debug, PartialEq)]
enum CssToken {
    Numeric { value: f64, unit: String },
    Text(String),
}

/// Tokenize a CSS value string into alternating non-numeric / numeric runs.
///
/// Splits the string at every contiguous run of `[0-9.]` that is followed by
/// an optional unit (`px`, `%`, `em`, `rem`, etc.) or nothing. The unit is
/// everything after the digits up to the next non-letter character. This is
/// intentionally simple and deterministic — we only need it for sub-pixel
/// jitter detection, not full CSS parsing.
fn tokenize_css_value(s: &str) -> Vec<CssToken> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut text_start = 0;

    while i < n {
        // Try to start a numeric token here. A digit (or '.' followed by digit)
        // begins a number token.
        let is_num_start = chars[i].is_ascii_digit()
            || (chars[i] == '.' && i + 1 < n && chars[i + 1].is_ascii_digit());
        if is_num_start {
            // Flush any pending text token.
            if i > text_start {
                tokens.push(CssToken::Text(chars[text_start..i].iter().collect()));
            }
            // Consume digits and at most one '.'.
            let num_start = i;
            let mut dot_seen = false;
            while i < n && (chars[i].is_ascii_digit() || (chars[i] == '.' && !dot_seen)) {
                if chars[i] == '.' {
                    dot_seen = true;
                }
                i += 1;
            }
            let num_str: String = chars[num_start..i].iter().collect();
            let value: f64 = num_str.parse().unwrap_or(f64::NAN);
            // Consume trailing unit letters (and '%').
            let unit_start = i;
            while i < n && (chars[i].is_ascii_alphabetic() || chars[i] == '%') {
                i += 1;
            }
            let unit: String = chars[unit_start..i].iter().collect();
            tokens.push(CssToken::Numeric { value, unit });
            text_start = i;
        } else {
            i += 1;
        }
    }
    // Flush remaining text.
    if text_start < n {
        tokens.push(CssToken::Text(chars[text_start..].iter().collect()));
    }
    tokens
}

/// Returns true if two already-normalised CSS property values are equal under
/// sub-pixel numeric epsilon (C2, M6 calibration).
///
/// Equality holds when both values tokenize to the same sequence where:
///   - every non-numeric (Text) token pair is identical, and
///   - every numeric token pair has identical unit and value difference < STYLE_NUMERIC_EPSILON.
///
/// If the sequences have different lengths or any non-numeric token differs, returns false.
fn values_equal_c2(a: &str, b: &str) -> bool {
    use crate::config::STYLE_NUMERIC_EPSILON;
    if a == b {
        return true;
    }
    let ta = tokenize_css_value(a);
    let tb = tokenize_css_value(b);
    if ta.len() != tb.len() {
        return false;
    }
    for (ta_tok, tb_tok) in ta.iter().zip(tb.iter()) {
        match (ta_tok, tb_tok) {
            (CssToken::Text(sa), CssToken::Text(sb)) => {
                if sa != sb {
                    return false;
                }
            }
            (
                CssToken::Numeric {
                    value: va,
                    unit: ua,
                },
                CssToken::Numeric {
                    value: vb,
                    unit: ub,
                },
            ) => {
                if ua != ub {
                    return false;
                }
                if (va - vb).abs() >= STYLE_NUMERIC_EPSILON {
                    return false;
                }
            }
            _ => return false, // mismatched token types
        }
    }
    true
}

// ---------------------------------------------------------------------------
// C3: url() filename-tail comparison in style values (M6 calibration)
// ---------------------------------------------------------------------------

/// Compute the url-insensitive form of a CSS property value: replace every
/// `url(...)` token with just the filename (final path segment, query/fragment
/// stripped) of its inner URL.
///
/// Returns `(insensitive_form, first_host)` where `first_host` is the
/// lowercase host of the first url() token found (empty string if the token
/// is not an absolute URL).  Used by C3 to gate cross-host suppression.
fn url_insensitive_form(value: &str) -> (String, String) {
    let mut result = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0;
    let mut first_host = String::new();

    while i < bytes.len() {
        if i + 4 <= bytes.len()
            && bytes[i].eq_ignore_ascii_case(&b'u')
            && bytes[i + 1].eq_ignore_ascii_case(&b'r')
            && bytes[i + 2].eq_ignore_ascii_case(&b'l')
            && bytes[i + 3] == b'('
        {
            let inner_start = i + 4;
            if let Some(close_pos) = find_matching_paren(value, i + 3) {
                let inner_raw = &value[inner_start..close_pos];
                let inner = strip_url_quotes(inner_raw);
                // Extract path segment (strip query + fragment, then take after last '/').
                let path_only = if let Some(q) = inner.find('?') {
                    &inner[..q]
                } else if let Some(f) = inner.find('#') {
                    &inner[..f]
                } else {
                    inner
                };
                let filename = match path_only.rfind('/') {
                    Some(pos) => &path_only[pos + 1..],
                    None => path_only,
                };
                // Record host of the first url() token for cross-host gating.
                if first_host.is_empty() {
                    if let Ok(parsed) = url::Url::parse(inner) {
                        if let Some(h) = parsed.host_str() {
                            first_host = h.to_ascii_lowercase();
                        }
                    }
                }
                result.push_str("url(");
                result.push_str(filename);
                result.push(')');
                i = close_pos + 1;
                continue;
            }
            result.push(bytes[i] as char);
            i += 1;
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    (result, first_host)
}

/// Returns true if two already-normalised CSS property values are equal under
/// url()-filename-tail comparison (C3, M6 calibration).
///
/// Suppresses (treats as equal) when the url-insensitive forms are equal AND
/// the host pair indicates migration noise rather than an author-controlled
/// path change.  The guard condition is:
///
///   `(host_a != host_b) && (host_a.is_empty() || host_b.is_empty() || host_a != host_b)`
///
/// Written out plainly — suppress when AT LEAST ONE host is non-empty AND the
/// two hosts are not the same:
///
/// - relative vs absolute (one host empty, one non-empty): own-origin was
///   stripped by `normalize_url_origins`; the absolute side is a CDN/third-party.
///   Migration noise → suppress.
/// - absolute vs absolute, different hosts: cross-CDN migration → suppress.
/// - both hosts equal (same CDN, different version path e.g. v1/ vs v2/):
///   genuine author-controlled change → DO NOT suppress.
/// - both hosts empty (both relative, same origin implied, e.g.
///   url("assets/a.svg") vs url("images/a.svg")): author-controlled path
///   change → DO NOT suppress.
fn values_equal_c3(a: &str, b: &str) -> bool {
    let (form_a, host_a) = url_insensitive_form(a);
    let (form_b, host_b) = url_insensitive_form(b);
    // Both relative (both hosts empty) → same-origin path change, not migration noise.
    if host_a.is_empty() && host_b.is_empty() {
        return false;
    }
    // Same non-empty host on both sides → same-host different-path, genuine change.
    if !host_a.is_empty() && !host_b.is_empty() && host_a == host_b {
        return false;
    }
    // Remaining cases: one or both hosts non-empty and they differ → migration noise.
    form_a == form_b
}

fn collapse_whitespace(s: &str) -> String {
    let trimmed = s.trim();
    let mut result = String::with_capacity(trimmed.len());
    let mut last_was_space = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                result.push(' ');
            }
            last_was_space = true;
        } else {
            result.push(ch);
            last_was_space = false;
        }
    }
    result
}

fn is_color_property(prop: &str) -> bool {
    matches!(prop, "color" | "background-color" | "border-color")
}

/// Normalize a CSS color string to canonical rgb(r, g, b) or rgba(r, g, b, a) form.
/// Returns None if parsing fails (caller uses raw value).
pub fn normalize_color(value: &str) -> Option<String> {
    let s = value.trim().to_lowercase();

    // rgb(r, g, b) or rgba(r, g, b, a) — already in canonical form from browser
    if s.starts_with("rgb(") && s.ends_with(')') {
        let inner = &s[4..s.len() - 1];
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 3 {
            let r = parts[0].trim().parse::<u8>().ok()?;
            let g = parts[1].trim().parse::<u8>().ok()?;
            let b = parts[2].trim().parse::<u8>().ok()?;
            return Some(format!("rgb({}, {}, {})", r, g, b));
        }
    }
    if s.starts_with("rgba(") && s.ends_with(')') {
        let inner = &s[5..s.len() - 1];
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 4 {
            let r = parts[0].trim().parse::<u8>().ok()?;
            let g = parts[1].trim().parse::<u8>().ok()?;
            let b = parts[2].trim().parse::<u8>().ok()?;
            let a_str = parts[3].trim();
            let a: f64 = a_str.parse().ok()?;
            // alpha == 1 → convert to rgb
            if (a - 1.0).abs() < 1e-9 {
                return Some(format!("rgb({}, {}, {})", r, g, b));
            }
            // Round alpha to avoid float noise
            let a_rounded = (a * 1000.0).round() / 1000.0;
            return Some(format!("rgba({}, {}, {}, {})", r, g, b, a_rounded));
        }
    }

    // #rgb → #rrggbb → rgb()
    if let Some(hex) = s.strip_prefix('#') {
        let (r, g, b, a) = if hex.len() == 3 {
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
            (r, g, b, None::<u8>)
        } else if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            (r, g, b, None::<u8>)
        } else if hex.len() == 8 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a_byte = u8::from_str_radix(&hex[6..8], 16).ok()?;
            (r, g, b, Some(a_byte))
        } else {
            return None;
        };
        return if let Some(a_byte) = a {
            let alpha = a_byte as f64 / 255.0;
            if (alpha - 1.0).abs() < 1e-3 {
                Some(format!("rgb({}, {}, {})", r, g, b))
            } else {
                let a_rounded = (alpha * 1000.0).round() / 1000.0;
                Some(format!("rgba({}, {}, {}, {})", r, g, b, a_rounded))
            }
        } else {
            Some(format!("rgb({}, {}, {})", r, g, b))
        };
    }

    // Named colors → rgb
    named_color_to_rgb(&s).map(|(r, g, b)| format!("rgb({}, {}, {})", r, g, b))
}

/// Map CSS named colors to (r, g, b).
/// Includes basic CSS colors + common extended ones.
fn named_color_to_rgb(name: &str) -> Option<(u8, u8, u8)> {
    match name {
        // Basic CSS colors
        "black" => Some((0, 0, 0)),
        "silver" => Some((192, 192, 192)),
        "gray" | "grey" => Some((128, 128, 128)),
        "white" => Some((255, 255, 255)),
        "maroon" => Some((128, 0, 0)),
        "red" => Some((255, 0, 0)),
        "purple" => Some((128, 0, 128)),
        "fuchsia" | "magenta" => Some((255, 0, 255)),
        "green" => Some((0, 128, 0)),
        "lime" => Some((0, 255, 0)),
        "olive" => Some((128, 128, 0)),
        "yellow" => Some((255, 255, 0)),
        "navy" => Some((0, 0, 128)),
        "blue" => Some((0, 0, 255)),
        "teal" => Some((0, 128, 128)),
        "aqua" | "cyan" => Some((0, 255, 255)),
        "orange" => Some((255, 165, 0)),
        "coral" => Some((255, 127, 80)),
        "tomato" => Some((255, 99, 71)),
        "pink" => Some((255, 192, 203)),
        "hotpink" => Some((255, 105, 180)),
        "deeppink" => Some((255, 20, 147)),
        "salmon" => Some((250, 128, 114)),
        "lightsalmon" => Some((255, 160, 122)),
        "darksalmon" => Some((233, 150, 122)),
        "lightcoral" => Some((240, 128, 128)),
        "indianred" => Some((205, 92, 92)),
        "crimson" => Some((220, 20, 60)),
        "firebrick" => Some((178, 34, 34)),
        "darkred" => Some((139, 0, 0)),
        "orangered" => Some((255, 69, 0)),
        "darkorange" => Some((255, 140, 0)),
        "gold" => Some((255, 215, 0)),
        "lightyellow" => Some((255, 255, 224)),
        "lemonchiffon" => Some((255, 250, 205)),
        "papayawhip" => Some((255, 239, 213)),
        "moccasin" => Some((255, 228, 181)),
        "peachpuff" => Some((255, 218, 185)),
        "khaki" => Some((240, 230, 140)),
        "darkkhaki" => Some((189, 183, 107)),
        "yellowgreen" => Some((154, 205, 50)),
        "chartreuse" => Some((127, 255, 0)),
        "lawngreen" => Some((124, 252, 0)),
        "greenyellow" => Some((173, 255, 47)),
        "palegreen" => Some((152, 251, 152)),
        "lightgreen" => Some((144, 238, 144)),
        "mediumspringgreen" => Some((0, 250, 154)),
        "springgreen" => Some((0, 255, 127)),
        "mediumseagreen" => Some((60, 179, 113)),
        "seagreen" => Some((46, 139, 87)),
        "forestgreen" => Some((34, 139, 34)),
        "darkgreen" => Some((0, 100, 0)),
        "limegreen" => Some((50, 205, 50)),
        "darkseagreen" => Some((143, 188, 143)),
        "lightseagreen" => Some((32, 178, 170)),
        "mediumaquamarine" => Some((102, 205, 170)),
        "aquamarine" => Some((127, 255, 212)),
        "turquoise" => Some((64, 224, 208)),
        "mediumturquoise" => Some((72, 209, 204)),
        "darkturquoise" => Some((0, 206, 209)),
        "cadetblue" => Some((95, 158, 160)),
        "steelblue" => Some((70, 130, 180)),
        "lightsteelblue" => Some((176, 196, 222)),
        "powderblue" => Some((176, 224, 230)),
        "lightblue" => Some((173, 216, 230)),
        "skyblue" => Some((135, 206, 235)),
        "lightskyblue" => Some((135, 206, 250)),
        "deepskyblue" => Some((0, 191, 255)),
        "dodgerblue" => Some((30, 144, 255)),
        "cornflowerblue" => Some((100, 149, 237)),
        "royalblue" => Some((65, 105, 225)),
        "mediumblue" => Some((0, 0, 205)),
        "darkblue" => Some((0, 0, 139)),
        "midnightblue" => Some((25, 25, 112)),
        "slateblue" => Some((106, 90, 205)),
        "mediumslateblue" => Some((123, 104, 238)),
        "darkslateblue" => Some((72, 61, 139)),
        "blueviolet" => Some((138, 43, 226)),
        "indigo" => Some((75, 0, 130)),
        "darkviolet" => Some((148, 0, 211)),
        "darkorchid" => Some((153, 50, 204)),
        "mediumpurple" => Some((147, 112, 219)),
        "orchid" => Some((218, 112, 214)),
        "plum" => Some((221, 160, 221)),
        "violet" => Some((238, 130, 238)),
        "lavender" => Some((230, 230, 250)),
        "thistle" => Some((216, 191, 216)),
        "darkmagenta" => Some((139, 0, 139)),
        "mediumvioletred" => Some((199, 21, 133)),
        "palevioletred" => Some((219, 112, 147)),
        "rosybrown" => Some((188, 143, 143)),
        "saddlebrown" => Some((139, 69, 19)),
        "sienna" => Some((160, 82, 45)),
        "brown" => Some((165, 42, 42)),
        "tan" => Some((210, 180, 140)),
        "sandybrown" => Some((244, 164, 96)),
        "burlywood" => Some((222, 184, 135)),
        "wheat" => Some((245, 222, 179)),
        "navajowhite" => Some((255, 222, 173)),
        "bisque" => Some((255, 228, 196)),
        "blanchedalmond" => Some((255, 235, 205)),
        "cornsilk" => Some((255, 248, 220)),
        "ivory" => Some((255, 255, 240)),
        "floralwhite" => Some((255, 250, 240)),
        "oldlace" => Some((253, 245, 230)),
        "linen" => Some((250, 240, 230)),
        "antiquewhite" => Some((250, 235, 215)),
        "seashell" => Some((255, 245, 238)),
        "honeydew" => Some((240, 255, 240)),
        "mintcream" => Some((245, 255, 250)),
        "azure" => Some((240, 255, 255)),
        "aliceblue" => Some((240, 248, 255)),
        "ghostwhite" => Some((248, 248, 255)),
        "whitesmoke" => Some((245, 245, 245)),
        "snow" => Some((255, 250, 250)),
        "gainsboro" => Some((220, 220, 220)),
        "lightgray" | "lightgrey" => Some((211, 211, 211)),
        "darkgray" | "darkgrey" => Some((169, 169, 169)),
        "dimgray" | "dimgrey" => Some((105, 105, 105)),
        "slategray" | "slategrey" => Some((112, 128, 144)),
        "lightslategray" | "lightslategrey" => Some((119, 136, 153)),
        "darkslategray" | "darkslategrey" => Some((47, 79, 79)),
        "transparent" => None, // special — skip
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Gradient parser (§3.3)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct GradientStop {
    pub color: String,
    pub position: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GradientSpec {
    pub kind: String,
    pub angle: Option<String>,
    pub stops: Vec<GradientStop>,
    /// Raw string, populated when parsing fails (degrade-to-presence-only).
    pub raw: Option<String>,
}

/// Parse a computed background-image value into gradient specs.
/// Returns only gradient layers (url(), none, etc. are ignored).
/// Never panics.
pub fn extract_gradients(value: &str) -> Vec<GradientSpec> {
    let layers = split_top_level_commas(value);
    let mut result = Vec::new();
    for layer in layers {
        let layer = layer.trim();
        if let Some(spec) = parse_gradient_layer(layer) {
            result.push(spec);
        }
    }
    result
}

/// Split a string on top-level commas (paren-aware).
pub fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0u32;
    let mut start = 0;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
            }
            b',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start <= s.len() {
        parts.push(&s[start..]);
    }
    parts
}

/// Try to parse a single background-image layer as a gradient.
fn parse_gradient_layer(layer: &str) -> Option<GradientSpec> {
    let lower = layer.trim().to_lowercase();

    // Determine gradient kind
    let (kind, prefix_len) = if lower.starts_with("repeating-linear-gradient(") {
        (
            "repeating-linear".to_string(),
            "repeating-linear-gradient(".len(),
        )
    } else if lower.starts_with("repeating-radial-gradient(") {
        (
            "repeating-radial".to_string(),
            "repeating-radial-gradient(".len(),
        )
    } else if lower.starts_with("repeating-conic-gradient(") {
        (
            "repeating-conic".to_string(),
            "repeating-conic-gradient(".len(),
        )
    } else if lower.starts_with("linear-gradient(") {
        ("linear".to_string(), "linear-gradient(".len())
    } else if lower.starts_with("radial-gradient(") {
        ("radial".to_string(), "radial-gradient(".len())
    } else if lower.starts_with("conic-gradient(") {
        ("conic".to_string(), "conic-gradient(".len())
    } else {
        return None; // Not a gradient
    };

    // Extract the inner content (strip closing paren)
    let trimmed = layer.trim();
    if !trimmed.ends_with(')') {
        // Malformed — degrade to presence-only
        return Some(GradientSpec {
            kind,
            angle: None,
            stops: vec![],
            raw: Some(layer.to_string()),
        });
    }
    let inner = &trimmed[prefix_len..trimmed.len() - 1];

    // Split inner on top-level commas
    let parts = split_top_level_commas(inner);
    if parts.is_empty() {
        return Some(GradientSpec {
            kind,
            angle: None,
            stops: vec![],
            raw: None,
        });
    }

    // Try to identify angle/direction prefix in first part
    let first_trimmed = parts[0].trim().to_lowercase();
    let (angle, stop_start_idx) = if is_angle_or_direction(&first_trimmed) {
        (Some(parts[0].trim().to_string()), 1)
    } else {
        (None, 0)
    };

    // Parse stops from remaining parts
    let mut stops = Vec::new();
    for part in &parts[stop_start_idx..] {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        // Each part is: <color> [<position>...]
        // Color token may be paren-enclosed (rgb(...), hsl(...), etc.)
        if let Some(stop) = parse_gradient_stop(part) {
            stops.push(stop);
        }
    }

    Some(GradientSpec {
        kind,
        angle,
        stops,
        raw: None,
    })
}

fn is_angle_or_direction(s: &str) -> bool {
    // Angle: <number>deg / rad / grad / turn
    // Direction: to <side/corner>
    s.starts_with("to ")
        || s.ends_with("deg")
        || s.ends_with("rad")
        || s.ends_with("grad")
        || s.ends_with("turn")
        || s.parse::<f64>().is_ok() // bare number (0)
}

/// Parse a gradient stop token: <color> [<position>]
fn parse_gradient_stop(s: &str) -> Option<GradientStop> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Find where the color token ends.
    // Color can be: named, #hex, rgb(...), rgba(...), hsl(...), hsla(...)
    if s.starts_with("rgb(")
        || s.starts_with("rgba(")
        || s.starts_with("hsl(")
        || s.starts_with("hsla(")
    {
        // Find closing paren
        let close = find_matching_paren(s, 0)?;
        let color = s[..=close].to_string();
        let rest = s[close + 1..].trim();
        let position = if rest.is_empty() {
            None
        } else {
            Some(rest.to_string())
        };
        return Some(GradientStop { color, position });
    }

    // Everything else: split on first space after the color token
    if s.starts_with('#') {
        // hex color: ends at first space
        let end = s.find(|c: char| c.is_whitespace()).unwrap_or(s.len());
        let color = s[..end].to_string();
        let rest = s[end..].trim();
        let position = if rest.is_empty() {
            None
        } else {
            Some(rest.to_string())
        };
        return Some(GradientStop { color, position });
    }

    // Named color or other: take first word
    let end = s.find(|c: char| c.is_whitespace()).unwrap_or(s.len());
    let color = s[..end].to_string();
    let rest = s[end..].trim();
    let position = if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    };
    Some(GradientStop { color, position })
}

/// Find the position of the closing ')' matching the '(' at `open_pos`.
fn find_matching_paren(s: &str, open_pos: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    if open_pos >= bytes.len() || bytes[open_pos] != b'(' {
        // Find '(' first
        let start = s[open_pos..].find('(')?;
        let open_pos2 = open_pos + start;
        return find_matching_paren(s, open_pos2);
    }
    let mut depth = 0u32;
    for (i, &b) in bytes[open_pos..].iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(open_pos + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Convert gradient specs to JSON value for evidence.
fn gradients_to_json(specs: &[GradientSpec]) -> serde_json::Value {
    let arr: Vec<serde_json::Value> = specs
        .iter()
        .map(|spec| {
            let stops: Vec<serde_json::Value> = spec
                .stops
                .iter()
                .map(|stop| {
                    serde_json::json!({
                        "color": stop.color,
                        "position": stop.position
                    })
                })
                .collect();
            let mut obj = serde_json::json!({
                "kind": spec.kind,
                "angle": spec.angle,
                "stops": stops
            });
            if let Some(raw) = &spec.raw {
                obj["raw"] = serde_json::Value::String(raw.clone());
            }
            obj
        })
        .collect();
    serde_json::Value::Array(arr)
}

// ---------------------------------------------------------------------------
// Issue construction helpers
// ---------------------------------------------------------------------------

/// Build a property-level evidence JSON object.
/// Avoids &&str by taking prop as &str.
fn build_prop_evidence(
    prop: &str,
    old_v: &str,
    new_v: &str,
    match_evidence: &serde_json::Value,
    gradient: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut old_map = serde_json::Map::new();
    old_map.insert(
        prop.to_string(),
        serde_json::Value::String(old_v.to_string()),
    );
    let mut new_map = serde_json::Map::new();
    new_map.insert(
        prop.to_string(),
        serde_json::Value::String(new_v.to_string()),
    );

    let mut ev = serde_json::Map::new();
    ev.insert("old".to_string(), serde_json::Value::Object(old_map));
    ev.insert("new".to_string(), serde_json::Value::Object(new_map));
    ev.insert("match".to_string(), match_evidence.clone());
    if let Some(g) = gradient {
        ev.insert("gradient".to_string(), g);
    }
    serde_json::Value::Object(ev)
}

fn build_locator(
    anchors: Anchors,
    css_selector_old: Option<&str>,
    css_selector_new: Option<&str>,
    bbox_old: Option<[i32; 4]>,
    bbox_new: Option<[i32; 4]>,
    seq_index_old: Option<u32>,
    seq_index_new: Option<u32>,
) -> Locator {
    Locator {
        anchors,
        css_selector_old: css_selector_old.map(str::to_string),
        css_selector_new: css_selector_new.map(str::to_string),
        bbox_old,
        bbox_new,
        seq_index_old,
        seq_index_new,
    }
}

fn build_remediation(prop: &str, old_v: &str, new_v: &str, anchors: &Anchors) -> serde_json::Value {
    let near = anchors.nearest_heading.as_deref();
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
    if grep_targets.is_empty() {
        if let Some(nh) = near {
            if !nh.is_empty() {
                grep_targets.push(serde_json::Value::String(nh.to_string()));
            }
        }
    }

    serde_json::json!({
        "action": "restore_css_property",
        "findBy": {
            "grep": grep_targets,
            "near": near
        },
        "property": prop,
        "from": new_v,
        "to": old_v,
        "note": "The tool does not name the component. Use the grep targets to locate it in source or CMS."
    })
}

fn build_message(prop: &str, old_v: &str, new_v: &str, anchors: &Anchors) -> String {
    let near = anchors.nearest_heading.as_deref().unwrap_or("");
    let near_part = if !near.is_empty() {
        format!(" near \"{}\"", near)
    } else {
        String::new()
    };
    format!("{} changed from {} to {}{}", prop, old_v, new_v, near_part)
}

fn build_gradient_message(issue_type: &IssueType, anchors: &Anchors) -> String {
    let near = anchors.nearest_heading.as_deref().unwrap_or("");
    let near_part = if !near.is_empty() {
        format!(" near \"{}\"", near)
    } else {
        String::new()
    };
    match issue_type {
        IssueType::BackgroundGradientLost => {
            format!("Background gradient lost on container{}", near_part)
        }
        IssueType::BackgroundGradientChanged => {
            format!("Background gradient changed on container{}", near_part)
        }
        _ => format!("Background gradient issue{}", near_part),
    }
}

// ---------------------------------------------------------------------------
// Evidence helpers
// ---------------------------------------------------------------------------

fn pair_match_evidence(
    pair: &crate::matching::MatchedPair,
    old_bundle: &CaptureBundle,
    new_bundle: &CaptureBundle,
    is_uncertain: bool,
) -> serde_json::Value {
    let _ = (old_bundle, new_bundle);
    let stage_str = match pair.stage {
        crate::matching::MatchStage::Identity => "identity",
        crate::matching::MatchStage::Assignment => "assignment",
    };
    let band_str = match pair.band {
        MatchBand::Matched => "matched",
        MatchBand::Uncertain => "uncertain",
    };
    let signals_val = {
        let mut map = serde_json::Map::new();
        for (k, v) in &pair.signals {
            map.insert(k.clone(), serde_json::Value::from(round4(*v)));
        }
        serde_json::Value::Object(map)
    };
    // Only emit uncertainPairing key when true (omit the key for confident pairs).
    if is_uncertain {
        serde_json::json!({
            "stage": stage_str,
            "score": round4(pair.score),
            "band": band_str,
            "signals": signals_val,
            "uncertainPairing": true
        })
    } else {
        serde_json::json!({
            "stage": stage_str,
            "score": round4(pair.score),
            "band": band_str,
            "signals": signals_val
        })
    }
}

fn node_to_anchors(node: &crate::contract::SemanticNode) -> Anchors {
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

fn round4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        A11yInfo, AncestorDescriptor, CaptureDeterminism, Environment, NetworkInfo, NodeAnchors,
        PageModel, Screenshots, SemanticNode, StepStatus, StyleCandidates, ViewportConfig,
    };
    use crate::matching::{MatchBand, MatchOutcome, MatchStage, MatchedPair};
    use crate::scoring::{ParityProfile, SeverityResolver};
    use std::collections::BTreeMap;

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

    fn make_bundle(
        url: &str,
        nodes: Vec<SemanticNode>,
        computed_styles: BTreeMap<String, BTreeMap<String, String>>,
    ) -> CaptureBundle {
        make_bundle_with_candidates(url, nodes, computed_styles, StyleCandidates::default())
    }

    fn make_bundle_with_candidates(
        url: &str,
        nodes: Vec<SemanticNode>,
        computed_styles: BTreeMap<String, BTreeMap<String, String>>,
        style_candidates: StyleCandidates,
    ) -> CaptureBundle {
        CaptureBundle {
            schema_version: "1.0".to_string(),
            captured_at: "2026-01-01T00:00:00Z".to_string(),
            viewport: make_viewport_cfg(),
            environment: make_env(),
            determinism: make_det(),
            page: make_page(url, nodes),
            computed_styles,
            screenshots: Screenshots {
                full_page: "desktop/old.png".to_string(),
                viewport: "desktop/old-vp.png".to_string(),
            },
            style_candidates,
            hit_tests: None,
            pseudo_elements: None,
            pseudo_truncated: None,
        }
    }

    fn make_node(id: &str, text: Option<&str>, nearest_heading: Option<&str>) -> SemanticNode {
        SemanticNode {
            id: id.to_string(),
            kind: "text".to_string(),
            role: None,
            text: text.map(str::to_string),
            acc_name: None,
            href: None,
            image_alt: None,
            bbox: [0, 0, 200, 50],
            seq_index: 0,
            anchors: NodeAnchors {
                text: text.map(str::to_string),
                role: None,
                href: None,
                alt: None,
                aria_label: None,
                nearest_heading: nearest_heading.map(str::to_string),
                landmark: Some("main".to_string()),
                ordinal_in_landmark: Some(1),
            },
            css_selector: Some("main p".to_string()),
            raw_href: None,
            src: None,
            natural_width: None,
            natural_height: None,
            loaded: None,
            heading_level: None,
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

    fn profile() -> SeverityResolver {
        SeverityResolver::from_profile(ParityProfile::ContentStructure)
    }

    fn styles(props: &[(&str, &str)]) -> BTreeMap<String, String> {
        props
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    // -----------------------------------------------------------------------
    // Gradient parser tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_gradient_plain_linear() {
        let grads = extract_gradients("linear-gradient(#6d28d9 0%, #2563eb 100%)");
        assert_eq!(grads.len(), 1);
        assert_eq!(grads[0].kind, "linear");
        assert_eq!(grads[0].angle, None);
        assert_eq!(grads[0].stops.len(), 2);
        assert_eq!(grads[0].stops[0].color, "#6d28d9");
        assert_eq!(grads[0].stops[0].position, Some("0%".to_string()));
    }

    #[test]
    fn test_gradient_with_angle_deg() {
        let grads = extract_gradients("linear-gradient(90deg, #6d28d9 0%, #2563eb 100%)");
        assert_eq!(grads.len(), 1);
        assert_eq!(grads[0].kind, "linear");
        assert_eq!(grads[0].angle, Some("90deg".to_string()));
        assert_eq!(grads[0].stops.len(), 2);
    }

    #[test]
    fn test_gradient_with_to_right() {
        let grads = extract_gradients("linear-gradient(to right, red 0%, blue 100%)");
        assert_eq!(grads.len(), 1);
        assert_eq!(grads[0].angle, Some("to right".to_string()));
    }

    #[test]
    fn test_gradient_radial() {
        let grads = extract_gradients("radial-gradient(circle, #fff 0%, #000 100%)");
        assert_eq!(grads.len(), 1);
        assert_eq!(grads[0].kind, "radial");
    }

    #[test]
    fn test_gradient_conic() {
        let grads = extract_gradients("conic-gradient(from 90deg, red, blue)");
        assert_eq!(grads.len(), 1);
        assert_eq!(grads[0].kind, "conic");
    }

    #[test]
    fn test_gradient_repeating_linear() {
        let grads = extract_gradients("repeating-linear-gradient(red, blue 20%)");
        assert_eq!(grads.len(), 1);
        assert_eq!(grads[0].kind, "repeating-linear");
    }

    #[test]
    fn test_gradient_multi_layer_with_url() {
        let grads = extract_gradients("linear-gradient(red, blue), url(\"image.png\")");
        // Only the gradient layer
        assert_eq!(grads.len(), 1);
        assert_eq!(grads[0].kind, "linear");
    }

    #[test]
    fn test_gradient_none() {
        let grads = extract_gradients("none");
        assert_eq!(grads.len(), 0);
    }

    #[test]
    fn test_gradient_url_only() {
        let grads = extract_gradients("url(\"background.png\")");
        assert_eq!(grads.len(), 0);
    }

    #[test]
    fn test_gradient_paren_aware_rgb_stops() {
        let grads = extract_gradients(
            "linear-gradient(90deg, rgb(109, 40, 217) 0%, rgb(37, 99, 235) 100%)",
        );
        assert_eq!(grads.len(), 1);
        assert_eq!(grads[0].stops.len(), 2);
        assert_eq!(grads[0].stops[0].color, "rgb(109, 40, 217)");
        assert_eq!(grads[0].stops[0].position, Some("0%".to_string()));
    }

    #[test]
    fn test_gradient_degrade_on_unparseable() {
        // Completely malformed — should not panic, should return presence-only
        let grads = extract_gradients("linear-gradient(totally-invalid-no-close-paren");
        // Either 0 or 1 spec; must not panic
        // Allow either outcome — the important thing is no panic
        let _ = grads;
    }

    // -----------------------------------------------------------------------
    // normalize_color tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_normalize_color_white_variants() {
        let canon_hex = normalize_color("#fff");
        let canon_named = normalize_color("white");
        let canon_rgb = normalize_color("rgb(255, 255, 255)");
        assert_eq!(canon_hex, Some("rgb(255, 255, 255)".to_string()));
        assert_eq!(canon_named, Some("rgb(255, 255, 255)".to_string()));
        assert_eq!(canon_rgb, Some("rgb(255, 255, 255)".to_string()));
        assert_eq!(canon_hex, canon_named);
        assert_eq!(canon_named, canon_rgb);
    }

    #[test]
    fn test_normalize_color_rgba_alpha_1_becomes_rgb() {
        let c = normalize_color("rgba(255, 0, 0, 1)");
        assert_eq!(c, Some("rgb(255, 0, 0)".to_string()));
    }

    #[test]
    fn test_normalize_color_rgba_with_alpha() {
        let c = normalize_color("rgba(0, 0, 0, 0.5)");
        assert!(c.is_some());
        let s = c.unwrap();
        assert!(s.starts_with("rgba(0, 0, 0,"));
    }

    #[test]
    fn test_normalize_color_hex_rrggbb() {
        let c = normalize_color("#ff0000");
        assert_eq!(c, Some("rgb(255, 0, 0)".to_string()));
    }

    #[test]
    fn test_normalize_color_named_red() {
        let c = normalize_color("red");
        assert_eq!(c, Some("rgb(255, 0, 0)".to_string()));
    }

    #[test]
    fn test_normalize_color_unknown_returns_none() {
        let c = normalize_color("notacolor");
        assert_eq!(c, None);
    }

    // -----------------------------------------------------------------------
    // Leaf channel classification tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_leaf_font_size_changed() {
        let old_node = make_node("o1", Some("Heading"), Some("Heading"));
        let new_node = make_node("n1", Some("Heading"), Some("Heading"));

        let old_cs = styles(&[("font-size", "60px")]);
        let new_cs = styles(&[("font-size", "48px")]);

        let mut old_styles = BTreeMap::new();
        old_styles.insert("o1".to_string(), old_cs);
        let mut new_styles = BTreeMap::new();
        new_styles.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://old.com/", vec![old_node], old_styles);
        let new_b = make_bundle("http://new.com/", vec![new_node], new_styles);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues
                .iter()
                .any(|i| i.issue_type == IssueType::StyleChanged
                    && i.evidence
                        .get("old")
                        .and_then(|o| o.get("font-size"))
                        .is_some()),
            "should emit style_changed for font-size"
        );
    }

    #[test]
    fn test_leaf_equal_after_normalization_emits_nothing() {
        // #fff and white and rgb(255,255,255) are equal
        let old_node = make_node("o1", Some("Text"), None);
        let new_node = make_node("n1", Some("Text"), None);

        let old_cs = styles(&[("color", "#fff")]);
        let new_cs = styles(&[("color", "white")]);

        let mut old_styles = BTreeMap::new();
        old_styles.insert("o1".to_string(), old_cs);
        let mut new_styles = BTreeMap::new();
        new_styles.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://old.com/", vec![old_node], old_styles);
        let new_b = make_bundle("http://new.com/", vec![new_node], new_styles);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues.is_empty(),
            "equal-after-normalization should produce no issues"
        );
    }

    #[test]
    fn test_leaf_gradient_lost_emits_background_gradient_lost() {
        let old_node = make_node("o1", Some("Hero"), Some("Reach more customers"));
        let new_node = make_node("n1", Some("Hero"), Some("Reach more customers"));

        let old_cs = styles(&[(
            "background-image",
            "linear-gradient(90deg, #6d28d9 0%, #2563eb 100%)",
        )]);
        let new_cs = styles(&[("background-image", "none")]);

        let mut old_styles = BTreeMap::new();
        old_styles.insert("o1".to_string(), old_cs);
        let mut new_styles = BTreeMap::new();
        new_styles.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://old.com/", vec![old_node], old_styles);
        let new_b = make_bundle("http://new.com/", vec![new_node], new_styles);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues
                .iter()
                .any(|i| i.issue_type == IssueType::BackgroundGradientLost),
            "should emit background_gradient_lost"
        );
        // Must NOT emit generic style_changed for background-image
        assert!(
            !issues.iter().any(|i| {
                i.issue_type == IssueType::StyleChanged
                    && i.evidence
                        .get("old")
                        .and_then(|o| o.get("background-image"))
                        .is_some()
            }),
            "gradient issue must suppress generic style_changed for background-image"
        );
    }

    #[test]
    fn test_leaf_gradient_suppresses_style_changed() {
        // Even when both sides have gradients (changed), still no style_changed
        let old_node = make_node("o1", Some("Hero"), None);
        let new_node = make_node("n1", Some("Hero"), None);

        let old_cs = styles(&[("background-image", "linear-gradient(red, blue)")]);
        let new_cs = styles(&[("background-image", "linear-gradient(green, yellow)")]);

        let mut old_styles = BTreeMap::new();
        old_styles.insert("o1".to_string(), old_cs);
        let mut new_styles = BTreeMap::new();
        new_styles.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://old.com/", vec![old_node], old_styles);
        let new_b = make_bundle("http://new.com/", vec![new_node], new_styles);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues
                .iter()
                .any(|i| i.issue_type == IssueType::BackgroundGradientChanged),
            "should emit background_gradient_changed"
        );
        assert!(
            !issues
                .iter()
                .any(|i| i.issue_type == IssueType::StyleChanged
                    && i.evidence
                        .get("old")
                        .and_then(|o| o.get("background-image"))
                        .is_some()),
            "gradient issue must suppress generic style_changed"
        );
    }

    #[test]
    fn test_issue_id_changes_with_property() {
        let anchors = Anchors::null();
        let id1 = compute_issue_id(
            &IssueType::StyleChanged,
            "desktop",
            &anchors,
            Some("font-size"),
        );
        let id2 = compute_issue_id(&IssueType::StyleChanged, "desktop", &anchors, Some("color"));
        assert_ne!(id1, id2, "issue id must differ when property differs");
    }

    // -----------------------------------------------------------------------
    // Ancestor channel tests
    // -----------------------------------------------------------------------

    fn make_anc_desc(id: &str, tag: &str, nearest_heading: Option<&str>) -> AncestorDescriptor {
        AncestorDescriptor {
            id: id.to_string(),
            tag: tag.to_string(),
            bbox: [0, 0, 1440, 720],
            depth: 3,
            css_selector: Some(format!("main > {}", tag)),
            anchors: Anchors {
                text: None,
                role: None,
                href: None,
                alt: None,
                aria_label: None,
                nearest_heading: nearest_heading.map(str::to_string),
                landmark: Some("main".to_string()),
                ordinal_in_landmark: Some(1),
            },
        }
    }

    #[test]
    fn test_ancestor_css_only_change_1to1_pair() {
        // Two pages with identical DOM: one matched node, one ancestor on each side.
        // Ancestor has a different background-color, but all other props are identical
        // (simulating a CSS-only change where only one property was edited).
        // sim = 8_equal/9_total + 0.05_tag_bonus = 0.944 > ANCESTOR_MIN_SIMILARITY=0.6 → pair accepted.
        let old_node = make_node("o1", Some("Text"), Some("Heading"));
        let new_node = make_node("n1", Some("Text"), Some("Heading"));

        let old_anc = make_anc_desc("anc_1", "div", Some("Heading"));
        let new_anc = make_anc_desc("anc_1", "div", Some("Heading"));

        // Many equal props plus one differing prop to exceed similarity floor
        let common_props: &[(&str, &str)] = &[
            ("color", "rgb(0, 0, 0)"),
            ("font-size", "16px"),
            ("font-weight", "400"),
            ("display", "block"),
            ("position", "relative"),
            ("opacity", "1"),
            ("padding-top", "0px"),
            ("padding-right", "0px"),
        ];
        let mut old_anc_styles = styles(common_props);
        old_anc_styles.insert(
            "background-color".to_string(),
            "rgb(255, 255, 255)".to_string(),
        );
        let mut new_anc_styles = styles(common_props);
        // Only background-color differs
        new_anc_styles.insert("background-color".to_string(), "rgb(0, 0, 255)".to_string());

        let mut old_cs: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        old_cs.insert("o1".to_string(), styles(&[("color", "rgb(0, 0, 0)")]));
        old_cs.insert("anc_1".to_string(), old_anc_styles);
        let mut new_cs: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        new_cs.insert("n1".to_string(), styles(&[("color", "rgb(0, 0, 0)")]));
        new_cs.insert("anc_1".to_string(), new_anc_styles);

        let mut old_chains = BTreeMap::new();
        old_chains.insert("o1".to_string(), vec!["anc_1".to_string()]);
        let mut new_chains = BTreeMap::new();
        new_chains.insert("n1".to_string(), vec!["anc_1".to_string()]);

        let old_candidates = StyleCandidates {
            ancestors: vec![old_anc],
            chains: old_chains,
            budget: 2000,
            truncated: false,
            dropped_count: 0,
        };
        let new_candidates = StyleCandidates {
            ancestors: vec![new_anc],
            chains: new_chains,
            budget: 2000,
            truncated: false,
            dropped_count: 0,
        };

        let old_b =
            make_bundle_with_candidates("http://old.com/", vec![old_node], old_cs, old_candidates);
        let new_b =
            make_bundle_with_candidates("http://new.com/", vec![new_node], new_cs, new_candidates);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);

        assert!(
            issues
                .iter()
                .any(|i| i.issue_type == IssueType::StyleChanged),
            "ancestor CSS change should produce style_changed"
        );
    }

    #[test]
    fn test_ancestor_inserted_wrapper_stays_silent() {
        // New side has an extra ancestor wrapper (different id, different descendants).
        // The unpaired ancestor should NOT emit any issue.
        let old_node = make_node("o1", Some("Text"), Some("Heading"));
        let new_node = make_node("n1", Some("Text"), Some("Heading"));

        // Old: just one ancestor
        let old_anc = make_anc_desc("anc_1", "div", Some("Heading"));
        // New: two ancestors — anc_1 is the same container, anc_2 is a new wrapper
        let new_anc1 = make_anc_desc("anc_1", "div", Some("Heading"));
        let new_anc2 = make_anc_desc("anc_2", "section", Some("Heading"));

        let mut old_cs: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        old_cs.insert("o1".to_string(), styles(&[("color", "rgb(0,0,0)")]));
        old_cs.insert(
            "anc_1".to_string(),
            styles(&[("background-color", "rgb(255,255,255)")]),
        );
        let mut new_cs: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        new_cs.insert("n1".to_string(), styles(&[("color", "rgb(0,0,0)")]));
        new_cs.insert(
            "anc_1".to_string(),
            styles(&[("background-color", "rgb(255,255,255)")]),
        );
        new_cs.insert(
            "anc_2".to_string(),
            styles(&[("background-color", "rgb(100,0,0)")]),
        );

        let mut old_chains = BTreeMap::new();
        old_chains.insert("o1".to_string(), vec!["anc_1".to_string()]);
        let mut new_chains = BTreeMap::new();
        // New node's chain includes both ancestors
        new_chains.insert(
            "n1".to_string(),
            vec!["anc_1".to_string(), "anc_2".to_string()],
        );

        let old_cands = StyleCandidates {
            ancestors: vec![old_anc],
            chains: old_chains,
            budget: 2000,
            truncated: false,
            dropped_count: 0,
        };
        let new_cands = StyleCandidates {
            ancestors: vec![new_anc1, new_anc2],
            chains: new_chains,
            budget: 2000,
            truncated: false,
            dropped_count: 0,
        };

        let old_b =
            make_bundle_with_candidates("http://old.com/", vec![old_node], old_cs, old_cands);
        let new_b =
            make_bundle_with_candidates("http://new.com/", vec![new_node], new_cs, new_cands);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);

        // anc_2 on new side is not matched to anything on old (different descendant sets) → silent
        // anc_1 is matched to anc_1 but has equal styles → silent too
        let style_issues_count = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::StyleChanged)
            .count();
        assert_eq!(
            style_issues_count, 0,
            "inserted wrapper ancestor with no old match should be silent"
        );
    }

    #[test]
    fn test_ancestor_similarity_floor_rejects_misaligned() {
        // Two ancestors share a descendant set key but differ in almost all properties.
        let old_node = make_node("o1", Some("Text"), Some("Heading"));
        let new_node = make_node("n1", Some("Text"), Some("Heading"));

        let old_anc = make_anc_desc("anc_1", "div", Some("Heading"));
        let new_anc = make_anc_desc("anc_2", "span", Some("Heading")); // Different tag + very different styles

        let mut old_cs: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        old_cs.insert("o1".to_string(), styles(&[("color", "rgb(0,0,0)")]));
        // Old ancestor: many properties with unique values
        old_cs.insert(
            "anc_1".to_string(),
            styles(&[
                ("color", "rgb(255,0,0)"),
                ("background-color", "rgb(0,0,255)"),
                ("font-size", "10px"),
                ("display", "block"),
                ("position", "absolute"),
                ("opacity", "0.1"),
            ]),
        );
        let mut new_cs: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        new_cs.insert("n1".to_string(), styles(&[("color", "rgb(0,0,0)")]));
        // New ancestor: all values completely different
        new_cs.insert(
            "anc_2".to_string(),
            styles(&[
                ("color", "rgb(0,255,0)"),
                ("background-color", "rgb(255,255,0)"),
                ("font-size", "100px"),
                ("display", "flex"),
                ("position", "relative"),
                ("opacity", "0.9"),
            ]),
        );

        let mut old_chains = BTreeMap::new();
        old_chains.insert("o1".to_string(), vec!["anc_1".to_string()]);
        let mut new_chains = BTreeMap::new();
        new_chains.insert("n1".to_string(), vec!["anc_2".to_string()]);

        let old_cands = StyleCandidates {
            ancestors: vec![old_anc],
            chains: old_chains,
            budget: 2000,
            truncated: false,
            dropped_count: 0,
        };
        let new_cands = StyleCandidates {
            ancestors: vec![new_anc],
            chains: new_chains,
            budget: 2000,
            truncated: false,
            dropped_count: 0,
        };

        let old_b =
            make_bundle_with_candidates("http://old.com/", vec![old_node], old_cs, old_cands);
        let new_b =
            make_bundle_with_candidates("http://new.com/", vec![new_node], new_cs, new_cands);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);

        // Similarity is low (0 equal props, different tags) → below ANCESTOR_MIN_SIMILARITY=0.6
        // Should produce no issues
        assert!(
            issues.is_empty(),
            "misaligned ancestor pair below similarity floor should emit nothing"
        );
    }

    // -----------------------------------------------------------------------
    // Uncapped similarity / nested-wrapper crossing regression tests
    // -----------------------------------------------------------------------

    /// Build a full-property style map for an ancestor that shares `n_props` equal properties
    /// with `other_map`, but differs in exactly `n_diff` of them.  We use a fixed curated list
    /// so the fraction is predictable.
    fn make_anc_styles_full(position: &str) -> BTreeMap<String, String> {
        // 8 properties all identical except `position` so we control base ratio precisely.
        styles(&[
            ("color", "rgb(0, 0, 0)"),
            ("background-color", "rgb(255, 255, 255)"),
            ("font-size", "16px"),
            ("display", "block"),
            ("opacity", "1"),
            ("padding-top", "0px"),
            ("padding-right", "0px"),
            ("position", position),
        ])
    }

    #[test]
    fn test_ancestor_nested_wrapper_no_crossing_phantom_issues() {
        // Regression: two old ancestors A (position:static) and B (position:relative) share
        // the same descendant set as two new ancestors A' (position:static) and B' (position:relative).
        // Correct pairing: A↔A', B↔B' → zero issues (all styles identical after pairing).
        // With the old capped similarity A↔B' had sim = 7/8 + 0.05 = 0.9375 (same tag div),
        // and A↔A' had 8/8 + 0.05 = 1.05 → capped 1.0.  With cap, A↔B' also rounded to
        // 1.0 in sort, making them tied and allowing id-order to cross-pair → phantom issues.
        // With uncapped sim A↔A' = 1.05 > A↔B' = 0.9875 strictly, correct pairing is selected.
        let old_node = make_node("o1", Some("Text"), Some("Heading"));
        let new_node = make_node("n1", Some("Text"), Some("Heading"));

        // anc_1_old and anc_1_new have position:static (identical)
        // anc_2_old and anc_2_new have position:relative (identical)
        let old_anc_a = make_anc_desc("anc_1", "div", Some("Heading"));
        let old_anc_b = make_anc_desc("anc_2", "div", Some("Heading"));
        let new_anc_a = make_anc_desc("anc_1", "div", Some("Heading"));
        let new_anc_b = make_anc_desc("anc_2", "div", Some("Heading"));

        let mut old_cs: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        old_cs.insert("o1".to_string(), styles(&[("color", "rgb(0,0,0)")]));
        old_cs.insert("anc_1".to_string(), make_anc_styles_full("static"));
        old_cs.insert("anc_2".to_string(), make_anc_styles_full("relative"));

        let mut new_cs: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        new_cs.insert("n1".to_string(), styles(&[("color", "rgb(0,0,0)")]));
        new_cs.insert("anc_1".to_string(), make_anc_styles_full("static"));
        new_cs.insert("anc_2".to_string(), make_anc_styles_full("relative"));

        // Both ancestors share the same descendant set key (both have o1/n1 in their chains).
        let mut old_chains = BTreeMap::new();
        old_chains.insert(
            "o1".to_string(),
            vec!["anc_1".to_string(), "anc_2".to_string()],
        );
        let mut new_chains = BTreeMap::new();
        new_chains.insert(
            "n1".to_string(),
            vec!["anc_1".to_string(), "anc_2".to_string()],
        );

        let old_cands = StyleCandidates {
            ancestors: vec![old_anc_a, old_anc_b],
            chains: old_chains,
            budget: 2000,
            truncated: false,
            dropped_count: 0,
        };
        let new_cands = StyleCandidates {
            ancestors: vec![new_anc_a, new_anc_b],
            chains: new_chains,
            budget: 2000,
            truncated: false,
            dropped_count: 0,
        };

        let old_b =
            make_bundle_with_candidates("http://old.com/", vec![old_node], old_cs, old_cands);
        let new_b =
            make_bundle_with_candidates("http://new.com/", vec![new_node], new_cs, new_cands);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);

        // Correct pairing A↔A', B↔B' → both have identical styles → zero issues.
        let phantom_position_issues: Vec<_> = issues
            .iter()
            .filter(|i| {
                i.issue_type == IssueType::StyleChanged
                    && i.evidence
                        .get("old")
                        .and_then(|o| o.get("position"))
                        .is_some()
            })
            .collect();
        assert!(
            phantom_position_issues.is_empty(),
            "no phantom position issues should be emitted; got {} issues: {:?}",
            phantom_position_issues.len(),
            phantom_position_issues
                .iter()
                .map(|i| &i.id)
                .collect::<Vec<_>>()
        );
        assert!(
            issues.is_empty(),
            "perfectly-matched ancestor group must emit zero issues; got {}",
            issues.len()
        );
    }

    #[test]
    fn test_ancestor_evidence_score_capped_and_signals_shape() {
        // A perfectly matching pair (base 1.0, same tag → uncapped 1.05) must report
        // evidence.match.score = 1.0 (capped), and signals carry styleSim/tagBonus.
        let old_node = make_node("o1", Some("Text"), Some("Heading"));
        let new_node = make_node("n1", Some("Text"), Some("Heading"));

        let old_anc = make_anc_desc("anc_1", "div", Some("Heading"));
        let new_anc = make_anc_desc("anc_1", "div", Some("Heading"));

        // Styles differ on background-color so an issue IS emitted — we inspect its evidence.
        let common: &[(&str, &str)] = &[
            ("color", "rgb(0, 0, 0)"),
            ("font-size", "16px"),
            ("display", "block"),
            ("opacity", "1"),
            ("padding-top", "0px"),
            ("padding-right", "0px"),
            ("padding-bottom", "0px"),
            ("padding-left", "0px"),
        ];
        let mut old_anc_styles = styles(common);
        old_anc_styles.insert("background-color".to_string(), "rgb(255, 0, 0)".to_string());
        let mut new_anc_styles = styles(common);
        new_anc_styles.insert("background-color".to_string(), "rgb(0, 0, 255)".to_string());

        let mut old_cs: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        old_cs.insert("o1".to_string(), styles(&[("color", "rgb(0,0,0)")]));
        old_cs.insert("anc_1".to_string(), old_anc_styles);
        let mut new_cs: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        new_cs.insert("n1".to_string(), styles(&[("color", "rgb(0,0,0)")]));
        new_cs.insert("anc_1".to_string(), new_anc_styles);

        let mut old_chains = BTreeMap::new();
        old_chains.insert("o1".to_string(), vec!["anc_1".to_string()]);
        let mut new_chains = BTreeMap::new();
        new_chains.insert("n1".to_string(), vec!["anc_1".to_string()]);

        let old_cands = StyleCandidates {
            ancestors: vec![old_anc],
            chains: old_chains,
            budget: 2000,
            truncated: false,
            dropped_count: 0,
        };
        let new_cands = StyleCandidates {
            ancestors: vec![new_anc],
            chains: new_chains,
            budget: 2000,
            truncated: false,
            dropped_count: 0,
        };

        let old_b =
            make_bundle_with_candidates("http://old.com/", vec![old_node], old_cs, old_cands);
        let new_b =
            make_bundle_with_candidates("http://new.com/", vec![new_node], new_cs, new_cands);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            !issues.is_empty(),
            "background-color change must emit an issue"
        );

        let issue = &issues[0];
        let match_ev = issue
            .evidence
            .get("match")
            .expect("evidence.match must exist");

        let score = match_ev
            .get("score")
            .and_then(|v| v.as_f64())
            .expect("evidence.match.score must be numeric");
        assert!(
            score <= 1.0,
            "evidence.match.score must be capped at 1.0, got {}",
            score
        );

        let signals = match_ev
            .get("signals")
            .expect("evidence.match.signals must exist");
        assert!(
            signals.get("styleSim").is_some(),
            "signals must contain styleSim"
        );
        assert!(
            signals.get("tagBonus").is_some(),
            "signals must contain tagBonus"
        );
        let style_sim = signals["styleSim"].as_f64().unwrap();
        let tag_bonus = signals["tagBonus"].as_f64().unwrap();
        // 8 props present, 8 equal (background-color differs but we check the SIM which
        // includes background-color as a diff → 8 equal out of 9 present = 0.8888)
        // Actually: 8 common + 1 differing background-color = 9 present, 8 equal.
        assert!(
            (0.0..=1.0).contains(&style_sim),
            "styleSim must be in [0,1], got {}",
            style_sim
        );
        assert!(
            tag_bonus == 0.0 || tag_bonus == 0.05,
            "tagBonus must be 0.0 or 0.05, got {}",
            tag_bonus
        );
        // Same tag → bonus must be 0.05
        assert_eq!(tag_bonus, 0.05, "same-tag pair must have tagBonus=0.05");
    }

    // -----------------------------------------------------------------------
    // Determinism test
    // -----------------------------------------------------------------------

    #[test]
    fn test_determinism_same_bundles_byte_identical() {
        let old_node = make_node("o1", Some("Text"), Some("Heading"));
        let new_node = make_node("n1", Some("Text"), Some("Heading"));

        let old_cs = styles(&[
            ("font-size", "60px"),
            ("background-color", "rgb(255, 0, 0)"),
        ]);
        let new_cs = styles(&[
            ("font-size", "48px"),
            ("background-color", "rgb(0, 0, 255)"),
        ]);

        let mut old_styles = BTreeMap::new();
        old_styles.insert("o1".to_string(), old_cs);
        let mut new_styles = BTreeMap::new();
        new_styles.insert("n1".to_string(), new_cs);

        let old_b = make_bundle(
            "http://old.com/",
            vec![old_node.clone()],
            old_styles.clone(),
        );
        let new_b = make_bundle(
            "http://new.com/",
            vec![new_node.clone()],
            new_styles.clone(),
        );
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues1 = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        let issues2 = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);

        let json1 = serde_json::to_string(&issues1).unwrap();
        let json2 = serde_json::to_string(&issues2).unwrap();
        assert_eq!(json1, json2, "style_issues must be byte-deterministic");
    }

    #[test]
    fn test_uncertain_pair_skipped() {
        // WP-E: Uncertain-band pairs emit style issues at Info severity (not skipped).
        // Evidence includes uncertainPairing:true; issues are excluded from style score.
        let old_node = make_node("o1", Some("Text"), None);
        let new_node = make_node("n1", Some("Text"), None);

        let old_cs = styles(&[("font-size", "60px")]);
        let new_cs = styles(&[("font-size", "48px")]);

        let mut old_styles = BTreeMap::new();
        old_styles.insert("o1".to_string(), old_cs);
        let mut new_styles = BTreeMap::new();
        new_styles.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://old.com/", vec![old_node], old_styles);
        let new_b = make_bundle("http://new.com/", vec![new_node], new_styles);

        // Uncertain band pair
        let mut signals = BTreeMap::new();
        signals.insert("text".to_string(), 0.6_f64);
        let uncertain_pair = MatchedPair {
            old_idx: 0,
            new_idx: 0,
            score: 0.6,
            stage: MatchStage::Assignment,
            band: MatchBand::Uncertain,
            signals,
        };
        let outcome = make_outcome(vec![uncertain_pair]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        // WP-E behavior: issues ARE emitted but at Info severity.
        assert!(
            !issues.is_empty(),
            "uncertain-band pairs must produce Info-severity style issues (not skipped)"
        );
        for issue in &issues {
            assert_eq!(
                issue.severity,
                crate::contract::IssueSeverity::Info,
                "uncertain-band style issues must be Info severity"
            );
            // Evidence must carry uncertainPairing: true
            assert_eq!(
                issue.evidence["match"]["uncertainPairing"],
                serde_json::Value::Bool(true),
                "uncertain-band style issues must carry uncertainPairing:true in evidence"
            );
        }
    }

    // -----------------------------------------------------------------------
    // url() origin normalization tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_origin_http_default_port() {
        // Default port 80 must be suppressed
        assert_eq!(
            extract_origin("http://localhost:80/path"),
            "http://localhost"
        );
        assert_eq!(extract_origin("http://localhost/path"), "http://localhost");
    }

    #[test]
    fn test_extract_origin_https_default_port() {
        assert_eq!(
            extract_origin("https://example.com:443/"),
            "https://example.com"
        );
        assert_eq!(
            extract_origin("https://example.com/"),
            "https://example.com"
        );
    }

    #[test]
    fn test_extract_origin_non_default_port() {
        assert_eq!(
            extract_origin("http://localhost:3000/"),
            "http://localhost:3000"
        );
        assert_eq!(
            extract_origin("http://localhost:3001/page"),
            "http://localhost:3001"
        );
    }

    #[test]
    fn test_extract_origin_cdn() {
        assert_eq!(
            extract_origin("https://cdn.example.com/img/x.svg"),
            "https://cdn.example.com"
        );
    }

    #[test]
    fn test_normalize_url_origins_double_quoted() {
        // url("http://localhost:3000/assets/x.svg") on root-mounted page → url("assets/x.svg")
        // Leading slash stripped because page dir is "/" and asset starts with "/".
        let val = r#"url("http://localhost:3000/assets/x.svg")"#;
        let result = normalize_url_origins(val, "http://localhost:3000/");
        assert_eq!(result, r#"url("assets/x.svg")"#);
    }

    #[test]
    fn test_normalize_url_origins_single_quoted() {
        let val = "url('http://localhost:3001/img/logo.png')";
        let result = normalize_url_origins(val, "http://localhost:3001/");
        assert_eq!(result, "url('img/logo.png')");
    }

    #[test]
    fn test_normalize_url_origins_unquoted() {
        let val = "url(http://localhost:3000/bg.jpg)";
        let result = normalize_url_origins(val, "http://localhost:3000/");
        assert_eq!(result, "url(bg.jpg)");
    }

    #[test]
    fn test_normalize_url_origins_third_party_untouched() {
        // Third-party URL should NOT be stripped
        let val = r#"url("https://cdn.example.com/logo.svg")"#;
        let result = normalize_url_origins(val, "http://localhost:3000/");
        assert_eq!(result, val, "third-party URL must not be modified");
    }

    #[test]
    fn test_normalize_url_origins_third_party_differs_emits_issue() {
        // Two different third-party urls → not equal after normalization → issue emitted
        let old_node = make_node("o1", Some("Img"), None);
        let new_node = make_node("n1", Some("Img"), None);

        let old_cs = styles(&[(
            "background-image",
            r#"url("https://cdn.example.com/v1/hero.svg")"#,
        )]);
        let new_cs = styles(&[(
            "background-image",
            r#"url("https://cdn.example.com/v2/hero.svg")"#,
        )]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        // Both pages on :3000 / :3001, CDN is a separate origin
        let old_b = make_bundle("http://localhost:3000/", vec![old_node], old_styles_map);
        let new_b = make_bundle("http://localhost:3001/", vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            !issues.is_empty(),
            "differing third-party URLs must still emit an issue"
        );
    }

    #[test]
    fn test_same_path_different_origin_ports_equal_after_normalization() {
        // Old: url("http://localhost:3000/assets/bg.svg") on :3000 page
        // New: url("http://localhost:3001/assets/bg.svg") on :3001 page
        // Both strip to /assets/bg.svg → equal → no issue
        let old_node = make_node("o1", Some("Hero"), None);
        let new_node = make_node("n1", Some("Hero"), None);

        let old_cs = styles(&[(
            "background-image",
            r#"url("http://localhost:3000/assets/bg.svg")"#,
        )]);
        let new_cs = styles(&[(
            "background-image",
            r#"url("http://localhost:3001/assets/bg.svg")"#,
        )]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://localhost:3000/", vec![old_node], old_styles_map);
        let new_b = make_bundle("http://localhost:3001/", vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues.is_empty(),
            "same-path url() differing only in localhost port must not emit an issue"
        );
    }

    #[test]
    fn test_mixed_value_url_and_gradient_normalization() {
        // Value: "url('http://localhost:3000/bg.svg'), linear-gradient(red, blue)"
        // On old side origin :3000 — url() stripped, gradient untouched.
        // On new side same value but :3001 — url() stripped, gradient untouched.
        // Result: both normalize to "url('/bg.svg'), linear-gradient(red, blue)" → equal → no issue.
        let old_node = make_node("o1", Some("Hero"), None);
        let new_node = make_node("n1", Some("Hero"), None);

        let old_val = "url('http://localhost:3000/bg.svg'), linear-gradient(red, blue)";
        let new_val = "url('http://localhost:3001/bg.svg'), linear-gradient(red, blue)";

        let old_cs = styles(&[("background-image", old_val)]);
        let new_cs = styles(&[("background-image", new_val)]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://localhost:3000/", vec![old_node], old_styles_map);
        let new_b = make_bundle("http://localhost:3001/", vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues.is_empty(),
            "same-origin url() plus identical gradient must be equal after normalization"
        );
    }

    #[test]
    fn test_url_only_old_gradient_new_emits_gradient_lost() {
        // Old side: a gradient. New side: a same-origin url() which normalizes to a path.
        // After normalization: old has gradient, new has no gradient → background_gradient_lost.
        let old_node = make_node("o1", Some("Hero"), Some("Heading"));
        let new_node = make_node("n1", Some("Hero"), Some("Heading"));

        let old_cs = styles(&[(
            "background-image",
            "linear-gradient(90deg, #6d28d9 0%, #2563eb 100%)",
        )]);
        // new side: same-origin url() → normalizes to url(/assets/bg.svg) — no gradient
        let new_cs = styles(&[(
            "background-image",
            r#"url("http://localhost:3001/assets/bg.svg")"#,
        )]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://localhost:3000/", vec![old_node], old_styles_map);
        let new_b = make_bundle("http://localhost:3001/", vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues
                .iter()
                .any(|i| i.issue_type == IssueType::BackgroundGradientLost),
            "gradient on old, url() on new → background_gradient_lost"
        );
    }

    #[test]
    fn test_evidence_contains_raw_values_for_genuine_url_change() {
        // Genuine URL change: old and new have DIFFERENT paths → issue with raw captured values in evidence
        let old_node = make_node("o1", Some("Banner"), None);
        let new_node = make_node("n1", Some("Banner"), None);

        // Same origin (:3000 / :3001) but different asset paths
        let old_cs = styles(&[(
            "background-image",
            r#"url("http://localhost:3000/assets/hero-v1.svg")"#,
        )]);
        let new_cs = styles(&[(
            "background-image",
            r#"url("http://localhost:3001/assets/hero-v2.svg")"#, // different path
        )]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://localhost:3000/", vec![old_node], old_styles_map);
        let new_b = make_bundle("http://localhost:3001/", vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(!issues.is_empty(), "different paths must emit an issue");

        // Evidence must contain the RAW (origin-bearing) values
        let issue = &issues[0];
        let old_ev = issue
            .evidence
            .get("old")
            .and_then(|o| o.get("background-image"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let new_ev = issue
            .evidence
            .get("new")
            .and_then(|o| o.get("background-image"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            old_ev.contains("localhost:3000"),
            "evidence.old must contain the raw (origin-bearing) value, got: {}",
            old_ev
        );
        assert!(
            new_ev.contains("localhost:3001"),
            "evidence.new must contain the raw (origin-bearing) value, got: {}",
            new_ev
        );
    }

    // -----------------------------------------------------------------------
    // norm_href-based url() normalization tests (v14 prefix-mount scenario)
    // -----------------------------------------------------------------------

    #[test]
    fn test_url_normalization_prefix_mounted_pages_equal() {
        // Real v14 shape: the new page is prefix-mounted, so computed CSS resolves the
        // asset URL with the page-directory prefix included.
        //   old page http://localhost:3000/  →  url("http://localhost:3000/assets/x.svg")
        //   new page http://localhost:3014/products/connect/branded-call/
        //                                   →  url("http://localhost:3014/products/connect/branded-call/assets/x.svg")
        // After normalization both sides produce "assets/x.svg" → no issue.
        let old_node = make_node("o1", Some("Hero"), None);
        let new_node = make_node("n1", Some("Hero"), None);

        let old_cs = styles(&[(
            "background-image",
            r#"url("http://localhost:3000/assets/x.svg")"#,
        )]);
        // New side: browser resolves relative asset against the prefix-mounted page dir.
        let new_cs = styles(&[(
            "background-image",
            r#"url("http://localhost:3014/products/connect/branded-call/assets/x.svg")"#,
        )]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://localhost:3000/", vec![old_node], old_styles_map);
        let new_b = make_bundle(
            "http://localhost:3014/products/connect/branded-call/",
            vec![new_node],
            new_styles_map,
        );
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues.is_empty(),
            "prefix-mounted pages with same asset path must not emit a false style_changed"
        );
    }

    #[test]
    fn test_url_normalization_same_origin_asset_not_under_page_dir_kept_absolute() {
        // An asset that is NOT under either page's directory (root-anchored, e.g. /shared/icon.svg
        // while both pages live under /section/page/).  norm_href strips the origin giving
        // /shared/icon.svg on both sides; our page-dir strip does not apply (asset not under dir)
        // so the absolute path /shared/icon.svg is kept.  Both sides produce the same absolute
        // path → no issue.  This documents the "keep absolute" branch.
        let asset_old = "http://localhost:3000/shared/icon.svg";
        let asset_new = "http://localhost:3001/shared/icon.svg";
        let page_old = "http://localhost:3000/section/page/";
        let page_new = "http://localhost:3001/section/page/";

        // Confirm norm_href produces the same absolute path for both sides.
        let normed_old = norm_href(asset_old, page_old);
        let normed_new = norm_href(asset_new, page_new);
        assert_eq!(
            normed_old, normed_new,
            "norm_href must produce the same absolute path for same-site root-anchored asset"
        );
        // The asset is NOT under the page dir, so the absolute path is preserved.
        assert_eq!(normed_old, "/shared/icon.svg");

        // End-to-end: both sides have the same root-anchored asset → no issue.
        let old_node = make_node("o1", Some("Icon"), None);
        let new_node = make_node("n1", Some("Icon"), None);

        let old_val = format!(r#"url("{}")"#, asset_old);
        let new_val = format!(r#"url("{}")"#, asset_new);
        let old_cs = styles(&[("background-image", old_val.as_str())]);
        let new_cs = styles(&[("background-image", new_val.as_str())]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle(page_old, vec![old_node], old_styles_map);
        let new_b = make_bundle(page_new, vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues.is_empty(),
            "same root-anchored asset on both sides must not emit an issue"
        );
    }

    #[test]
    fn test_url_normalization_third_origin_byte_compared() {
        // A CDN url() on both sides: unchanged by norm_href (external).
        // Same CDN URL → no issue; different CDN URLs → issue.
        let old_node_same = make_node("o1", Some("A"), None);
        let new_node_same = make_node("n1", Some("A"), None);
        let old_node_diff = make_node("o2", Some("B"), None);
        let new_node_diff = make_node("n2", Some("B"), None);

        let same_cdn = r#"url("https://cdn.example.com/logo.svg")"#;
        let old_cdn_diff = r#"url("https://cdn.example.com/v1/logo.svg")"#;
        let new_cdn_diff = r#"url("https://cdn.example.com/v2/logo.svg")"#;

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), styles(&[("background-image", same_cdn)]));
        old_styles_map.insert(
            "o2".to_string(),
            styles(&[("background-image", old_cdn_diff)]),
        );
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), styles(&[("background-image", same_cdn)]));
        new_styles_map.insert(
            "n2".to_string(),
            styles(&[("background-image", new_cdn_diff)]),
        );

        let mut old_nodes_seq = make_node("o1", Some("A"), None);
        old_nodes_seq.seq_index = 0;
        let mut new_nodes_seq = make_node("n1", Some("A"), None);
        new_nodes_seq.seq_index = 0;
        let mut old_node_diff2 = make_node("o2", Some("B"), None);
        old_node_diff2.seq_index = 1;
        let mut new_node_diff2 = make_node("n2", Some("B"), None);
        new_node_diff2.seq_index = 1;

        let _ = (old_node_same, new_node_same, old_node_diff, new_node_diff);

        let old_b = make_bundle(
            "http://localhost:3000/",
            vec![old_nodes_seq.clone(), old_node_diff2.clone()],
            old_styles_map,
        );
        let new_b = make_bundle(
            "http://localhost:3001/",
            vec![new_nodes_seq.clone(), new_node_diff2.clone()],
            new_styles_map,
        );
        let outcome = make_outcome(vec![make_matched_pair(0, 0), make_matched_pair(1, 1)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);

        // o1/n1: same CDN URL → no issue
        let same_cdn_issue = issues.iter().any(|i| {
            i.issue_type == IssueType::StyleChanged
                && i.evidence
                    .get("old")
                    .and_then(|o| o.get("background-image"))
                    .and_then(|v| v.as_str())
                    .map(|s| s == same_cdn)
                    .unwrap_or(false)
        });
        assert!(
            !same_cdn_issue,
            "identical third-party CDN URL must not emit an issue"
        );

        // o2/n2: different CDN URLs → issue must be emitted
        let diff_cdn_issue = issues.iter().any(|i| {
            i.issue_type == IssueType::StyleChanged
                && i.evidence
                    .get("old")
                    .and_then(|o| o.get("background-image"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.contains("v1"))
                    .unwrap_or(false)
        });
        assert!(
            diff_cdn_issue,
            "differing third-party CDN URLs must emit an issue"
        );
    }

    #[test]
    fn test_v14_exact_literal_values_no_false_positive() {
        // Exact v14-trailing-slash values that were producing false style_changed issues.
        // Old page finalUrl:  http://localhost:3000/
        //   value: url("http://localhost:3000/assets/images/6765be588915fa7e814d2472_custom-bullet.svg")
        // New page finalUrl:  http://localhost:3014/products/connect/branded-call/
        //   value: url("http://localhost:3014/products/connect/branded-call/assets/images/6765be588915fa7e814d2472_custom-bullet.svg")
        // Both must normalize to the same page-relative form → no issue.
        let old_val = r#"url("http://localhost:3000/assets/images/6765be588915fa7e814d2472_custom-bullet.svg")"#;
        let new_val = r#"url("http://localhost:3014/products/connect/branded-call/assets/images/6765be588915fa7e814d2472_custom-bullet.svg")"#;
        let old_page = "http://localhost:3000/";
        let new_page = "http://localhost:3014/products/connect/branded-call/";

        // Direct string-equality assertion on the normalized forms.
        let normed_old = normalize_url_origins(old_val, old_page);
        let normed_new = normalize_url_origins(new_val, new_page);
        assert_eq!(
            normed_old, normed_new,
            "v14 exact values must normalize to the same string; old={:?} new={:?}",
            normed_old, normed_new
        );

        // End-to-end: no style_changed issue emitted.
        let old_node = make_node("o1", Some("Bullet"), None);
        let new_node = make_node("n1", Some("Bullet"), None);

        let old_cs = styles(&[("background-image", old_val)]);
        let new_cs = styles(&[("background-image", new_val)]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle(old_page, vec![old_node], old_styles_map);
        let new_b = make_bundle(new_page, vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues.is_empty(),
            "v14 exact literal values must not emit a false style_changed (got {} issues)",
            issues.len()
        );
    }

    // -----------------------------------------------------------------------
    // C2: sub-pixel numeric epsilon unit tests (M6 calibration)
    // -----------------------------------------------------------------------

    /// C2-a: 19.5776px vs 19.6px — sub-pixel jitter → equal.
    #[test]
    fn test_c2_subpixel_line_height_equal() {
        assert!(
            values_equal_c2("19.5776px", "19.6px"),
            "19.5776px vs 19.6px must be equal under C2 epsilon"
        );
    }

    /// C2-b: 13.984px vs 14px — sub-pixel jitter → equal.
    #[test]
    fn test_c2_subpixel_font_size_equal() {
        assert!(
            values_equal_c2("13.984px", "14px"),
            "13.984px vs 14px must be equal under C2 epsilon"
        );
    }

    /// C2-c: 13px vs 14px — diff 1.0 >= epsilon 0.1 → NOT equal.
    #[test]
    fn test_c2_one_px_diff_not_equal() {
        assert!(
            !values_equal_c2("13px", "14px"),
            "13px vs 14px must NOT be equal under C2 epsilon (diff 1.0 >= 0.1)"
        );
    }

    /// C2-d: "0px none rgb(0, 0, 0)" vs "0px none rgb(80, 93, 111)" — rgb channel differs by >epsilon.
    #[test]
    fn test_c2_rgb_channel_diff_not_equal() {
        assert!(
            !values_equal_c2("0px none rgb(0, 0, 0)", "0px none rgb(80, 93, 111)"),
            "rgb channel diff >epsilon must not be suppressed"
        );
    }

    /// C2-e: values with differing non-numeric text → not equal.
    #[test]
    fn test_c2_mixed_text_differing_not_equal() {
        assert!(
            !values_equal_c2("solid 1px red", "dashed 1px red"),
            "differing non-numeric tokens must not be equal"
        );
    }

    /// C2-f: end-to-end: sub-pixel line-height jitter must NOT emit style_changed.
    #[test]
    fn test_c2_end_to_end_subpixel_suppressed() {
        let old_node = make_node("o1", Some("Text"), None);
        let new_node = make_node("n1", Some("Text"), None);

        let old_cs = styles(&[("line-height", "19.6px")]);
        let new_cs = styles(&[("line-height", "19.5776px")]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://localhost:3000/", vec![old_node], old_styles_map);
        let new_b = make_bundle("http://localhost:3001/", vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues.is_empty(),
            "sub-pixel line-height jitter must not emit style_changed, got {} issues",
            issues.len()
        );
    }

    // -----------------------------------------------------------------------
    // C3: url() filename-tail comparison unit tests (M6 calibration)
    // -----------------------------------------------------------------------

    /// C3-a: CDN vs localhost, same filename → equal.
    #[test]
    fn test_c3_cdn_vs_localhost_same_filename_equal() {
        let cdn = r#"url("https://cdn.prod.website-files.com/abc123/images/icon.svg")"#;
        let local = r#"url("http://localhost:3000/assets/images/icon.svg")"#;
        assert!(
            values_equal_c3(cdn, local),
            "same filename on different hosts must be equal under C3"
        );
    }

    /// C3-b: same hosts but different filenames → not equal.
    #[test]
    fn test_c3_same_host_different_filename_not_equal() {
        let a = r#"url("http://localhost:3000/assets/icon-a.svg")"#;
        let b = r#"url("http://localhost:3000/assets/icon-b.svg")"#;
        assert!(
            !values_equal_c3(a, b),
            "different filenames must not be equal under C3"
        );
    }

    /// C3-c: gradient values without url() are unaffected (C3 returns false, falls through).
    #[test]
    fn test_c3_gradient_without_url_not_equal() {
        let a = "linear-gradient(red, blue)";
        let b = "linear-gradient(green, yellow)";
        assert!(
            !values_equal_c3(a, b),
            "gradient values without url() must not be equal under C3"
        );
    }

    /// C3-d: mixed value where non-url parts differ → not equal.
    #[test]
    fn test_c3_mixed_non_url_parts_differ_not_equal() {
        // Both have same url filename but the non-url part differs ("no-repeat" vs "repeat")
        let a = r#"no-repeat url("https://cdn.prod.website-files.com/a/icon.svg")"#;
        let b = r#"repeat url("http://localhost:3000/assets/icon.svg")"#;
        assert!(
            !values_equal_c3(a, b),
            "differing non-url parts must not be equal under C3"
        );
    }

    /// C3-f: relative (own-origin stripped) vs absolute CDN, same filename → equal.
    /// This is the primary observed R3 flood: normalize_url_origins strips the old
    /// side's own-origin url() to a relative form; the new side keeps a CDN absolute url().
    #[test]
    fn test_c3_relative_vs_absolute_cdn_same_filename_equal() {
        // Old side: own-origin stripped to relative by normalize_url_origins.
        // New side: CDN absolute URL (different host, not the page's origin).
        let relative = r#"url("assets/images/x.avif")"#;
        let cdn = r#"url("https://cdn.prod.website-files.com/abc123/images/x.avif")"#;
        assert!(
            values_equal_c3(relative, cdn),
            "relative (own-origin stripped) vs CDN absolute, same filename must be equal under C3"
        );
        // Symmetric: cdn vs relative must also be equal.
        assert!(
            values_equal_c3(cdn, relative),
            "CDN absolute vs relative (own-origin stripped), same filename must be equal under C3 (symmetric)"
        );
    }

    /// C3-g: both hostless relative urls with different paths → NOT equal (author-controlled change).
    #[test]
    fn test_c3_both_relative_different_paths_not_equal() {
        let a = r#"url("assets/a.svg")"#;
        let b = r#"url("images/a.svg")"#;
        assert!(
            !values_equal_c3(a, b),
            "both-hostless relative urls must NOT be equal under C3 (author-controlled path change)"
        );
    }

    /// C3-e: end-to-end: two different CDN hosts serving the same filename must NOT emit
    /// style_changed. Both URLs are external (neither is the page's own origin), so
    /// normalize_url_origins doesn't strip them — C3 sees full absolute URLs from different hosts
    /// with the same filename.
    #[test]
    fn test_c3_end_to_end_cross_cdn_same_filename_suppressed() {
        let old_node = make_node("o1", Some("Hero"), None);
        let new_node = make_node("n1", Some("Hero"), None);

        // Both URLs are on different CDN hosts (neither is the page's origin) → external,
        // not stripped by normalize_url_origins. C3 sees different hosts, same filename.
        let old_cs = styles(&[(
            "background-image",
            r#"url("https://cdn.prod.website-files.com/abc123/images/icon.svg")"#,
        )]);
        let new_cs = styles(&[(
            "background-image",
            r#"url("https://assets.example-cdn.net/hash456/images/icon.svg")"#,
        )]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://localhost:3000/", vec![old_node], old_styles_map);
        let new_b = make_bundle("http://localhost:3001/", vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues.is_empty(),
            "cross-CDN same-filename must not emit style_changed, got {} issues",
            issues.len()
        );
    }

    // -----------------------------------------------------------------------
    // C1 v05-regression: dup-label node inside a link STILL emits style_changed
    // when its computed styles differ (the label is not suppressed from style_diff).
    // -----------------------------------------------------------------------

    fn make_link_node(
        id: &str,
        text: Option<&str>,
        bbox: [i32; 4],
        seq_index: u32,
    ) -> SemanticNode {
        SemanticNode {
            id: id.to_string(),
            kind: "link".to_string(),
            role: None,
            text: text.map(str::to_string),
            acc_name: None,
            href: Some("/demo".to_string()),
            image_alt: None,
            bbox,
            seq_index,
            anchors: NodeAnchors {
                text: text.map(str::to_string),
                role: None,
                href: Some("/demo".to_string()),
                alt: None,
                aria_label: None,
                nearest_heading: None,
                landmark: Some("main".to_string()),
                ordinal_in_landmark: Some(1),
            },
            css_selector: Some(".button".to_string()),
            raw_href: None,
            src: None,
            natural_width: None,
            natural_height: None,
            loaded: None,
            heading_level: None,
        }
    }

    fn make_text_node_with_bbox(
        id: &str,
        text: Option<&str>,
        bbox: [i32; 4],
        seq_index: u32,
    ) -> SemanticNode {
        SemanticNode {
            id: id.to_string(),
            kind: "text".to_string(),
            role: None,
            text: text.map(str::to_string),
            acc_name: None,
            href: None,
            image_alt: None,
            bbox,
            seq_index,
            anchors: NodeAnchors {
                text: text.map(str::to_string),
                role: None,
                href: None,
                alt: None,
                aria_label: None,
                nearest_heading: None,
                landmark: Some("main".to_string()),
                ordinal_in_landmark: Some(1),
            },
            css_selector: Some(".button_content".to_string()),
            raw_href: None,
            src: None,
            natural_width: None,
            natural_height: None,
            loaded: None,
            heading_level: None,
        }
    }

    // -----------------------------------------------------------------------
    // C4: canonicalize_for_compare unit tests (WP-C)
    // -----------------------------------------------------------------------

    /// Rule 1: border with "none" style token — different colors, both invisible → equal.
    #[test]
    fn test_canon_border_none_style_invisible_both_sides_equal() {
        let a = canonicalize_for_compare("border", "0px none rgb(0, 0, 0)", None);
        let b = canonicalize_for_compare("border", "0px none rgb(38, 38, 38)", None);
        assert_eq!(a, "none");
        assert_eq!(b, "none");
        assert_eq!(a, b);
    }

    /// Rule 1: border with solid style — should NOT be collapsed.
    #[test]
    fn test_canon_border_solid_not_collapsed() {
        let a = canonicalize_for_compare("border", "2px solid rgb(0, 0, 0)", None);
        assert_eq!(a, "2px solid rgb(0, 0, 0)");
    }

    /// Rule 1: outline with none token → collapsed.
    #[test]
    fn test_canon_outline_none_collapsed() {
        let a = canonicalize_for_compare("outline", "0px none rgb(255, 255, 255)", None);
        assert_eq!(a, "none");
    }

    /// Rule 1: "none" must be a whole-token match — "noneblue" must not trigger.
    #[test]
    fn test_canon_border_partial_none_token_not_collapsed() {
        // Hypothetical value where "none" appears as part of a longer token — should NOT fire.
        let a = canonicalize_for_compare("border", "1px noneblue rgb(0,0,0)", None);
        assert_eq!(a, "1px noneblue rgb(0,0,0)");
    }

    /// Rule 2: text-align start → left.
    #[test]
    fn test_canon_text_align_start_to_left() {
        let a = canonicalize_for_compare("text-align", "start", None);
        assert_eq!(a, "left");
    }

    /// Rule 2: text-align end → right.
    #[test]
    fn test_canon_text_align_end_to_right() {
        let a = canonicalize_for_compare("text-align", "end", None);
        assert_eq!(a, "right");
    }

    /// Rule 2: text-align left → unchanged.
    #[test]
    fn test_canon_text_align_left_unchanged() {
        let a = canonicalize_for_compare("text-align", "left", None);
        assert_eq!(a, "left");
    }

    /// Rule 2: text-align center → unchanged.
    #[test]
    fn test_canon_text_align_center_unchanged() {
        let a = canonicalize_for_compare("text-align", "center", None);
        assert_eq!(a, "center");
    }

    /// Rule 3: line-height normal with font-size 18.84px → 22.6080px.
    #[test]
    fn test_canon_line_height_normal_resolves_with_font_size() {
        let a = canonicalize_for_compare("line-height", "normal", Some("18.84px"));
        // 18.84 * 1.2 = 22.608 → formatted as 4 decimals: "22.6080px"
        assert_eq!(a, "22.6080px");
    }

    /// Rule 3: line-height normal with font-size 16px → 19.2000px.
    #[test]
    fn test_canon_line_height_normal_font_size_16px() {
        let a = canonicalize_for_compare("line-height", "normal", Some("16px"));
        assert_eq!(a, "19.2000px");
    }

    /// Rule 3: line-height normal without font-size — unchanged.
    #[test]
    fn test_canon_line_height_normal_no_font_size_unchanged() {
        let a = canonicalize_for_compare("line-height", "normal", None);
        assert_eq!(a, "normal");
    }

    /// Rule 3: line-height explicit px — unchanged.
    #[test]
    fn test_canon_line_height_explicit_px_unchanged() {
        let a = canonicalize_for_compare("line-height", "22.6094px", Some("18.84px"));
        assert_eq!(a, "22.6094px");
    }

    /// Pass-through: non-canonicalized property (e.g. color) — unchanged.
    #[test]
    fn test_canon_passthrough_color_unchanged() {
        let a = canonicalize_for_compare("color", "rgb(1, 1, 1)", None);
        assert_eq!(a, "rgb(1, 1, 1)");
    }

    // -----------------------------------------------------------------------
    // C4: diff_styles end-to-end integration tests (WP-C)
    // -----------------------------------------------------------------------

    /// End-to-end: border "0px none <colorA>" vs "0px none <colorB>" → no issue.
    #[test]
    fn test_c4_border_none_suppressed_end_to_end() {
        let old_node = make_node("o1", Some("Text"), None);
        let new_node = make_node("n1", Some("Text"), None);

        let old_cs = styles(&[("border", "0px none rgb(0, 0, 0)")]);
        let new_cs = styles(&[("border", "0px none rgb(38, 38, 38)")]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://old.com/", vec![old_node], old_styles_map);
        let new_b = make_bundle("http://new.com/", vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues.is_empty(),
            "border 0px none with different colors must not emit an issue; got {} issues",
            issues.len()
        );
    }

    /// End-to-end: border "0px none ..." vs "2px solid ..." → issue IS emitted.
    #[test]
    fn test_c4_border_solid_vs_none_emits_issue() {
        let old_node = make_node("o1", Some("Text"), None);
        let new_node = make_node("n1", Some("Text"), None);

        let old_cs = styles(&[("border", "0px none rgb(255, 255, 255)")]);
        let new_cs = styles(&[("border", "2px solid rgb(0, 0, 0)")]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://old.com/", vec![old_node], old_styles_map);
        let new_b = make_bundle("http://new.com/", vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues
                .iter()
                .any(|i| i.issue_type == IssueType::StyleChanged),
            "border none vs solid must emit a style_changed issue"
        );
    }

    /// End-to-end: text-align start vs left → no issue.
    #[test]
    fn test_c4_text_align_start_vs_left_no_issue() {
        let old_node = make_node("o1", Some("Text"), None);
        let new_node = make_node("n1", Some("Text"), None);

        let old_cs = styles(&[("text-align", "start")]);
        let new_cs = styles(&[("text-align", "left")]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://old.com/", vec![old_node], old_styles_map);
        let new_b = make_bundle("http://new.com/", vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues.is_empty(),
            "text-align start vs left must not emit an issue (LTR equivalence)"
        );
    }

    /// End-to-end: text-align start vs center → issue IS emitted.
    #[test]
    fn test_c4_text_align_start_vs_center_emits_issue() {
        let old_node = make_node("o1", Some("Text"), None);
        let new_node = make_node("n1", Some("Text"), None);

        let old_cs = styles(&[("text-align", "start")]);
        let new_cs = styles(&[("text-align", "center")]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://old.com/", vec![old_node], old_styles_map);
        let new_b = make_bundle("http://new.com/", vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues
                .iter()
                .any(|i| i.issue_type == IssueType::StyleChanged),
            "text-align start vs center must emit a style_changed issue"
        );
    }

    /// End-to-end: text-align end vs right → no issue.
    #[test]
    fn test_c4_text_align_end_vs_right_no_issue() {
        let old_node = make_node("o1", Some("Text"), None);
        let new_node = make_node("n1", Some("Text"), None);

        let old_cs = styles(&[("text-align", "end")]);
        let new_cs = styles(&[("text-align", "right")]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://old.com/", vec![old_node], old_styles_map);
        let new_b = make_bundle("http://new.com/", vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues.is_empty(),
            "text-align end vs right must not emit an issue (LTR equivalence)"
        );
    }

    /// End-to-end: line-height normal (font-size 18.84px) vs 22.6094px → no issue.
    #[test]
    fn test_c4_line_height_normal_vs_computed_no_issue() {
        let old_node = make_node("o1", Some("Text"), None);
        let new_node = make_node("n1", Some("Text"), None);

        // Old side: computed px value (e.g. from Webflow); new side: "normal"
        let old_cs = styles(&[("line-height", "22.6094px"), ("font-size", "18.84px")]);
        let new_cs = styles(&[("line-height", "normal"), ("font-size", "18.84px")]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://old.com/", vec![old_node], old_styles_map);
        let new_b = make_bundle("http://new.com/", vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        // line-height issues only
        let lh_issues: Vec<_> = issues
            .iter()
            .filter(|i| {
                i.issue_type == IssueType::StyleChanged
                    && i.evidence
                        .get("old")
                        .and_then(|o| o.get("line-height"))
                        .is_some()
            })
            .collect();
        assert!(
            lh_issues.is_empty(),
            "line-height normal vs 22.6094px (font-size 18.84px) must not emit issue; got {:?}",
            lh_issues
        );
    }

    /// End-to-end: line-height normal (font-size 16px) vs 28px → issue IS emitted.
    #[test]
    fn test_c4_line_height_normal_vs_28px_emits_issue() {
        let old_node = make_node("o1", Some("Text"), None);
        let new_node = make_node("n1", Some("Text"), None);

        let old_cs = styles(&[("line-height", "normal"), ("font-size", "16px")]);
        let new_cs = styles(&[("line-height", "28px"), ("font-size", "16px")]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://old.com/", vec![old_node], old_styles_map);
        let new_b = make_bundle("http://new.com/", vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        let lh_issues: Vec<_> = issues
            .iter()
            .filter(|i| {
                i.issue_type == IssueType::StyleChanged
                    && i.evidence
                        .get("old")
                        .and_then(|o| o.get("line-height"))
                        .is_some()
            })
            .collect();
        assert!(
            !lh_issues.is_empty(),
            "line-height normal (16px * 1.2 = 19.2px) vs 28px must emit a style_changed issue"
        );
    }

    /// End-to-end: color change still emits an issue (non-canonicalized property).
    #[test]
    fn test_c4_non_canon_prop_still_emits() {
        let old_node = make_node("o1", Some("Text"), None);
        let new_node = make_node("n1", Some("Text"), None);

        let old_cs = styles(&[("color", "rgb(1, 1, 1)")]);
        let new_cs = styles(&[("color", "rgb(2, 2, 2)")]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://old.com/", vec![old_node], old_styles_map);
        let new_b = make_bundle("http://new.com/", vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            issues
                .iter()
                .any(|i| i.issue_type == IssueType::StyleChanged),
            "color change rgb(1,1,1) vs rgb(2,2,2) must still emit style_changed"
        );
    }

    /// End-to-end: emitted issue evidence still uses raw normalized values (not canonical).
    #[test]
    fn test_c4_evidence_uses_normalized_not_canonical_values() {
        // border none vs solid: issue emitted, evidence must show raw normalized values.
        let old_node = make_node("o1", Some("Text"), None);
        let new_node = make_node("n1", Some("Text"), None);

        let old_cs = styles(&[("border", "0px none rgb(255, 255, 255)")]);
        let new_cs = styles(&[("border", "2px solid rgb(0, 0, 0)")]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("o1".to_string(), old_cs);
        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("n1".to_string(), new_cs);

        let old_b = make_bundle("http://old.com/", vec![old_node], old_styles_map);
        let new_b = make_bundle("http://new.com/", vec![new_node], new_styles_map);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);
        assert!(
            !issues.is_empty(),
            "border none vs solid must emit an issue"
        );
        let issue = &issues[0];
        let old_ev = issue
            .evidence
            .get("old")
            .and_then(|o| o.get("border"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let new_ev = issue
            .evidence
            .get("new")
            .and_then(|o| o.get("border"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // Evidence must contain the raw (non-canonical) values
        assert!(
            old_ev.contains("none"),
            "evidence.old.border must contain the raw value (with 'none'), got: {}",
            old_ev
        );
        assert!(
            new_ev.contains("solid"),
            "evidence.new.border must contain the raw value (with 'solid'), got: {}",
            new_ev
        );
        // Must NOT contain the canonical form
        assert_ne!(
            old_ev, "none",
            "evidence.old.border must NOT be the collapsed canonical 'none'"
        );
    }

    /// v05 regression (C1): a `.button_content` label div is a dup-label nested inside the
    /// `<a class="button">` link. When computed styles on the label change (background-color),
    /// style_changed MUST still be emitted — the C1 suppression only affects missing_text
    /// emission in semantic_diff, NOT style_diff.
    #[test]
    fn test_c1_v05_regression_style_changed_still_emitted_for_dup_label() {
        // Old: link (parent) + text label (dup-label, nested inside link bbox)
        let old_link = make_link_node("old-link", Some("Get a Demo"), [100, 200, 200, 50], 0);
        let old_label =
            make_text_node_with_bbox("old-label", Some("Get a Demo"), [110, 210, 180, 30], 1);

        // New: same structure with same text
        let new_link = make_link_node("new-link", Some("Get a Demo"), [100, 200, 200, 50], 0);
        let new_label =
            make_text_node_with_bbox("new-label", Some("Get a Demo"), [110, 210, 180, 30], 1);

        // Old label has a specific background-color; new label has a changed one.
        let old_label_styles = styles(&[
            ("background-color", "rgb(79, 70, 229)"),
            ("padding", "12px 24px"),
        ]);
        let new_label_styles = styles(&[
            ("background-color", "rgb(37, 99, 235)"),
            ("padding", "12px 24px"),
        ]);

        let mut old_styles_map = BTreeMap::new();
        old_styles_map.insert("old-link".to_string(), styles(&[]));
        old_styles_map.insert("old-label".to_string(), old_label_styles);

        let mut new_styles_map = BTreeMap::new();
        new_styles_map.insert("new-link".to_string(), styles(&[]));
        new_styles_map.insert("new-label".to_string(), new_label_styles);

        let old_b = make_bundle(
            "http://localhost:3000/",
            vec![old_link, old_label],
            old_styles_map,
        );
        let new_b = make_bundle(
            "http://localhost:3001/",
            vec![new_link, new_label],
            new_styles_map,
        );

        // Pairs: link↔link (index 0↔0), label↔label (index 1↔1)
        let outcome = make_outcome(vec![make_matched_pair(0, 0), make_matched_pair(1, 1)]);

        let issues = style_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false);

        let style_changed: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == crate::contract::IssueType::StyleChanged)
            .collect();

        assert!(
            !style_changed.is_empty(),
            "v05 regression: dup-label with changed computed style MUST emit style_changed; got 0 issues"
        );
    }
}
