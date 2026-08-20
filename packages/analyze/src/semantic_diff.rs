//! Content/semantic diff from matched pairs and missing records (M3.md §5.2–5.4).
//!
//! Entry point: `semantic_issues(old, new, outcome, viewport, profile, env_mismatch) -> Vec<Issue>`
//!
//! Emission order (§5.1):
//!   1. Page-level checks (title, meta_description, h1)
//!   2. Per-pair attribute diffs in old-seq order
//!   3. Missing old nodes in old-seq order (missing_*)
//!   4. broken_link from new.page.link_probes in owning-node seq order
//!
//! DETERMINISM: no HashMap, BTreeMap or sort-by-stable-key everywhere.

use std::collections::{BTreeMap, BTreeSet};

use url::Url;

use crate::config::ImageDimensionsMode;
use crate::config::{
    base_confidence, ASPECT_RATIO_TOLERANCE, CHROME_PENALTY, DUP_LABEL_BBOX_TOLERANCE_PX,
    IMAGE_DIM_RATIO_FLOOR, UNCERTAIN_MULTIPLIER,
};
use crate::contract::{
    Anchors, CaptureBundle, Issue, IssueCategory, IssueSeverity, IssueType, Locator, SemanticNode,
};
use crate::issue::compute_issue_id;
use crate::matching::{norm_href, MatchBand, MatchOutcome, MatchStage, MissRecord};
use crate::scoring::{compute_confidence, SeverityResolver};

// ---------------------------------------------------------------------------
// C1: dup-label id set (M6 calibration, emission-side suppression only)
// ---------------------------------------------------------------------------

/// Normalise text for the dup-label comparison: trim whitespace, collapse
/// internal whitespace, ASCII-lowercase.  Mirrors `matching::text_sim` tokens.
fn norm_text_for_dup_filter(s: &str) -> String {
    s.split_whitespace()
        .map(|w| w.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Returns the set of old-stream text-node ids that are "duplicate labels":
/// a `text` node T is a dup-label when a `link` or `button` node L exists
/// in the same stream satisfying ALL of:
///   1. norm_text(T.text) == norm_text(L.text), both non-empty
///   2. T.bbox is contained within L.bbox ± DUP_LABEL_BBOX_TOLERANCE_PX
///
/// Suppresses only `missing_text` emission for these nodes; they remain in
/// all matcher / style / sequence inputs.
pub fn dup_label_ids(nodes: &[SemanticNode]) -> BTreeSet<String> {
    // Build a stable lookup of link/button text → list of their bboxes.
    // Key: normalised text; value: vec of [lx, ly, lw, lh] (as f64 for tolerance math).
    let mut container_bboxes: BTreeMap<String, Vec<[f64; 4]>> = BTreeMap::new();
    for node in nodes {
        if node.kind == "link" || node.kind == "button" {
            let raw = node.text.as_deref().unwrap_or("");
            let normed = norm_text_for_dup_filter(raw);
            if normed.is_empty() {
                continue;
            }
            let [lx, ly, lw, lh] = node.bbox;
            container_bboxes
                .entry(normed)
                .or_default()
                .push([lx as f64, ly as f64, lw as f64, lh as f64]);
        }
    }

    let tol = DUP_LABEL_BBOX_TOLERANCE_PX;
    let mut dup_ids: BTreeSet<String> = BTreeSet::new();

    for node in nodes {
        if node.kind != "text" {
            continue;
        }
        let raw = node.text.as_deref().unwrap_or("");
        let normed = norm_text_for_dup_filter(raw);
        if normed.is_empty() {
            continue;
        }
        let containers = match container_bboxes.get(&normed) {
            Some(v) => v,
            None => continue,
        };
        let [tx, ty, tw, th] = node.bbox;
        let (tx, ty, tw, th) = (tx as f64, ty as f64, tw as f64, th as f64);
        // Check containment: T fully within L ± tol
        for &[lx, ly, lw, lh] in containers {
            if tx >= lx - tol
                && ty >= ly - tol
                && tx + tw <= lx + lw + tol
                && ty + th <= ly + lh + tol
            {
                dup_ids.insert(node.id.clone());
                break;
            }
        }
    }

    dup_ids
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Derive semantic/content issues from matcher output.
///
/// Returns issues in the emission order specified by M3.md §5.1.
pub fn semantic_issues(
    old: &CaptureBundle,
    new: &CaptureBundle,
    outcome: &MatchOutcome,
    viewport: &str,
    profile: &SeverityResolver,
    env_mismatch: bool,
    image_dims_mode: ImageDimensionsMode,
) -> Vec<Issue> {
    let new_lang = new.page.lang.clone();
    let old_nodes = &old.page.nodes;
    let new_nodes = &new.page.nodes;
    let old_det = &old.determinism;
    let new_det = &new.determinism;

    let mut issues: Vec<Issue> = Vec::new();

    // 1. Page-level checks (§5.2)
    issues.extend(page_level_checks(old, new, viewport, profile, &new_lang));

    // 2. Per-pair attribute diffs (§5.3) in old-seq order
    // outcome.pairs is already sorted by old seq_index
    for pair in &outcome.pairs {
        let old_node = &old_nodes[pair.old_idx];
        let new_node = &new_nodes[pair.new_idx];

        let match_evidence = serde_json::json!({
            "stage": match pair.stage { MatchStage::Identity => "identity", MatchStage::Assignment => "assignment" },
            "score": round4(pair.score),
            "band": match pair.band { MatchBand::Matched => "matched", MatchBand::Uncertain => "uncertain" },
            "signals": signals_to_json(&pair.signals),
        });

        // Confidence base
        let base = match pair.stage {
            MatchStage::Identity => base_confidence::CONTENT_IDENTITY,
            MatchStage::Assignment => base_confidence::CONTENT_ASSIGNMENT,
        };
        // × UNCERTAIN_MULTIPLIER if band Uncertain
        let base_with_band = if pair.band == MatchBand::Uncertain {
            base * UNCERTAIN_MULTIPLIER
        } else {
            base
        };
        // × CHROME_PENALTY if new node's landmark is banner/navigation/contentinfo
        let base_with_chrome = if is_chrome_landmark(new_node.anchors.landmark.as_deref()) {
            base_with_band * CHROME_PENALTY
        } else {
            base_with_band
        };
        let confidence = compute_confidence(base_with_chrome, env_mismatch, old_det, new_det);

        // Locator uses NEW node anchors (both sides selectors/bboxes/seq)
        let anchors = node_to_anchors(new_node);

        let mut pair_issues = pair_attribute_issues(
            old_node,
            new_node,
            &match_evidence,
            confidence,
            viewport,
            &new_lang,
            profile,
            &old.page.final_url,
            &new.page.final_url,
            old_det,
            new_det,
            env_mismatch,
            &anchors,
            pair.band == MatchBand::Uncertain,
            matches!(pair.stage, MatchStage::Assignment),
            image_dims_mode,
        );

        issues.append(&mut pair_issues);
    }

    // 3. Missing old nodes in old-seq order (§5.4)
    // C1 (M6 calibration): compute dup-label id set once for the old stream.
    // text nodes whose id is in this set are dup-labels nested inside a link/button;
    // suppress missing_text emission for them only.
    let dup_ids = dup_label_ids(old_nodes);

    for miss in &outcome.missing_old {
        let old_node = &old_nodes[miss.idx];

        // C1: skip missing_text for dup-label text nodes.
        if old_node.kind == "text" && dup_ids.contains(&old_node.id) {
            continue;
        }

        // Chrome penalty by OLD node's landmark
        let base = base_confidence::CONTENT_ASSIGNMENT;
        let base_with_chrome = if is_chrome_landmark(old_node.anchors.landmark.as_deref()) {
            base * CHROME_PENALTY
        } else {
            base
        };
        let confidence = compute_confidence(base_with_chrome, env_mismatch, old_det, new_det);

        let match_evidence = missing_match_evidence(miss);

        if let Some(issue) = missing_node_issue(
            old_node,
            &match_evidence,
            confidence,
            viewport,
            &new_lang,
            profile,
        ) {
            issues.push(issue);
        }
    }

    // 4. broken_link from new.page.link_probes in owning-node seq order (§5.4)
    let mut broken_link_issues = broken_link_issues(old, new, outcome, viewport, &new_lang);
    issues.append(&mut broken_link_issues);

    issues
}

// ---------------------------------------------------------------------------
// §5.2: Page-level checks
// ---------------------------------------------------------------------------

fn page_level_checks(
    old: &CaptureBundle,
    new: &CaptureBundle,
    viewport: &str,
    profile: &SeverityResolver,
    new_lang: &Option<String>,
) -> Vec<Issue> {
    let mut issues = Vec::new();
    let conf = base_confidence::PAGE_FACT;

    // Title
    let old_title = old
        .page
        .title
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let new_title = new
        .page
        .title
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    match (old_title, new_title) {
        (Some(ot), None) => {
            // missing_title
            let anchors = Anchors::null();
            let id = compute_issue_id(&IssueType::MissingTitle, viewport, &anchors, None);
            let severity = profile.severity_for(&IssueType::MissingTitle, &IssueCategory::Content);
            let evidence = serde_json::json!({
                "old": { "title": ot },
                "new": null
            });
            let remediation = serde_json::json!({
                "action": "restore_text",
                "findBy": { "grep": [ot] },
                "from": null,
                "to": ot
            });
            issues.push(Issue {
                id,
                issue_type: IssueType::MissingTitle,
                category: IssueCategory::Content,
                severity,
                confidence: conf,
                viewport: viewport.to_string(),
                locale: new_lang.clone(),
                goal: Some("G2".to_string()),
                message: format!("Title removed: was '{}'", ot),
                locator: null_locator(anchors),
                evidence,
                remediation: Some(remediation),
            });
        }
        (Some(ot), Some(nt)) if ot != nt => {
            // changed_title
            let anchors = Anchors::null();
            let id = compute_issue_id(&IssueType::ChangedTitle, viewport, &anchors, None);
            let severity = profile.severity_for(&IssueType::ChangedTitle, &IssueCategory::Content);
            let evidence = serde_json::json!({
                "old": { "title": ot },
                "new": { "title": nt }
            });
            let remediation = serde_json::json!({
                "action": "restore_text",
                "findBy": { "grep": [nt] },
                "from": nt,
                "to": ot
            });
            issues.push(Issue {
                id,
                issue_type: IssueType::ChangedTitle,
                category: IssueCategory::Content,
                severity,
                confidence: conf,
                viewport: viewport.to_string(),
                locale: new_lang.clone(),
                goal: Some("G2".to_string()),
                message: format!("Title changed from '{}' to '{}'", ot, nt),
                locator: null_locator(anchors),
                evidence,
                remediation: Some(remediation),
            });
        }
        _ => {}
    }

    // Meta description
    let old_meta = old
        .page
        .meta_description
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let new_meta = new
        .page
        .meta_description
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    match (old_meta, new_meta) {
        (Some(om), None) => {
            let anchors = Anchors::null();
            let id = compute_issue_id(&IssueType::MissingMetaDescription, viewport, &anchors, None);
            let severity =
                profile.severity_for(&IssueType::MissingMetaDescription, &IssueCategory::Content);
            let evidence = serde_json::json!({
                "old": { "metaDescription": om },
                "new": null
            });
            let remediation = serde_json::json!({
                "action": "restore_text",
                "findBy": { "grep": [om] },
                "from": null,
                "to": om
            });
            issues.push(Issue {
                id,
                issue_type: IssueType::MissingMetaDescription,
                category: IssueCategory::Content,
                severity,
                confidence: conf,
                viewport: viewport.to_string(),
                locale: new_lang.clone(),
                goal: Some("G2".to_string()),
                message: "Meta description removed".to_string(),
                locator: null_locator(anchors),
                evidence,
                remediation: Some(remediation),
            });
        }
        (Some(om), Some(nm)) if om != nm => {
            let anchors = Anchors::null();
            let id = compute_issue_id(&IssueType::ChangedMetaDescription, viewport, &anchors, None);
            let severity =
                profile.severity_for(&IssueType::ChangedMetaDescription, &IssueCategory::Content);
            let evidence = serde_json::json!({
                "old": { "metaDescription": om },
                "new": { "metaDescription": nm }
            });
            let remediation = serde_json::json!({
                "action": "restore_text",
                "findBy": { "grep": [nm] },
                "from": nm,
                "to": om
            });
            issues.push(Issue {
                id,
                issue_type: IssueType::ChangedMetaDescription,
                category: IssueCategory::Content,
                severity,
                confidence: conf,
                viewport: viewport.to_string(),
                locale: new_lang.clone(),
                goal: Some("G2".to_string()),
                message: "Meta description changed".to_string(),
                locator: null_locator(anchors),
                evidence,
                remediation: Some(remediation),
            });
        }
        _ => {}
    }

    // missing_h1: old has a headingLevel==1 node, new has none (§5.2 item 3)
    let old_has_h1 = old.page.nodes.iter().any(|n| n.heading_level == Some(1));
    let new_has_h1 = new.page.nodes.iter().any(|n| n.heading_level == Some(1));

    if old_has_h1 && !new_has_h1 {
        // Find the first (lowest seq_index) old h1 for anchors
        let old_h1 = old
            .page
            .nodes
            .iter()
            .filter(|n| n.heading_level == Some(1))
            .min_by_key(|n| (n.seq_index, n.id.clone()));

        let (anchors, css_selector_old, bbox_old, seq_index_old) = if let Some(h1) = old_h1 {
            (
                node_to_anchors(h1),
                h1.css_selector.clone(),
                Some(h1.bbox),
                Some(h1.seq_index),
            )
        } else {
            (Anchors::null(), None, None, None)
        };

        let old_text = old_h1.and_then(|h| h.text.as_deref()).unwrap_or("");
        let id = compute_issue_id(&IssueType::MissingH1, viewport, &anchors, None);
        let severity = profile.severity_for(&IssueType::MissingH1, &IssueCategory::Content);
        let evidence = serde_json::json!({
            "old": { "text": old_text },
            "new": null
        });
        let remediation = serde_json::json!({
            "action": "restore_content",
            "findBy": { "grep": [old_text] },
            "from": null,
            "to": old_text
        });
        issues.push(Issue {
            id,
            issue_type: IssueType::MissingH1,
            category: IssueCategory::Content,
            severity,
            confidence: conf,
            viewport: viewport.to_string(),
            locale: new_lang.clone(),
            goal: Some("G2".to_string()),
            message: format!("H1 heading removed: was '{}'", old_text),
            locator: Locator {
                anchors,
                css_selector_old,
                css_selector_new: None,
                bbox_old,
                bbox_new: None,
                seq_index_old,
                seq_index_new: None,
            },
            evidence,
            remediation: Some(remediation),
        });
    }

    issues
}

// ---------------------------------------------------------------------------
// §5.3: Per-pair attribute diffs
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn pair_attribute_issues(
    old_node: &SemanticNode,
    new_node: &SemanticNode,
    match_evidence: &serde_json::Value,
    confidence: f64,
    viewport: &str,
    new_lang: &Option<String>,
    profile: &SeverityResolver,
    old_page_url: &str,
    new_page_url: &str,
    old_det: &crate::contract::CaptureDeterminism,
    new_det: &crate::contract::CaptureDeterminism,
    env_mismatch: bool,
    anchors: &Anchors,
    is_uncertain: bool,
    is_assignment: bool,
    image_dims_mode: ImageDimensionsMode,
) -> Vec<Issue> {
    let mut issues = Vec::new();

    let kind = old_node.kind.as_str();

    match kind {
        "heading" => {
            heading_pair_issues(
                old_node,
                new_node,
                match_evidence,
                confidence,
                viewport,
                new_lang,
                profile,
                anchors,
                &mut issues,
            );
        }
        "text" | "generic" => {
            text_pair_issues(
                old_node,
                new_node,
                match_evidence,
                confidence,
                viewport,
                new_lang,
                profile,
                anchors,
                &mut issues,
            );
        }
        "link" | "button" => {
            link_button_pair_issues(
                old_node,
                new_node,
                match_evidence,
                confidence,
                viewport,
                new_lang,
                profile,
                old_page_url,
                new_page_url,
                anchors,
                &mut issues,
            );
        }
        "image" => {
            image_pair_issues(
                old_node,
                new_node,
                match_evidence,
                confidence,
                viewport,
                new_lang,
                profile,
                old_det,
                new_det,
                env_mismatch,
                anchors,
                is_uncertain,
                is_assignment,
                image_dims_mode,
                &mut issues,
            );
        }
        _ => {}
    }

    issues
}

#[allow(clippy::too_many_arguments)]
fn heading_pair_issues(
    old_node: &SemanticNode,
    new_node: &SemanticNode,
    match_evidence: &serde_json::Value,
    confidence: f64,
    viewport: &str,
    new_lang: &Option<String>,
    profile: &SeverityResolver,
    anchors: &Anchors,
    issues: &mut Vec<Issue>,
) {
    let old_text = old_node.text.as_deref().unwrap_or("");
    let new_text = new_node.text.as_deref().unwrap_or("");
    let old_level = old_node.heading_level;
    let new_level = new_node.heading_level;

    // heading pair, text differs (check h1 first)
    if old_text != new_text {
        let either_h1 = old_level == Some(1) || new_level == Some(1);
        let itype = if either_h1 {
            IssueType::ChangedH1
        } else {
            IssueType::ChangedText
        };

        let id = compute_issue_id(&itype, viewport, anchors, None);
        let severity = profile.severity_for(&itype, &IssueCategory::Content);

        let near = new_node.anchors.nearest_heading.as_deref();
        let evidence = serde_json::json!({
            "match": match_evidence,
            "old": { "text": old_text },
            "new": { "text": new_text }
        });
        let remediation = serde_json::json!({
            "action": "restore_text",
            "findBy": { "grep": [new_text], "near": near },
            "from": new_text,
            "to": old_text
        });

        issues.push(Issue {
            id,
            issue_type: itype,
            category: IssueCategory::Content,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_lang.clone(),
            goal: Some("G2".to_string()),
            message: format!("Heading text changed from '{}' to '{}'", old_text, new_text),
            locator: node_pair_locator(anchors.clone(), old_node, new_node),
            evidence,
            remediation: Some(remediation),
        });
    }

    // heading pair, level differs
    if old_level != new_level {
        let id = compute_issue_id(&IssueType::HeadingStructureChanged, viewport, anchors, None);
        let severity =
            profile.severity_for(&IssueType::HeadingStructureChanged, &IssueCategory::Content);

        let old_tag = old_level
            .map(|l| format!("h{}", l))
            .unwrap_or_else(|| "h?".to_string());
        let new_tag = new_level
            .map(|l| format!("h{}", l))
            .unwrap_or_else(|| "h?".to_string());

        let evidence = serde_json::json!({
            "match": match_evidence,
            "old": { "level": old_level, "text": old_text },
            "new": { "level": new_level, "text": new_text }
        });
        let remediation = serde_json::json!({
            "action": "restore_heading_level",
            "findBy": { "grep": [new_text] },
            "from": new_tag,
            "to": old_tag
        });

        issues.push(Issue {
            id,
            issue_type: IssueType::HeadingStructureChanged,
            category: IssueCategory::Content,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_lang.clone(),
            goal: Some("G2".to_string()),
            message: format!("Heading level changed for '{}'", old_text),
            locator: node_pair_locator(anchors.clone(), old_node, new_node),
            evidence,
            remediation: Some(remediation),
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn text_pair_issues(
    old_node: &SemanticNode,
    new_node: &SemanticNode,
    match_evidence: &serde_json::Value,
    confidence: f64,
    viewport: &str,
    new_lang: &Option<String>,
    profile: &SeverityResolver,
    anchors: &Anchors,
    issues: &mut Vec<Issue>,
) {
    let old_text = old_node.text.as_deref().unwrap_or("");
    let new_text = new_node.text.as_deref().unwrap_or("");

    if old_text != new_text {
        let id = compute_issue_id(&IssueType::ChangedText, viewport, anchors, None);
        let severity = profile.severity_for(&IssueType::ChangedText, &IssueCategory::Content);

        let near = new_node.anchors.nearest_heading.as_deref();
        let evidence = serde_json::json!({
            "match": match_evidence,
            "old": { "text": old_text },
            "new": { "text": new_text }
        });
        let remediation = serde_json::json!({
            "action": "restore_text",
            "findBy": { "grep": [new_text], "near": near },
            "from": new_text,
            "to": old_text
        });

        issues.push(Issue {
            id,
            issue_type: IssueType::ChangedText,
            category: IssueCategory::Content,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_lang.clone(),
            goal: Some("G2".to_string()),
            message: format!("Text changed from '{}' to '{}'", old_text, new_text),
            locator: node_pair_locator(anchors.clone(), old_node, new_node),
            evidence,
            remediation: Some(remediation),
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn link_button_pair_issues(
    old_node: &SemanticNode,
    new_node: &SemanticNode,
    match_evidence: &serde_json::Value,
    confidence: f64,
    viewport: &str,
    new_lang: &Option<String>,
    profile: &SeverityResolver,
    old_page_url: &str,
    new_page_url: &str,
    anchors: &Anchors,
    issues: &mut Vec<Issue>,
) {
    let old_raw = old_node.raw_href.as_deref().or(old_node.href.as_deref());
    let new_raw = new_node.raw_href.as_deref().or(new_node.href.as_deref());

    // changed_link_target: raw hrefs differ AND norm(href) values differ (D3)
    if raw_and_norm_hrefs_differ(old_raw, new_raw, old_page_url, new_page_url) {
        let id = compute_issue_id(&IssueType::ChangedLinkTarget, viewport, anchors, None);
        let severity = profile.severity_for(&IssueType::ChangedLinkTarget, &IssueCategory::Content);

        let old_norm = old_raw
            .map(|h| norm_href(h, old_page_url))
            .unwrap_or_default();
        let new_norm = new_raw
            .map(|h| norm_href(h, new_page_url))
            .unwrap_or_default();

        let old_text = old_node.text.as_deref().unwrap_or("");
        let evidence = serde_json::json!({
            "match": match_evidence,
            "old": { "href": old_raw, "resolved": old_norm },
            "new": { "href": new_raw, "resolved": new_norm }
        });
        let remediation = serde_json::json!({
            "action": "update_link_target",
            "findBy": { "grep": [new_raw, old_text] },
            "from": new_raw,
            "to": old_raw
        });

        issues.push(Issue {
            id,
            issue_type: IssueType::ChangedLinkTarget,
            category: IssueCategory::Content,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_lang.clone(),
            goal: Some("G2".to_string()),
            message: format!(
                "Link target changed from '{}' to '{}'",
                old_raw.unwrap_or(""),
                new_raw.unwrap_or("")
            ),
            locator: node_pair_locator(anchors.clone(), old_node, new_node),
            evidence,
            remediation: Some(remediation),
        });
    }

    // changed_link_text: text differs
    let old_text = old_node.text.as_deref().unwrap_or("");
    let new_text = new_node.text.as_deref().unwrap_or("");
    if old_text != new_text {
        let id = compute_issue_id(&IssueType::ChangedLinkText, viewport, anchors, None);
        let severity = profile.severity_for(&IssueType::ChangedLinkText, &IssueCategory::Content);

        let near = new_node.anchors.nearest_heading.as_deref();
        let evidence = serde_json::json!({
            "match": match_evidence,
            "old": { "text": old_text },
            "new": { "text": new_text }
        });
        let remediation = serde_json::json!({
            "action": "restore_text",
            "findBy": { "grep": [new_text], "near": near },
            "from": new_text,
            "to": old_text
        });

        issues.push(Issue {
            id,
            issue_type: IssueType::ChangedLinkText,
            category: IssueCategory::Content,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_lang.clone(),
            goal: Some("G2".to_string()),
            message: format!("Link text changed from '{}' to '{}'", old_text, new_text),
            locator: node_pair_locator(anchors.clone(), old_node, new_node),
            evidence,
            remediation: Some(remediation),
        });
    }
}

/// Returns true if raw hrefs differ AND norm(href) values differ.
///
/// C4 (M6 calibration): uses cross-origin normalisation so that absolute links
/// on either input origin are treated as same-site.  Evidence/remediation
/// values keep their current raw forms — only the equality decision changes.
fn raw_and_norm_hrefs_differ(
    old_raw: Option<&str>,
    new_raw: Option<&str>,
    old_page: &str,
    new_page: &str,
) -> bool {
    use crate::matching::norm_href_cross_origin;
    match (old_raw, new_raw) {
        (None, None) => false,
        (Some(a), Some(b)) => {
            if a == b {
                return false;
            }
            // C4: normalise each side against BOTH page origins.
            let na = norm_href_cross_origin(a, old_page, new_page);
            let nb = norm_href_cross_origin(b, new_page, old_page);
            na != nb
        }
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn image_pair_issues(
    old_node: &SemanticNode,
    new_node: &SemanticNode,
    match_evidence: &serde_json::Value,
    confidence: f64,
    viewport: &str,
    new_lang: &Option<String>,
    profile: &SeverityResolver,
    old_det: &crate::contract::CaptureDeterminism,
    new_det: &crate::contract::CaptureDeterminism,
    env_mismatch: bool,
    anchors: &Anchors,
    is_uncertain: bool,
    _is_assignment: bool,
    image_dims_mode: ImageDimensionsMode,
    issues: &mut Vec<Issue>,
) {
    let old_loaded = old_node.loaded.unwrap_or(true);
    let new_loaded = new_node.loaded.unwrap_or(true);

    // broken_image: old loaded && new !loaded (suppresses alt/dim for this pair)
    if old_loaded && !new_loaded {
        // broken_image confidence: BROKEN_IMAGE base × uncertain if uncertain × env multipliers
        let base = base_confidence::BROKEN_IMAGE;
        let base_with_band = if is_uncertain {
            base * UNCERTAIN_MULTIPLIER
        } else {
            base
        };
        // Chrome penalty for new node's landmark
        let base_with_chrome = if is_chrome_landmark(new_node.anchors.landmark.as_deref()) {
            base_with_band * CHROME_PENALTY
        } else {
            base_with_band
        };
        let broken_confidence =
            compute_confidence(base_with_chrome, env_mismatch, old_det, new_det);

        let id = compute_issue_id(&IssueType::BrokenImage, viewport, anchors, None);
        let severity = profile.severity_for(&IssueType::BrokenImage, &IssueCategory::Content);

        let near = new_node.anchors.nearest_heading.as_deref();
        let src_filename = filename_from_src(new_node.src.as_deref());
        let new_alt = new_node.image_alt.as_deref().unwrap_or("");
        let new_src = new_node.src.as_deref().unwrap_or("");

        let evidence = serde_json::json!({
            "match": match_evidence,
            "old": {
                "src": old_node.src,
                "naturalWidth": old_node.natural_width,
                "naturalHeight": old_node.natural_height,
                "loaded": true
            },
            "new": {
                "src": new_node.src,
                "naturalWidth": new_node.natural_width,
                "naturalHeight": new_node.natural_height,
                "loaded": false
            }
        });
        let remediation = serde_json::json!({
            "action": "fix_image_asset",
            "findBy": { "grep": [src_filename, new_alt], "near": near },
            "from": new_src,
            "to": null,
            "note": "Image asset failed to load on new page"
        });

        issues.push(Issue {
            id,
            issue_type: IssueType::BrokenImage,
            category: IssueCategory::Content,
            severity,
            confidence: broken_confidence,
            viewport: viewport.to_string(),
            locale: new_lang.clone(),
            goal: Some("G7".to_string()),
            message: format!("Image failed to load: {}", new_src),
            locator: node_pair_locator(anchors.clone(), old_node, new_node),
            evidence,
            remediation: Some(remediation),
        });
        // Suppress changed_alt_text / missing_alt_text / changed_image_dimensions for this pair
        return;
    }

    // Both loaded (or at least not the broken case)
    let old_alt = old_node.image_alt.as_deref();
    let new_alt = new_node.image_alt.as_deref();

    // changed_alt_text: both loaded, alts differ, both non-empty
    let old_alt_nonempty = old_alt.map(|s| !s.is_empty()).unwrap_or(false);
    let new_alt_nonempty = new_alt.map(|s| !s.is_empty()).unwrap_or(false);

    if old_alt != new_alt && old_alt_nonempty && new_alt_nonempty {
        let id = compute_issue_id(&IssueType::ChangedAltText, viewport, anchors, None);
        let severity = profile.severity_for(&IssueType::ChangedAltText, &IssueCategory::Content);

        let src_filename = filename_from_src(new_node.src.as_deref());
        let evidence = serde_json::json!({
            "match": match_evidence,
            "old": { "alt": old_alt },
            "new": { "alt": new_alt }
        });
        let remediation = serde_json::json!({
            "action": "restore_text",
            "findBy": { "grep": [src_filename] },
            "from": new_alt,
            "to": old_alt
        });

        issues.push(Issue {
            id,
            issue_type: IssueType::ChangedAltText,
            category: IssueCategory::Content,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_lang.clone(),
            goal: Some("G2".to_string()),
            message: format!(
                "Alt text changed from '{}' to '{}'",
                old_alt.unwrap_or(""),
                new_alt.unwrap_or("")
            ),
            locator: node_pair_locator(anchors.clone(), old_node, new_node),
            evidence,
            remediation: Some(remediation),
        });
    }

    // missing_alt_text: old alt non-empty, new alt empty/null
    if old_alt_nonempty && !new_alt_nonempty {
        let id = compute_issue_id(&IssueType::MissingAltText, viewport, anchors, None);
        let severity = profile.severity_for(&IssueType::MissingAltText, &IssueCategory::Content);

        let src_filename = filename_from_src(new_node.src.as_deref());
        let evidence = serde_json::json!({
            "match": match_evidence,
            "old": { "alt": old_alt },
            "new": { "alt": new_alt }
        });
        let remediation = serde_json::json!({
            "action": "restore_text",
            "findBy": { "grep": [src_filename] },
            "from": new_alt,
            "to": old_alt
        });

        issues.push(Issue {
            id,
            issue_type: IssueType::MissingAltText,
            category: IssueCategory::Content,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_lang.clone(),
            goal: Some("G2".to_string()),
            message: "Alt text removed from image".to_string(),
            locator: node_pair_locator(anchors.clone(), old_node, new_node),
            evidence,
            remediation: Some(remediation),
        });
    }

    // changed_image_dimensions: both loaded, dim ratio < IMAGE_DIM_RATIO_FLOOR on either axis
    if old_loaded && new_loaded {
        if let (Some(ow), Some(oh), Some(nw), Some(nh)) = (
            old_node.natural_width,
            old_node.natural_height,
            new_node.natural_width,
            new_node.natural_height,
        ) {
            if ow > 0 && oh > 0 && nw > 0 && nh > 0 {
                let w_ratio = f64::min(ow as f64, nw as f64) / f64::max(ow as f64, nw as f64);
                let h_ratio = f64::min(oh as f64, nh as f64) / f64::max(oh as f64, nh as f64);

                let strict_fires =
                    w_ratio < IMAGE_DIM_RATIO_FLOOR || h_ratio < IMAGE_DIM_RATIO_FLOOR;

                if strict_fires {
                    match image_dims_mode {
                        ImageDimensionsMode::Strict => {
                            // Byte-identical to the pre-WP-I behaviour.
                            let id = compute_issue_id(
                                &IssueType::ChangedImageDimensions,
                                viewport,
                                anchors,
                                None,
                            );
                            let severity = profile.severity_for(
                                &IssueType::ChangedImageDimensions,
                                &IssueCategory::Content,
                            );

                            let evidence = serde_json::json!({
                                "match": match_evidence,
                                "old": { "naturalWidth": ow, "naturalHeight": oh },
                                "new": { "naturalWidth": nw, "naturalHeight": nh }
                            });

                            issues.push(Issue {
                                id,
                                issue_type: IssueType::ChangedImageDimensions,
                                category: IssueCategory::Content,
                                severity,
                                confidence,
                                viewport: viewport.to_string(),
                                locale: new_lang.clone(),
                                goal: Some("G7".to_string()),
                                message: format!(
                                    "Image dimensions changed from {}x{} to {}x{}",
                                    ow, oh, nw, nh
                                ),
                                locator: node_pair_locator(anchors.clone(), old_node, new_node),
                                evidence,
                                remediation: None,
                            });
                        }
                        ImageDimensionsMode::Responsive => {
                            // Step 1: upscale — new is larger on either axis.
                            let is_upscale = nw > ow || nh > oh;
                            if is_upscale {
                                let id = compute_issue_id(
                                    &IssueType::ChangedImageDimensions,
                                    viewport,
                                    anchors,
                                    None,
                                );
                                let severity = profile.severity_for(
                                    &IssueType::ChangedImageDimensions,
                                    &IssueCategory::Content,
                                );
                                let evidence = serde_json::json!({
                                    "match": match_evidence,
                                    "old": { "naturalWidth": ow, "naturalHeight": oh },
                                    "new": { "naturalWidth": nw, "naturalHeight": nh },
                                    "responsive": { "verdict": "upscale" }
                                });
                                issues.push(Issue {
                                    id,
                                    issue_type: IssueType::ChangedImageDimensions,
                                    category: IssueCategory::Content,
                                    severity,
                                    confidence,
                                    viewport: viewport.to_string(),
                                    locale: new_lang.clone(),
                                    goal: Some("G7".to_string()),
                                    message: format!(
                                        "Image dimensions changed from {}x{} to {}x{}",
                                        ow, oh, nw, nh
                                    ),
                                    locator: node_pair_locator(anchors.clone(), old_node, new_node),
                                    evidence,
                                    remediation: None,
                                });
                                return;
                            }

                            // Step 2: aspect change — |old_ar - new_ar| / old_ar > tolerance.
                            let old_ar = ow as f64 / oh as f64;
                            let new_ar = nw as f64 / nh as f64;
                            let ar_frac_diff = (old_ar - new_ar).abs() / old_ar;
                            let is_aspect_changed = ar_frac_diff > ASPECT_RATIO_TOLERANCE;
                            if is_aspect_changed {
                                let id = compute_issue_id(
                                    &IssueType::ChangedImageDimensions,
                                    viewport,
                                    anchors,
                                    None,
                                );
                                let severity = profile.severity_for(
                                    &IssueType::ChangedImageDimensions,
                                    &IssueCategory::Content,
                                );
                                let evidence = serde_json::json!({
                                    "match": match_evidence,
                                    "old": { "naturalWidth": ow, "naturalHeight": oh },
                                    "new": { "naturalWidth": nw, "naturalHeight": nh },
                                    "responsive": { "verdict": "aspect_changed" }
                                });
                                issues.push(Issue {
                                    id,
                                    issue_type: IssueType::ChangedImageDimensions,
                                    category: IssueCategory::Content,
                                    severity,
                                    confidence,
                                    viewport: viewport.to_string(),
                                    locale: new_lang.clone(),
                                    goal: Some("G7".to_string()),
                                    message: format!(
                                        "Image dimensions changed from {}x{} to {}x{}",
                                        ow, oh, nw, nh
                                    ),
                                    locator: node_pair_locator(anchors.clone(), old_node, new_node),
                                    evidence,
                                    remediation: None,
                                });
                                return;
                            }

                            // Step 3: undersized — nw < rendered width of new node's bbox.
                            // bbox[2] is the rendered CSS width. Skip when bbox is missing or
                            // zero (treat as covering).
                            let rendered_w = new_node.bbox[2];
                            let is_undersized = rendered_w > 0 && (nw as i32) < rendered_w;

                            let (final_severity, verdict, rendered_w_evidence) = if is_undersized {
                                let severity = profile.severity_for(
                                    &IssueType::ChangedImageDimensions,
                                    &IssueCategory::Content,
                                );
                                (severity, "undersized", Some(rendered_w))
                            } else {
                                // Step 4: aspect-preserving downscale that covers the box → Info.
                                (
                                    IssueSeverity::Info,
                                    "intentional_downscale",
                                    if rendered_w > 0 {
                                        Some(rendered_w)
                                    } else {
                                        None
                                    },
                                )
                            };

                            let id = compute_issue_id(
                                &IssueType::ChangedImageDimensions,
                                viewport,
                                anchors,
                                None,
                            );

                            let responsive_evidence = match rendered_w_evidence {
                                Some(rw) => serde_json::json!({
                                    "verdict": verdict,
                                    "renderedWidth": rw
                                }),
                                None => serde_json::json!({
                                    "verdict": verdict,
                                    "renderedWidth": null
                                }),
                            };

                            let evidence = serde_json::json!({
                                "match": match_evidence,
                                "old": { "naturalWidth": ow, "naturalHeight": oh },
                                "new": { "naturalWidth": nw, "naturalHeight": nh },
                                "responsive": responsive_evidence
                            });

                            issues.push(Issue {
                                id,
                                issue_type: IssueType::ChangedImageDimensions,
                                category: IssueCategory::Content,
                                severity: final_severity,
                                confidence,
                                viewport: viewport.to_string(),
                                locale: new_lang.clone(),
                                goal: Some("G7".to_string()),
                                message: format!(
                                    "Image dimensions changed from {}x{} to {}x{}",
                                    ow, oh, nw, nh
                                ),
                                locator: node_pair_locator(anchors.clone(), old_node, new_node),
                                evidence,
                                remediation: None,
                            });
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// §5.4: Missing nodes → missing_* issues
// ---------------------------------------------------------------------------

fn missing_node_issue(
    old_node: &SemanticNode,
    match_evidence: &serde_json::Value,
    confidence: f64,
    viewport: &str,
    new_lang: &Option<String>,
    profile: &SeverityResolver,
) -> Option<Issue> {
    let kind = old_node.kind.as_str();

    // Map kind → IssueType
    let itype = match kind {
        "heading" => IssueType::MissingText, // headings emit missing_text (D8)
        "text" | "generic" => IssueType::MissingText,
        "link" => IssueType::MissingLink,
        "button" => IssueType::MissingButton,
        "image" => IssueType::MissingImage,
        "form" => IssueType::MissingForm,
        "field" => IssueType::MissingFormField,
        _ => IssueType::MissingText,
    };

    let goal = "G2";

    let anchors = node_to_anchors(old_node);
    let id = compute_issue_id(&itype, viewport, &anchors, None);
    let severity = profile.severity_for(&itype, &IssueCategory::Content);

    // Build evidence based on kind
    let evidence = match kind {
        "link" | "button" => {
            let href = old_node
                .raw_href
                .as_deref()
                .or(old_node.href.as_deref())
                .unwrap_or("");
            let text = old_node.text.as_deref().unwrap_or("");
            serde_json::json!({
                "match": match_evidence,
                "old": { "text": text, "href": href }
            })
        }
        "image" => {
            let src = old_node.src.as_deref().unwrap_or("");
            let alt = old_node.image_alt.as_deref().unwrap_or("");
            serde_json::json!({
                "match": match_evidence,
                "old": { "src": src, "alt": alt }
            })
        }
        _ => {
            let text = old_node.text.as_deref().unwrap_or("");
            serde_json::json!({
                "match": match_evidence,
                "old": { "text": text }
            })
        }
    };

    // Remediation
    let near = old_node.anchors.nearest_heading.as_deref();
    let remediation = match kind {
        "link" | "button" => {
            let href = old_node
                .raw_href
                .as_deref()
                .or(old_node.href.as_deref())
                .unwrap_or("");
            let text = old_node.text.as_deref().unwrap_or("");
            serde_json::json!({
                "action": "restore_content",
                "findBy": { "grep": [text, href], "near": near },
                "from": null,
                "to": text
            })
        }
        "image" => {
            let src = old_node.src.as_deref().unwrap_or("");
            let alt = old_node.image_alt.as_deref().unwrap_or("");
            serde_json::json!({
                "action": "restore_content",
                "findBy": { "grep": [src, alt], "near": near },
                "from": null,
                "to": src
            })
        }
        _ => {
            let text = old_node.text.as_deref().unwrap_or("");
            serde_json::json!({
                "action": "restore_content",
                "findBy": { "grep": [text], "near": near },
                "from": null,
                "to": text
            })
        }
    };

    let message = match kind {
        "link" => format!(
            "Link removed: '{}'",
            old_node
                .text
                .as_deref()
                .unwrap_or(old_node.href.as_deref().unwrap_or(""))
        ),
        "button" => format!(
            "Button removed: '{}'",
            old_node.text.as_deref().unwrap_or("")
        ),
        "image" => format!("Image removed: {}", old_node.src.as_deref().unwrap_or("")),
        "form" => "Form removed".to_string(),
        "field" => format!(
            "Form field removed: '{}'",
            old_node.acc_name.as_deref().unwrap_or("")
        ),
        _ => format!(
            "Content removed: '{}'",
            old_node.text.as_deref().unwrap_or("")
        ),
    };

    Some(Issue {
        id,
        issue_type: itype,
        category: IssueCategory::Content,
        severity,
        confidence,
        viewport: viewport.to_string(),
        locale: new_lang.clone(),
        goal: Some(goal.to_string()),
        message,
        locator: Locator {
            anchors,
            css_selector_old: old_node.css_selector.clone(),
            css_selector_new: None,
            bbox_old: Some(old_node.bbox),
            bbox_new: None,
            seq_index_old: Some(old_node.seq_index),
            seq_index_new: None,
        },
        evidence,
        remediation: Some(remediation),
    })
}

fn missing_match_evidence(miss: &MissRecord) -> serde_json::Value {
    serde_json::json!({
        "stage": "assignment",
        "bestScore": miss.best_score.map(round4),
        "band": "unmatched"
    })
}

// ---------------------------------------------------------------------------
// §5.4: broken_link
// ---------------------------------------------------------------------------

fn broken_link_issues(
    old: &CaptureBundle,
    new: &CaptureBundle,
    outcome: &MatchOutcome,
    viewport: &str,
    new_lang: &Option<String>,
) -> Vec<Issue> {
    let new_page_url = &new.page.final_url;

    // Build href → owning node map for new page (lowest seq_index match, fragment-stripped resolved href)
    let mut href_to_new_node: BTreeMap<String, &SemanticNode> = BTreeMap::new();
    for node in &new.page.nodes {
        if let Some(href) = node.raw_href.as_deref().or(node.href.as_deref()) {
            let resolved = resolve_and_strip_fragment(href, new_page_url);
            href_to_new_node
                .entry(resolved)
                .and_modify(|existing| {
                    if node.seq_index < existing.seq_index
                        || (node.seq_index == existing.seq_index && node.id < existing.id)
                    {
                        *existing = node;
                    }
                })
                .or_insert(node);
        }
    }

    // Build a reverse map from new_idx → old_idx for matched pairs
    let mut new_to_old: BTreeMap<usize, usize> = BTreeMap::new();
    for pair in &outcome.pairs {
        new_to_old.insert(pair.new_idx, pair.old_idx);
    }

    // Build old href → probe status map (fragment-stripped resolved href)
    let old_page_url = &old.page.final_url;
    let mut old_href_to_probe_status: BTreeMap<String, (Option<i32>, Option<String>)> =
        BTreeMap::new();
    for probe in &old.page.link_probes {
        // probe.url is already absolute and fragment-stripped
        old_href_to_probe_status
            .entry(probe.url.clone())
            .or_insert_with(|| (probe.status, probe.error.clone()));
    }

    // Also build old node's href → resolved URL for parity lookup
    let mut old_node_href_resolved: BTreeMap<usize, String> = BTreeMap::new();
    for (i, node) in old.page.nodes.iter().enumerate() {
        if let Some(href) = node.raw_href.as_deref().or(node.href.as_deref()) {
            let resolved = resolve_and_strip_fragment(href, old_page_url);
            old_node_href_resolved.insert(i, resolved);
        }
    }

    // Collect broken new probes, sort by owning-node seq_index then id for determinism
    let mut candidates: Vec<(u32, String, &crate::contract::LinkProbe)> = Vec::new();

    for probe in &new.page.link_probes {
        // Only skipped == null and status >= 400
        if probe.skipped.is_some() {
            continue;
        }
        let status = match probe.status {
            Some(s) if s >= 400 => s,
            _ => continue,
        };

        let _ = status; // used in emission below

        // Find the owning new node
        let probe_url_stripped = probe.url.clone(); // already fragment-stripped per contract
        let new_node = match href_to_new_node.get(&probe_url_stripped) {
            Some(n) => *n,
            None => continue,
        };

        candidates.push((new_node.seq_index, new_node.id.clone(), probe));
    }

    // Sort by (owning node seq_index, id) for determinism
    candidates.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.url.cmp(&b.2.url))
    });

    let mut issues = Vec::new();

    for (_, _, probe) in &candidates {
        let probe_url_stripped = &probe.url;
        let new_node = match href_to_new_node.get(probe_url_stripped.as_str()) {
            Some(n) => *n,
            None => continue,
        };

        // Parity check: does matched old partner also have a probe failure for the same URL?
        // Find new node's position by pointer identity, then look up its matched old idx.
        let new_node_idx = new
            .page
            .nodes
            .iter()
            .position(|n| std::ptr::eq(n, new_node));
        let old_idx = new_node_idx.and_then(|ni| new_to_old.get(&ni)).copied();

        // Get old partner's href → check old probes
        let suppress = if let Some(oi) = old_idx {
            // The old partner exists: look up its href in old probes
            if let Some(old_href_resolved) = old_node_href_resolved.get(&oi) {
                // Check if old probe for this URL has status >= 400 or error
                match old_href_to_probe_status.get(old_href_resolved) {
                    Some((Some(s), _)) if *s >= 400 => true, // old also 404+ → suppress
                    Some((_, Some(_))) => true,              // old has error → suppress
                    _ => false,                              // old was OK or unprobed → emit
                }
            } else {
                // Old partner has no href → emit
                false
            }
        } else {
            // No matched old partner → emit (unmatched/external/unprobed)
            false
        };

        if suppress {
            continue;
        }

        // Emit broken_link
        let new_raw = new_node
            .raw_href
            .as_deref()
            .or(new_node.href.as_deref())
            .unwrap_or("");
        let link_text = new_node.text.as_deref().unwrap_or("");

        let anchors = node_to_anchors(new_node);
        let id = compute_issue_id(&IssueType::BrokenLink, viewport, &anchors, None);

        // Old-side evidence
        let old_evidence = if let Some(oi) = old_idx {
            if let Some(old_href_resolved) = old_node_href_resolved.get(&oi) {
                match old_href_to_probe_status.get(old_href_resolved) {
                    Some((status, _)) => serde_json::json!({
                        "url": old_href_resolved,
                        "status": status
                    }),
                    None => serde_json::Value::Null,
                }
            } else {
                serde_json::Value::Null
            }
        } else {
            serde_json::Value::Null
        };

        let evidence = serde_json::json!({
            "new": { "url": probe.url, "status": probe.status },
            "old": old_evidence
        });
        let remediation = serde_json::json!({
            "action": "fix_broken_link",
            "findBy": { "grep": [new_raw, link_text] },
            "from": new_raw,
            "to": null,
            "note": format!("Link returned status {}", probe.status.unwrap_or(0))
        });

        issues.push(Issue {
            id,
            issue_type: IssueType::BrokenLink,
            category: IssueCategory::Content,
            severity: crate::contract::IssueSeverity::Error,
            confidence: base_confidence::BROKEN_LINK,
            viewport: viewport.to_string(),
            locale: new_lang.clone(),
            goal: Some("G7".to_string()),
            message: format!(
                "Broken link: {} returned {}",
                probe.url,
                probe.status.unwrap_or(0)
            ),
            locator: Locator {
                anchors,
                css_selector_old: None,
                css_selector_new: new_node.css_selector.clone(),
                bbox_old: None,
                bbox_new: Some(new_node.bbox),
                seq_index_old: None,
                seq_index_new: Some(new_node.seq_index),
            },
            evidence,
            remediation: Some(remediation),
        });
    }

    // Deduplicate by id (same URL may appear via multiple node matches)
    let mut seen_ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    issues.retain(|i| seen_ids.insert(i.id.clone()));

    issues
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a SemanticNode's anchors to Issue Anchors.
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

/// Build a Locator with both old and new node positions.
fn node_pair_locator(
    anchors: Anchors,
    old_node: &SemanticNode,
    new_node: &SemanticNode,
) -> Locator {
    Locator {
        anchors,
        css_selector_old: old_node.css_selector.clone(),
        css_selector_new: new_node.css_selector.clone(),
        bbox_old: Some(old_node.bbox),
        bbox_new: Some(new_node.bbox),
        seq_index_old: Some(old_node.seq_index),
        seq_index_new: Some(new_node.seq_index),
    }
}

/// Null locator (page-level issues).
fn null_locator(anchors: Anchors) -> Locator {
    Locator {
        anchors,
        css_selector_old: None,
        css_selector_new: None,
        bbox_old: None,
        bbox_new: None,
        seq_index_old: None,
        seq_index_new: None,
    }
}

/// Extract filename from a src URL.
fn filename_from_src(src: Option<&str>) -> String {
    match src {
        None => String::new(),
        Some(s) => {
            if let Ok(u) = Url::parse(s) {
                u.path_segments()
                    .and_then(|mut segs| segs.next_back())
                    .unwrap_or("")
                    .to_string()
            } else {
                s.rsplit('/').next().unwrap_or(s).to_string()
            }
        }
    }
}

/// Resolve href against page_url and strip fragment.
fn resolve_and_strip_fragment(href: &str, page_url: &str) -> String {
    if let Ok(base) = Url::parse(page_url) {
        if let Ok(mut resolved) = base.join(href) {
            resolved.set_fragment(None);
            return resolved.to_string();
        }
    }
    href.to_string()
}

/// Returns true if landmark is a "chrome" landmark (banner, navigation, contentinfo).
fn is_chrome_landmark(landmark: Option<&str>) -> bool {
    matches!(
        landmark,
        Some("banner") | Some("navigation") | Some("contentinfo")
    )
}

/// Round f64 to 4 decimal places.
fn round4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}

/// Convert BTreeMap<String, f64> signals to serde_json::Value.
fn signals_to_json(signals: &std::collections::BTreeMap<String, f64>) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (k, v) in signals {
        map.insert(k.clone(), serde_json::Value::from(*v));
    }
    serde_json::Value::Object(map)
}

// ---------------------------------------------------------------------------
// Unit tests (§5.8)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        A11yInfo, CaptureDeterminism, Environment, LinkProbe, NetworkInfo, NodeAnchors, PageModel,
        Screenshots, StepStatus, ViewportConfig,
    };
    use crate::matching::{
        match_nodes, MatchBand, MatchOutcome, MatchStage, MatchedPair, MissRecord, PageCtx,
    };
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

    fn make_page(url: &str, final_url: &str, nodes: Vec<SemanticNode>) -> PageModel {
        PageModel {
            url: url.to_string(),
            final_url: final_url.to_string(),
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

    fn make_bundle(url: &str, final_url: &str, nodes: Vec<SemanticNode>) -> CaptureBundle {
        CaptureBundle {
            schema_version: "1.0".to_string(),
            captured_at: "2026-01-01T00:00:00Z".to_string(),
            viewport: make_viewport_cfg(),
            environment: make_env(),
            determinism: make_det(),
            page: make_page(url, final_url, nodes),
            computed_styles: BTreeMap::new(),
            screenshots: Screenshots {
                full_page: "desktop/old.png".to_string(),
                viewport: "desktop/old-vp.png".to_string(),
            },
            style_candidates: Default::default(),
            hit_tests: None,
            pseudo_elements: None,
            pseudo_truncated: None,
        }
    }

    fn make_node(
        id: &str,
        kind: &str,
        text: Option<&str>,
        href: Option<&str>,
        raw_href: Option<&str>,
        heading_level: Option<u8>,
        image_alt: Option<&str>,
        src: Option<&str>,
        natural_width: Option<u32>,
        natural_height: Option<u32>,
        loaded: Option<bool>,
        bbox: [i32; 4],
        seq_index: u32,
        landmark: Option<&str>,
        nearest_heading: Option<&str>,
    ) -> SemanticNode {
        SemanticNode {
            id: id.to_string(),
            kind: kind.to_string(),
            role: None,
            text: text.map(|s| s.to_string()),
            acc_name: None,
            href: href.map(|s| s.to_string()),
            image_alt: image_alt.map(|s| s.to_string()),
            bbox,
            seq_index,
            anchors: NodeAnchors {
                text: text.map(|s| s.to_string()),
                role: None,
                href: href.map(|s| s.to_string()),
                alt: image_alt.map(|s| s.to_string()),
                aria_label: None,
                nearest_heading: nearest_heading.map(|s| s.to_string()),
                landmark: landmark.map(|s| s.to_string()),
                ordinal_in_landmark: None,
            },
            css_selector: None,
            raw_href: raw_href.map(|s| s.to_string()),
            src: src.map(|s| s.to_string()),
            natural_width,
            natural_height,
            loaded,
            heading_level,
        }
    }

    fn profile() -> SeverityResolver {
        SeverityResolver::from_profile(ParityProfile::ContentStructure)
    }

    fn make_outcome_from_pair(
        old_idx: usize,
        new_idx: usize,
        stage: MatchStage,
        band: MatchBand,
        score: f64,
    ) -> MatchOutcome {
        let mut signals = BTreeMap::new();
        signals.insert("text".to_string(), score);
        MatchOutcome {
            pairs: vec![MatchedPair {
                old_idx,
                new_idx,
                score,
                stage,
                band,
                signals,
            }],
            missing_old: vec![],
            added_new: vec![],
        }
    }

    // -----------------------------------------------------------------------
    // §5.3 table: each row fires on a synthetic matched pair
    // -----------------------------------------------------------------------

    #[test]
    fn test_changed_h1_fires() {
        let old_node = make_node(
            "o1",
            "heading",
            Some("Old H1"),
            None,
            None,
            Some(1),
            None,
            None,
            None,
            None,
            None,
            [0, 100, 200, 40],
            0,
            Some("main"),
            None,
        );
        let new_node = make_node(
            "n1",
            "heading",
            Some("New H1"),
            None,
            None,
            Some(1),
            None,
            None,
            None,
            None,
            None,
            [0, 100, 200, 40],
            0,
            Some("main"),
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node.clone()]);
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        let h1_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::ChangedH1)
            .collect();
        assert_eq!(h1_issues.len(), 1, "changed_h1 should fire");
    }

    #[test]
    fn test_changed_text_fires_for_non_h1_heading() {
        let old_node = make_node(
            "o1",
            "heading",
            Some("Old Heading"),
            None,
            None,
            Some(2),
            None,
            None,
            None,
            None,
            None,
            [0, 100, 200, 30],
            0,
            Some("main"),
            None,
        );
        let new_node = make_node(
            "n1",
            "heading",
            Some("New Heading"),
            None,
            None,
            Some(2),
            None,
            None,
            None,
            None,
            None,
            [0, 100, 200, 30],
            0,
            Some("main"),
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node.clone()]);
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(
            issues
                .iter()
                .any(|i| i.issue_type == IssueType::ChangedText),
            "changed_text fires for non-h1 heading with text diff"
        );
        assert!(
            !issues.iter().any(|i| i.issue_type == IssueType::ChangedH1),
            "changed_h1 must NOT fire when neither level is 1"
        );
    }

    #[test]
    fn test_heading_structure_changed_fires() {
        let old_node = make_node(
            "o1",
            "heading",
            Some("Same Text"),
            None,
            None,
            Some(2),
            None,
            None,
            None,
            None,
            None,
            [0, 100, 200, 30],
            0,
            Some("main"),
            None,
        );
        let new_node = make_node(
            "n1",
            "heading",
            Some("Same Text"),
            None,
            None,
            Some(3),
            None,
            None,
            None,
            None,
            None,
            [0, 100, 200, 30],
            0,
            Some("main"),
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node.clone()]);
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(
            issues
                .iter()
                .any(|i| i.issue_type == IssueType::HeadingStructureChanged),
            "heading_structure_changed should fire"
        );
    }

    #[test]
    fn test_changed_text_fires_for_text_node() {
        let old_node = make_node(
            "o1",
            "text",
            Some("Old paragraph text"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 200, 400, 20],
            0,
            Some("main"),
            None,
        );
        let new_node = make_node(
            "n1",
            "text",
            Some("New paragraph text"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 200, 400, 20],
            0,
            Some("main"),
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node.clone()]);
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::ChangedText));
    }

    #[test]
    fn test_changed_link_target_fires_when_both_differ() {
        let old_node = make_node(
            "o1",
            "link",
            Some("Click me"),
            Some("http://old.com/page"),
            Some("page.html"),
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 100, 100, 30],
            0,
            None,
            None,
        );
        let new_node = make_node(
            "n1",
            "link",
            Some("Click me"),
            Some("http://new.com/other"),
            Some("other.html"),
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 100, 100, 30],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node.clone()]);
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::ChangedLinkTarget));
    }

    #[test]
    fn test_changed_link_target_suppressed_when_raw_equal() {
        // v14 shape: raw hrefs are identical even though resolved differ
        let old_node = make_node(
            "o1",
            "link",
            Some("Pricing"),
            Some("http://old.com/pricing.html"),
            Some("pricing.html"),
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 100, 100, 30],
            0,
            None,
            None,
        );
        let new_node = make_node(
            "n1",
            "link",
            Some("Pricing"),
            Some("http://new.com/products/connect/branded-call/pricing.html"),
            Some("pricing.html"),
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 100, 100, 30],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle(
            "http://new.com/products/connect/branded-call/",
            "http://new.com/products/connect/branded-call/",
            vec![new_node.clone()],
        );
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(
            !issues
                .iter()
                .any(|i| i.issue_type == IssueType::ChangedLinkTarget),
            "changed_link_target must NOT fire when raw hrefs are identical"
        );
    }

    #[test]
    fn test_changed_link_text_fires() {
        let old_node = make_node(
            "o1",
            "link",
            Some("Get a Demo"),
            Some("http://old.com/demo"),
            Some("demo.html"),
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 100, 100, 30],
            0,
            None,
            None,
        );
        let new_node = make_node(
            "n1",
            "link",
            Some("Schedule Demo"),
            Some("http://old.com/demo"),
            Some("demo.html"),
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 100, 100, 30],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node.clone()]);
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::ChangedLinkText));
    }

    #[test]
    fn test_broken_image_fires_and_suppresses_alt_dim() {
        // old loaded=true, new loaded=false
        let old_node = make_node(
            "o1",
            "image",
            None,
            None,
            None,
            None,
            Some("logo"),
            Some("http://old.com/logo.png"),
            Some(200),
            Some(100),
            Some(true),
            [0, 100, 200, 100],
            0,
            None,
            None,
        );
        let new_node = make_node(
            "n1",
            "image",
            None,
            None,
            None,
            None,
            Some(""),
            Some("http://new.com/logo.png"),
            Some(0),
            Some(0),
            Some(false),
            [0, 100, 200, 100],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node.clone()]);
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        // broken_image fires
        assert!(
            issues
                .iter()
                .any(|i| i.issue_type == IssueType::BrokenImage),
            "broken_image should fire"
        );
        // alt/dim suppressed
        assert!(
            !issues
                .iter()
                .any(|i| i.issue_type == IssueType::ChangedAltText),
            "changed_alt_text suppressed by broken_image"
        );
        assert!(
            !issues
                .iter()
                .any(|i| i.issue_type == IssueType::MissingAltText),
            "missing_alt_text suppressed by broken_image"
        );
        assert!(
            !issues
                .iter()
                .any(|i| i.issue_type == IssueType::ChangedImageDimensions),
            "changed_image_dimensions suppressed by broken_image"
        );
    }

    #[test]
    fn test_changed_alt_text_fires_both_nonempty() {
        let old_node = make_node(
            "o1",
            "image",
            None,
            None,
            None,
            None,
            Some("Old Alt"),
            Some("http://old.com/img.png"),
            Some(200),
            Some(100),
            Some(true),
            [0, 100, 200, 100],
            0,
            None,
            None,
        );
        let new_node = make_node(
            "n1",
            "image",
            None,
            None,
            None,
            None,
            Some("New Alt"),
            Some("http://new.com/img.png"),
            Some(200),
            Some(100),
            Some(true),
            [0, 100, 200, 100],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node.clone()]);
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::ChangedAltText));
    }

    #[test]
    fn test_both_empty_alt_no_issue() {
        let old_node = make_node(
            "o1",
            "image",
            None,
            None,
            None,
            None,
            Some(""),
            Some("http://old.com/img.png"),
            Some(200),
            Some(100),
            Some(true),
            [0, 100, 200, 100],
            0,
            None,
            None,
        );
        let new_node = make_node(
            "n1",
            "image",
            None,
            None,
            None,
            None,
            Some(""),
            Some("http://new.com/img.png"),
            Some(200),
            Some(100),
            Some(true),
            [0, 100, 200, 100],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node.clone()]);
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(
            !issues
                .iter()
                .any(|i| i.issue_type == IssueType::ChangedAltText),
            "both-empty alt should not fire"
        );
        assert!(
            !issues
                .iter()
                .any(|i| i.issue_type == IssueType::MissingAltText),
            "both-empty alt should not fire missing_alt_text"
        );
    }

    #[test]
    fn test_missing_alt_text_fires() {
        let old_node = make_node(
            "o1",
            "image",
            None,
            None,
            None,
            None,
            Some("Meaningful alt"),
            Some("http://old.com/img.png"),
            Some(200),
            Some(100),
            Some(true),
            [0, 100, 200, 100],
            0,
            None,
            None,
        );
        let new_node = make_node(
            "n1",
            "image",
            None,
            None,
            None,
            None,
            Some(""),
            Some("http://new.com/img.png"),
            Some(200),
            Some(100),
            Some(true),
            [0, 100, 200, 100],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node.clone()]);
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::MissingAltText));
    }

    #[test]
    fn test_changed_image_dimensions_fires() {
        let old_node = make_node(
            "o1",
            "image",
            None,
            None,
            None,
            None,
            Some("alt"),
            Some("http://old.com/img.png"),
            Some(600),
            Some(400),
            Some(true),
            [0, 100, 200, 133],
            0,
            None,
            None,
        );
        let new_node = make_node(
            "n1",
            "image",
            None,
            None,
            None,
            None,
            Some("alt"),
            Some("http://new.com/img.png"),
            Some(300),
            Some(200),
            Some(true),
            [0, 100, 200, 133],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node.clone()]);
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::ChangedImageDimensions));
    }

    // -----------------------------------------------------------------------
    // Page-level checks
    // -----------------------------------------------------------------------

    #[test]
    fn test_missing_title_fires() {
        let mut old_b = make_bundle("http://old.com/", "http://old.com/", vec![]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![]);
        old_b.page.title = Some("Old Title".to_string());
        let outcome = MatchOutcome {
            pairs: vec![],
            missing_old: vec![],
            added_new: vec![],
        };
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::MissingTitle));
    }

    #[test]
    fn test_changed_title_fires() {
        let mut old_b = make_bundle("http://old.com/", "http://old.com/", vec![]);
        let mut new_b = make_bundle("http://new.com/", "http://new.com/", vec![]);
        old_b.page.title = Some("Old Title".to_string());
        new_b.page.title = Some("New Title".to_string());
        let outcome = MatchOutcome {
            pairs: vec![],
            missing_old: vec![],
            added_new: vec![],
        };
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::ChangedTitle));
    }

    #[test]
    fn test_missing_meta_description_fires() {
        let mut old_b = make_bundle("http://old.com/", "http://old.com/", vec![]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![]);
        old_b.page.meta_description = Some("Old description".to_string());
        let outcome = MatchOutcome {
            pairs: vec![],
            missing_old: vec![],
            added_new: vec![],
        };
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::MissingMetaDescription));
    }

    #[test]
    fn test_missing_h1_fires_when_old_has_h1_new_does_not() {
        let h1_node = make_node(
            "h1old",
            "heading",
            Some("Big Title"),
            None,
            None,
            Some(1),
            None,
            None,
            None,
            None,
            None,
            [0, 50, 400, 50],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![h1_node]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![]);
        let outcome = MatchOutcome {
            pairs: vec![],
            missing_old: vec![],
            added_new: vec![],
        };
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(
            issues.iter().any(|i| i.issue_type == IssueType::MissingH1),
            "missing_h1 should fire"
        );
    }

    #[test]
    fn test_missing_h1_not_fires_when_both_have_h1() {
        let old_h1 = make_node(
            "h1old",
            "heading",
            Some("Title Old"),
            None,
            None,
            Some(1),
            None,
            None,
            None,
            None,
            None,
            [0, 50, 400, 50],
            0,
            None,
            None,
        );
        let new_h1 = make_node(
            "h1new",
            "heading",
            Some("Title New"),
            None,
            None,
            Some(1),
            None,
            None,
            None,
            None,
            None,
            [0, 50, 400, 50],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_h1.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_h1.clone()]);
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(
            !issues.iter().any(|i| i.issue_type == IssueType::MissingH1),
            "missing_h1 must NOT fire when both pages have h1"
        );
    }

    // -----------------------------------------------------------------------
    // Missing nodes: kind → type mapping
    // -----------------------------------------------------------------------

    #[test]
    fn test_missing_heading_emits_missing_text() {
        // heading kind → missing_text (D8)
        let old_node = make_node(
            "o1",
            "heading",
            Some("Section Heading"),
            None,
            None,
            Some(2),
            None,
            None,
            None,
            None,
            None,
            [0, 100, 300, 30],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![]);
        let outcome = MatchOutcome {
            pairs: vec![],
            missing_old: vec![MissRecord {
                idx: 0,
                best_score: None,
            }],
            added_new: vec![],
        };
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(
            issues
                .iter()
                .any(|i| i.issue_type == IssueType::MissingText),
            "heading → missing_text"
        );
    }

    #[test]
    fn test_missing_link_emits_missing_link() {
        let old_node = make_node(
            "o1",
            "link",
            Some("Get a Demo"),
            Some("/demo"),
            Some("/demo"),
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 100, 100, 30],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![]);
        let outcome = MatchOutcome {
            pairs: vec![],
            missing_old: vec![MissRecord {
                idx: 0,
                best_score: Some(0.5),
            }],
            added_new: vec![],
        };
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(
            issues
                .iter()
                .any(|i| i.issue_type == IssueType::MissingLink),
            "link → missing_link"
        );
    }

    #[test]
    fn test_missing_button_emits_missing_button() {
        let old_node = make_node(
            "o1",
            "button",
            Some("Submit"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 100, 100, 30],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![]);
        let outcome = MatchOutcome {
            pairs: vec![],
            missing_old: vec![MissRecord {
                idx: 0,
                best_score: None,
            }],
            added_new: vec![],
        };
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::MissingButton));
    }

    #[test]
    fn test_missing_form_emits_critical() {
        let old_node = make_node(
            "o1",
            "form",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 100, 300, 200],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![]);
        let outcome = MatchOutcome {
            pairs: vec![],
            missing_old: vec![MissRecord {
                idx: 0,
                best_score: None,
            }],
            added_new: vec![],
        };
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        let form_issue: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::MissingForm)
            .collect();
        assert_eq!(form_issue.len(), 1, "missing_form should fire");
        assert_eq!(
            form_issue[0].severity,
            crate::contract::IssueSeverity::Critical,
            "missing_form should be critical"
        );
    }

    // -----------------------------------------------------------------------
    // Chrome penalty
    // -----------------------------------------------------------------------

    #[test]
    fn test_chrome_penalty_applied_for_navigation_landmark() {
        let old_node = make_node(
            "o1",
            "text",
            Some("Nav text old"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 50, 200, 20],
            0,
            Some("navigation"),
            None,
        );
        let new_node = make_node(
            "n1",
            "text",
            Some("Nav text new"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 50, 200, 20],
            0,
            Some("navigation"),
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node.clone()]);
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        let issue = issues
            .iter()
            .find(|i| i.issue_type == IssueType::ChangedText)
            .unwrap();
        // base=0.95, *CHROME_PENALTY=0.85 => 0.8075
        let expected = round4(base_confidence::CONTENT_IDENTITY * CHROME_PENALTY);
        assert!(
            (issue.confidence - expected).abs() < 1e-9,
            "chrome penalty applied: {} vs {}",
            issue.confidence,
            expected
        );
    }

    #[test]
    fn test_chrome_penalty_not_applied_for_main_landmark() {
        let old_node = make_node(
            "o1",
            "text",
            Some("Main text old"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 200, 400, 20],
            0,
            Some("main"),
            None,
        );
        let new_node = make_node(
            "n1",
            "text",
            Some("Main text new"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 200, 400, 20],
            0,
            Some("main"),
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node.clone()]);
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        let issue = issues
            .iter()
            .find(|i| i.issue_type == IssueType::ChangedText)
            .unwrap();
        let expected = base_confidence::CONTENT_IDENTITY;
        assert!(
            (issue.confidence - expected).abs() < 1e-9,
            "no chrome penalty for main: {} vs {}",
            issue.confidence,
            expected
        );
    }

    // -----------------------------------------------------------------------
    // evidence.match present on all matcher-derived issues
    // -----------------------------------------------------------------------

    #[test]
    fn test_evidence_match_present_on_pair_issue() {
        let old_node = make_node(
            "o1",
            "text",
            Some("Old"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 0, 100, 20],
            0,
            None,
            None,
        );
        let new_node = make_node(
            "n1",
            "text",
            Some("New"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 0, 100, 20],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node.clone()]);
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        let issue = issues
            .iter()
            .find(|i| i.issue_type == IssueType::ChangedText)
            .unwrap();
        assert!(
            issue.evidence.get("match").is_some(),
            "evidence.match should be present"
        );
        assert_eq!(issue.evidence["match"]["stage"], "identity");
        assert_eq!(issue.evidence["match"]["band"], "matched");
    }

    #[test]
    fn test_evidence_match_present_on_missing_issue() {
        let old_node = make_node(
            "o1",
            "link",
            Some("Demo"),
            Some("/demo"),
            Some("/demo"),
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 100, 100, 30],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![]);
        let outcome = MatchOutcome {
            pairs: vec![],
            missing_old: vec![MissRecord {
                idx: 0,
                best_score: Some(0.4),
            }],
            added_new: vec![],
        };
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        let issue = issues
            .iter()
            .find(|i| i.issue_type == IssueType::MissingLink)
            .unwrap();
        assert!(
            issue.evidence.get("match").is_some(),
            "evidence.match should be present on missing issue"
        );
        assert_eq!(issue.evidence["match"]["band"], "unmatched");
        assert_eq!(issue.evidence["match"]["stage"], "assignment");
    }

    // -----------------------------------------------------------------------
    // broken_link parity matrix
    // -----------------------------------------------------------------------

    #[test]
    fn test_broken_link_old_404_suppressed() {
        // New link 404, old link also 404 → suppress
        let old_node = make_node(
            "o1",
            "link",
            Some("Pricing"),
            Some("http://old.com/pricing"),
            Some("/pricing"),
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 100, 100, 30],
            0,
            None,
            None,
        );
        let new_node = make_node(
            "n1",
            "link",
            Some("Pricing"),
            Some("http://new.com/pricing"),
            Some("/pricing"),
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 100, 100, 30],
            0,
            None,
            None,
        );

        let mut old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let mut new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node.clone()]);

        old_b.page.link_probes = vec![LinkProbe {
            url: "http://old.com/pricing".to_string(),
            redirect_chain: vec![],
            final_url: None,
            status: Some(404),
            skipped: None,
            error: None,
        }];

        new_b.page.link_probes = vec![LinkProbe {
            url: "http://new.com/pricing".to_string(),
            redirect_chain: vec![],
            final_url: None,
            status: Some(404),
            skipped: None,
            error: None,
        }];

        // Use real matcher so indices match up
        let ctx = PageCtx {
            old_final_url: "http://old.com/".to_string(),
            new_final_url: "http://new.com/".to_string(),
        };
        let outcome = match_nodes(&old_b.page.nodes, &new_b.page.nodes, &ctx, 4000, 4000);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        let broken: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::BrokenLink)
            .collect();
        assert!(
            broken.is_empty(),
            "broken_link suppressed when old also 404"
        );
    }

    #[test]
    fn test_broken_link_old_200_emitted() {
        // New link 404, old link 200 → emit
        let old_node = make_node(
            "o1",
            "link",
            Some("Pricing"),
            Some("http://old.com/pricing"),
            Some("/pricing"),
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 100, 100, 30],
            0,
            None,
            None,
        );
        let new_node = make_node(
            "n1",
            "link",
            Some("Pricing"),
            Some("http://new.com/pricing"),
            Some("/pricing"),
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 100, 100, 30],
            0,
            None,
            None,
        );

        let mut old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let mut new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node.clone()]);

        old_b.page.link_probes = vec![LinkProbe {
            url: "http://old.com/pricing".to_string(),
            redirect_chain: vec![],
            final_url: Some("http://old.com/pricing".to_string()),
            status: Some(200),
            skipped: None,
            error: None,
        }];
        new_b.page.link_probes = vec![LinkProbe {
            url: "http://new.com/pricing".to_string(),
            redirect_chain: vec![],
            final_url: None,
            status: Some(404),
            skipped: None,
            error: None,
        }];

        let ctx = PageCtx {
            old_final_url: "http://old.com/".to_string(),
            new_final_url: "http://new.com/".to_string(),
        };
        let outcome = match_nodes(&old_b.page.nodes, &new_b.page.nodes, &ctx, 4000, 4000);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        let broken: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::BrokenLink)
            .collect();
        assert_eq!(broken.len(), 1, "broken_link emitted when old was 200");
    }

    #[test]
    fn test_broken_link_old_external_unprobed_emitted() {
        // v11 shape: new link 404, old link was external (no old-side probe) → emit
        let old_node = make_node(
            "o1",
            "link",
            Some("Free Inspection"),
            Some("https://www.external.com/free-call-inspection"),
            Some("https://www.external.com/free-call-inspection"),
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 200, 150, 30],
            0,
            None,
            None,
        );
        let new_node = make_node(
            "n1",
            "link",
            Some("Free Inspection"),
            Some("http://localhost:3011/free-call-inspection"),
            Some("/free-call-inspection"),
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 200, 150, 30],
            0,
            None,
            None,
        );

        let old_b = make_bundle(
            "http://localhost:3000/",
            "http://localhost:3000/",
            vec![old_node.clone()],
        );
        let mut new_b = make_bundle(
            "http://localhost:3011/",
            "http://localhost:3011/",
            vec![new_node.clone()],
        );

        // No old-side probes (external link not probed)
        new_b.page.link_probes = vec![LinkProbe {
            url: "http://localhost:3011/free-call-inspection".to_string(),
            redirect_chain: vec![],
            final_url: None,
            status: Some(404),
            skipped: None,
            error: None,
        }];

        let ctx = PageCtx {
            old_final_url: "http://localhost:3000/".to_string(),
            new_final_url: "http://localhost:3011/".to_string(),
        };
        let outcome = match_nodes(&old_b.page.nodes, &new_b.page.nodes, &ctx, 4000, 4000);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        let broken: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::BrokenLink)
            .collect();
        assert_eq!(
            broken.len(),
            1,
            "broken_link emitted when old was external/unprobed"
        );
    }

    // -----------------------------------------------------------------------
    // Uncertain multiplier
    // -----------------------------------------------------------------------

    #[test]
    fn test_uncertain_multiplier_applied() {
        let old_node = make_node(
            "o1",
            "text",
            Some("Some text here"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 100, 200, 20],
            0,
            None,
            None,
        );
        let new_node = make_node(
            "n1",
            "text",
            Some("Different text here"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 300, 200, 20],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node.clone()]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node.clone()]);
        let outcome =
            make_outcome_from_pair(0, 0, MatchStage::Assignment, MatchBand::Uncertain, 0.55);
        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        let issue = issues
            .iter()
            .find(|i| i.issue_type == IssueType::ChangedText)
            .unwrap();
        let expected = round4(base_confidence::CONTENT_ASSIGNMENT * UNCERTAIN_MULTIPLIER);
        assert!(
            (issue.confidence - expected).abs() < 1e-9,
            "uncertain multiplier: got {} expected {}",
            issue.confidence,
            expected
        );
    }

    // -----------------------------------------------------------------------
    // C1: dup_label_ids helper tests (M6 calibration)
    // -----------------------------------------------------------------------

    fn make_c1_node_sd(
        id: &str,
        kind: &str,
        text: Option<&str>,
        bbox: [i32; 4],
        seq_index: u32,
    ) -> SemanticNode {
        use crate::contract::NodeAnchors;
        SemanticNode {
            id: id.to_string(),
            kind: kind.to_string(),
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
                landmark: None,
                ordinal_in_landmark: None,
            },
            css_selector: None,
            raw_href: None,
            src: None,
            natural_width: None,
            natural_height: None,
            loaded: None,
            heading_level: None,
        }
    }

    /// C1-a (semantic_diff module): text node inside a link's bbox with matching text →
    /// dup_label_ids returns the text node's id.
    #[test]
    fn test_sd_c1_dup_label_inside_link_in_set() {
        let link = make_c1_node_sd("link1", "link", Some("Get a Demo"), [0, 0, 200, 50], 0);
        let text = make_c1_node_sd("text1", "text", Some("Get a Demo"), [10, 10, 180, 30], 1);
        let nodes = vec![link, text];
        let set = dup_label_ids(&nodes);
        assert!(
            set.contains("text1"),
            "text dup-label id must be in the set"
        );
        assert!(!set.contains("link1"), "link id must NOT be in the set");
    }

    /// C1-b: text node outside link bbox → NOT in set.
    #[test]
    fn test_sd_c1_equal_text_outside_bbox_not_in_set() {
        let link = make_c1_node_sd("link1", "link", Some("Get a Demo"), [0, 0, 200, 50], 0);
        let text = make_c1_node_sd("text1", "text", Some("Get a Demo"), [300, 10, 180, 30], 1);
        let nodes = vec![link, text];
        let set = dup_label_ids(&nodes);
        assert!(!set.contains("text1"), "outside bbox must NOT be in set");
    }

    /// C1-c: different text, inside bbox → NOT in set.
    #[test]
    fn test_sd_c1_different_text_inside_bbox_not_in_set() {
        let link = make_c1_node_sd("link1", "link", Some("Get a Demo"), [0, 0, 200, 50], 0);
        let text = make_c1_node_sd("text1", "text", Some("Schedule Now"), [10, 10, 180, 30], 1);
        let nodes = vec![link, text];
        let set = dup_label_ids(&nodes);
        assert!(!set.contains("text1"), "different text must NOT be in set");
    }

    /// C1-d: end-to-end: full (unfiltered) streams — old has link+dup-label text, new has only
    /// link. semantic_issues must NOT emit missing_text for the dup-label text node.
    #[test]
    fn test_sd_c1_end_to_end_no_missing_text_for_dup_label() {
        let old_link = make_c1_node_sd(
            "old-link",
            "link",
            Some("Get a Demo"),
            [100, 200, 200, 50],
            0,
        );
        let old_text = make_c1_node_sd(
            "old-text",
            "text",
            Some("Get a Demo"),
            [110, 210, 180, 30],
            1,
        );
        let new_link = make_c1_node_sd(
            "new-link",
            "link",
            Some("Get a Demo"),
            [100, 200, 200, 50],
            0,
        );

        let old_b = make_bundle(
            "http://old.com/",
            "http://old.com/",
            vec![old_link, old_text],
        );
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_link]);

        let ctx = PageCtx {
            old_final_url: old_b.page.final_url.clone(),
            new_final_url: new_b.page.final_url.clone(),
        };
        let outcome = match_nodes(
            &old_b.page.nodes,
            &new_b.page.nodes,
            &ctx,
            old_b.page.page_height,
            new_b.page.page_height,
        );

        // The old-text node must be in missing_old (no text node in new stream).
        assert!(
            outcome
                .missing_old
                .iter()
                .any(|m| old_b.page.nodes[m.idx].id == "old-text"),
            "old-text must be missing_old in unfiltered match"
        );

        let issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );

        let missing_text: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::MissingText)
            .collect();
        assert!(
            missing_text.is_empty(),
            "C1: dup-label text node must NOT produce missing_text; got {} issues",
            missing_text.len()
        );
    }

    // -----------------------------------------------------------------------
    // Schema validation
    // -----------------------------------------------------------------------

    #[test]
    fn test_emitted_issue_validates_schema() {
        use crate::contract::{
            AgentSummary, Artifacts, DeterminismSummary, DiffResult, Scores, Suppressed,
            ViewportResult,
        };
        use crate::report::json::make_default_det_for_test;
        use jsonschema::JSONSchema;

        let old_node = make_node(
            "o1",
            "text",
            Some("Old text"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 100, 200, 20],
            0,
            None,
            None,
        );
        let new_node = make_node(
            "n1",
            "text",
            Some("New text"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            [0, 100, 200, 20],
            0,
            None,
            None,
        );
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node]);
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let mut issues = semantic_issues(
            &old_b,
            &new_b,
            &outcome,
            "desktop",
            &profile(),
            false,
            ImageDimensionsMode::Strict,
        );
        crate::issue::resolve_id_collisions(&mut issues);

        let result = DiffResult {
            schema_version: "1.3".to_string(),
            tool_version: "0.1.0".to_string(),
            run_id: "2026-01-01T00-00-00Z".to_string(),
            old_url: "http://old.com/".to_string(),
            new_url: "http://new.com/".to_string(),
            parity_profile: "content-structure".to_string(),
            severity_map: None,
            status: crate::contract::Status::Fail,
            agent_summary: AgentSummary {
                fixable_now: 0,
                by_type: BTreeMap::new(),
                by_severity: BTreeMap::new(),
                cluster_count: 0,
                region_count: 0,
                top_fixes: vec![],
            },
            scores: Scores::all_pass(),
            viewports: vec![ViewportResult {
                name: "desktop".to_string(),
                status: crate::contract::Status::Fail,
                issues: issues.iter().map(|i| i.id.clone()).collect(),
                artifacts: Artifacts {
                    old: "desktop/old.png".to_string(),
                    new: "desktop/new.png".to_string(),
                    diff: "desktop/diff.png".to_string(),
                },
            }],
            issues,
            clusters: vec![],
            regions: vec![],
            suppressed: Suppressed {
                count: 0,
                ids: vec![],
            },
            warnings: vec![],
            scoped_to: None,
            out_of_scope: crate::contract::OutOfScope {
                count: 0,
                ids: vec![],
            },
            determinism: DeterminismSummary {
                old: make_default_det_for_test(),
                new: make_default_det_for_test(),
            },
            artifacts: Artifacts {
                old: "desktop/old.png".to_string(),
                new: "desktop/new.png".to_string(),
                diff: "desktop/diff.png".to_string(),
            },
        };

        let json_str = result.to_json().expect("should serialize");
        let json_val: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let schema_str = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../contract/diff-result.schema.json"),
        )
        .expect("schema file must exist");
        let schema_val: serde_json::Value = serde_json::from_str(&schema_str).unwrap();
        let compiled = JSONSchema::compile(&schema_val).expect("schema must compile");
        let validation = compiled.validate(&json_val);
        if let Err(errors) = validation {
            let msgs: Vec<_> = errors.map(|e| e.to_string()).collect();
            panic!(
                "DiffResult with semantic issues failed schema validation:\n{}",
                msgs.join("\n")
            );
        }
    }

    // -----------------------------------------------------------------------
    // WP-I: ImageDimensionsMode tests
    // -----------------------------------------------------------------------

    /// Helper: make a minimal image SemanticNode with given natural dimensions and rendered bbox.
    fn make_img_node(id: &str, nw: u32, nh: u32, rendered_w: i32) -> SemanticNode {
        make_node(
            id,
            "image",
            None,
            None,
            None,
            None,
            Some("alt"),
            Some("http://example.com/img.jpg"),
            Some(nw),
            Some(nh),
            Some(true),
            [0, 100, rendered_w, 400],
            0,
            None,
            None,
        )
    }

    /// Run semantic_issues with the given mode for a single matched image pair and return the
    /// changed_image_dimensions issue(s), if any.
    fn run_img_dim_issues(
        old_nw: u32,
        old_nh: u32,
        old_rw: i32,
        new_nw: u32,
        new_nh: u32,
        new_rw: i32,
        mode: ImageDimensionsMode,
    ) -> Vec<crate::contract::Issue> {
        let old_node = make_img_node("o1", old_nw, old_nh, old_rw);
        let new_node = make_img_node("n1", new_nw, new_nh, new_rw);
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![old_node]);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![new_node]);
        let outcome = make_outcome_from_pair(0, 0, MatchStage::Identity, MatchBand::Matched, 1.0);
        let issues = semantic_issues(&old_b, &new_b, &outcome, "desktop", &profile(), false, mode);
        issues
            .into_iter()
            .filter(|i| i.issue_type == IssueType::ChangedImageDimensions)
            .collect()
    }

    /// WP-I-strict-existing: strict mode passes through the old behaviour — an aspect-preserving
    /// downscale still emits an error-severity issue.
    #[test]
    fn test_wpi_strict_aspect_preserving_downscale_still_errors() {
        // 1600x1067 → 800x533 (approx. same ratio): strict_fires = true (w_ratio=0.5 < 0.9)
        // Strict mode must still emit it.
        let dim_issues =
            run_img_dim_issues(1600, 1067, 1440, 800, 533, 600, ImageDimensionsMode::Strict);
        assert_eq!(
            dim_issues.len(),
            1,
            "strict: aspect-preserving downscale must still emit 1 issue"
        );
        assert_eq!(
            dim_issues[0].severity,
            crate::contract::IssueSeverity::Error,
            "strict: severity must be Error"
        );
        // No 'responsive' key in evidence in strict mode.
        assert!(
            dim_issues[0].evidence.get("responsive").is_none(),
            "strict: evidence must not contain 'responsive'"
        );
    }

    /// WP-I-responsive-intentional-downscale: 1600x1067 → 800x534, rendered width 600.
    /// nw(800) >= rendered_w(600): aspect-preserving downscale → Info severity, intentional_downscale.
    #[test]
    fn test_wpi_responsive_intentional_downscale_info() {
        let dim_issues = run_img_dim_issues(
            1600,
            1067,
            1440,
            800,
            534,
            600,
            ImageDimensionsMode::Responsive,
        );
        assert_eq!(
            dim_issues.len(),
            1,
            "responsive intentional_downscale: must emit exactly 1 issue"
        );
        let issue = &dim_issues[0];
        assert_eq!(
            issue.severity,
            crate::contract::IssueSeverity::Info,
            "responsive intentional_downscale: severity must be Info"
        );
        let resp = issue
            .evidence
            .get("responsive")
            .expect("responsive evidence must exist");
        assert_eq!(
            resp.get("verdict").and_then(|v| v.as_str()),
            Some("intentional_downscale"),
            "verdict must be 'intentional_downscale'"
        );
        assert_eq!(
            resp.get("renderedWidth").and_then(|v| v.as_i64()),
            Some(600),
            "renderedWidth must be 600"
        );
    }

    /// WP-I-responsive-upscale: 800x534 → 1600x1067.
    /// new is larger → upscale → Error severity, verdict "upscale".
    #[test]
    fn test_wpi_responsive_upscale_errors() {
        let dim_issues = run_img_dim_issues(
            800,
            534,
            800,
            1600,
            1067,
            1000,
            ImageDimensionsMode::Responsive,
        );
        assert_eq!(
            dim_issues.len(),
            1,
            "responsive upscale: must emit exactly 1 issue"
        );
        let issue = &dim_issues[0];
        assert_eq!(
            issue.severity,
            crate::contract::IssueSeverity::Error,
            "responsive upscale: severity must be Error"
        );
        let resp = issue
            .evidence
            .get("responsive")
            .expect("responsive evidence must exist");
        assert_eq!(
            resp.get("verdict").and_then(|v| v.as_str()),
            Some("upscale"),
            "verdict must be 'upscale'"
        );
    }

    /// WP-I-responsive-aspect-changed: 1600x1067 → 1200x900.
    /// old aspect ratio: 1600/1067 ≈ 1.4994; new: 1200/900 ≈ 1.3333.
    /// |1.4994 - 1.3333| / 1.4994 ≈ 0.111 >> 0.02 → aspect_changed, Error severity.
    #[test]
    fn test_wpi_responsive_aspect_changed_errors() {
        let dim_issues = run_img_dim_issues(
            1600,
            1067,
            1440,
            1200,
            900,
            800,
            ImageDimensionsMode::Responsive,
        );
        assert_eq!(
            dim_issues.len(),
            1,
            "responsive aspect_changed: must emit exactly 1 issue"
        );
        let issue = &dim_issues[0];
        assert_eq!(
            issue.severity,
            crate::contract::IssueSeverity::Error,
            "responsive aspect_changed: severity must be Error"
        );
        let resp = issue
            .evidence
            .get("responsive")
            .expect("responsive evidence must exist");
        assert_eq!(
            resp.get("verdict").and_then(|v| v.as_str()),
            Some("aspect_changed"),
            "verdict must be 'aspect_changed'"
        );
    }

    /// WP-I-responsive-undersized: 1600x1067 → 800x534, but rendered width is 900.
    /// nw(800) < rendered_w(900) → undersized, Error severity, renderedWidth 900.
    #[test]
    fn test_wpi_responsive_undersized_errors() {
        let dim_issues = run_img_dim_issues(
            1600,
            1067,
            1440,
            800,
            534,
            900,
            ImageDimensionsMode::Responsive,
        );
        assert_eq!(
            dim_issues.len(),
            1,
            "responsive undersized: must emit exactly 1 issue"
        );
        let issue = &dim_issues[0];
        assert_eq!(
            issue.severity,
            crate::contract::IssueSeverity::Error,
            "responsive undersized: severity must be Error"
        );
        let resp = issue
            .evidence
            .get("responsive")
            .expect("responsive evidence must exist");
        assert_eq!(
            resp.get("verdict").and_then(|v| v.as_str()),
            Some("undersized"),
            "verdict must be 'undersized'"
        );
        assert_eq!(
            resp.get("renderedWidth").and_then(|v| v.as_i64()),
            Some(900),
            "renderedWidth must be 900"
        );
    }

    /// WP-I-responsive-bbox-zero: bbox rendered width = 0 → step 3 skipped, intentional_downscale.
    #[test]
    fn test_wpi_responsive_bbox_zero_intentional_downscale() {
        // rendered_w = 0 → skip undersized check, land on intentional_downscale
        let dim_issues = run_img_dim_issues(
            1600,
            1067,
            1440,
            800,
            534,
            0,
            ImageDimensionsMode::Responsive,
        );
        assert_eq!(
            dim_issues.len(),
            1,
            "responsive bbox=0: must emit exactly 1 issue"
        );
        let issue = &dim_issues[0];
        assert_eq!(
            issue.severity,
            crate::contract::IssueSeverity::Info,
            "responsive bbox=0: severity must be Info (intentional_downscale)"
        );
        let resp = issue
            .evidence
            .get("responsive")
            .expect("responsive evidence must exist");
        assert_eq!(
            resp.get("verdict").and_then(|v| v.as_str()),
            Some("intentional_downscale"),
            "verdict must be 'intentional_downscale' when rendered_w=0"
        );
        // renderedWidth should be null when rendered_w is 0 (treated as missing)
        assert!(
            resp.get("renderedWidth")
                .map(|v| v.is_null())
                .unwrap_or(false),
            "renderedWidth must be null when bbox width is 0"
        );
    }
}
