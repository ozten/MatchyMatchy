//! Clickable-area hit-test diff (port-parity U7).
//!
//! Consumes the per-node `hitTests` grids captured by U6's probe and emits
//! `clickable_area_regressed` for matched pairs where the target went from
//! reliably clickable on `old` to meaningfully occluded on `new`.
//!
//! DETERMINISM: BTreeMap for miss-winner tallies; matched pairs processed in
//! (old.seq_index, old.id) order; float ratios rounded to 4 decimals before
//! formatting into evidence strings.

use std::collections::BTreeMap;

use crate::config::{
    base_confidence, CLICKABLE_DELTA, CLICKABLE_OLD_FLOOR, CLICKABLE_SETTLE_DEMOTION,
    MIN_HIT_DENOMINATOR,
};
use crate::contract::{
    Anchors, CaptureBundle, CaptureDeterminism, HitTestOutcome, HitTestPoint, HitTestStatus,
    Issue, IssueCategory, IssueType, Locator, QuiescenceStatus, SemanticNode, StepStatus,
};
use crate::issue::compute_issue_id;
use crate::matching::{MatchBand, MatchOutcome};
use crate::scoring::{compute_confidence, SeverityResolver};

/// Per-pair hit-test tally after index-alignment, exclusion, and both-miss drop.
///
/// `pub(crate)`: also reused by `explain.rs` to render the same joint
/// adjustment for a hand-inspected node pair (single source of truth for the
/// alignment/exclusion algorithm — never a second implementation).
pub(crate) struct HitTally {
    /// Points excluded because either side was `clipped` or `offViewport`.
    pub(crate) excluded: u32,
    /// Surviving denominator after exclusion and both-side-miss drop.
    pub(crate) denominator: u32,
    pub(crate) hits_old: u32,
    pub(crate) hits_new: u32,
    /// Old-side miss winner selector -> count, over the surviving denominator.
    /// Not used by the detector itself (only the new-side regression signal
    /// matters for evidence/remediation) — kept for `explain`'s symmetric view.
    pub(crate) old_miss_winners: BTreeMap<String, u32>,
    /// New-side miss winner selector -> count, over the surviving denominator.
    pub(crate) new_miss_winners: BTreeMap<String, u32>,
}

/// Index-align, exclude, and tally one pair's old/new point arrays (design brief
/// steps 1-2). Returns `None` when the arrays can't be compared at all (length
/// mismatch never emitted by capture, but defensive here).
pub(crate) fn tally_points(
    old_points: &[HitTestPoint],
    new_points: &[HitTestPoint],
) -> Option<HitTally> {
    if old_points.is_empty() || old_points.len() != new_points.len() {
        return None;
    }

    let mut excluded = 0u32;
    let mut denominator = 0u32;
    let mut hits_old = 0u32;
    let mut hits_new = 0u32;
    let mut old_miss_winners: BTreeMap<String, u32> = BTreeMap::new();
    let mut new_miss_winners: BTreeMap<String, u32> = BTreeMap::new();

    for (op, np) in old_points.iter().zip(new_points.iter()) {
        let old_excluded = matches!(op.o, HitTestOutcome::Clipped | HitTestOutcome::OffViewport);
        let new_excluded = matches!(np.o, HitTestOutcome::Clipped | HitTestOutcome::OffViewport);
        if old_excluded || new_excluded {
            excluded += 1;
            continue;
        }

        let old_miss = op.o == HitTestOutcome::Miss;
        let new_miss = np.o == HitTestOutcome::Miss;
        if old_miss && new_miss {
            // Both sides miss at this index: dropped from the denominator
            // entirely (design brief step 2) — neither an occlusion delta
            // nor an exclusion.
            continue;
        }

        denominator += 1;
        if !old_miss {
            hits_old += 1;
        } else if let Some(winner) = &op.winner {
            *old_miss_winners.entry(winner.clone()).or_insert(0) += 1;
        }
        if !new_miss {
            hits_new += 1;
        } else if let Some(winner) = &np.winner {
            *new_miss_winners.entry(winner.clone()).or_insert(0) += 1;
        }
    }

    Some(HitTally {
        excluded,
        denominator,
        hits_old,
        hits_new,
        old_miss_winners,
        new_miss_winners,
    })
}

/// Format the top-3 miss winners by (count desc, selector asc) as
/// `"sel (xN); sel2 (xM)"`. Empty string when there are no recorded winners.
pub(crate) fn format_miss_winners(winners: &BTreeMap<String, u32>) -> String {
    let mut entries: Vec<(&String, &u32)> = winners.iter().collect();
    entries.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    entries
        .into_iter()
        .take(3)
        .map(|(sel, count)| format!("{} (x{})", sel, count))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The single top miss-winner selector (count desc, then selector asc), if any.
fn top_miss_winner(winners: &BTreeMap<String, u32>) -> Option<String> {
    winners
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(sel, _)| sel.clone())
}

/// True when either bundle's determinism shows the settle stage failed to
/// cleanly reach quiescence. Absent settle/quiescence fields (pre-settle
/// bundles) mean NO demotion.
fn settle_penalty_applies(old_det: &CaptureDeterminism, new_det: &CaptureDeterminism) -> bool {
    let bad = |det: &CaptureDeterminism| -> bool {
        matches!(det.quiescence, Some(QuiescenceStatus::Timeout))
            || matches!(det.settle, Some(StepStatus::Failed) | Some(StepStatus::Skipped))
    };
    bad(old_det) || bad(new_det)
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

/// Build the `restore_clickable_area` remediation (design brief step 8):
/// the node's existing grep targets (href/text, falling back to nearestHeading)
/// plus the top miss-winner selector, with a note explaining the overlap.
fn build_remediation(anchors: &Anchors, top_winner: Option<&str>) -> serde_json::Value {
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
    if let Some(w) = top_winner {
        grep_targets.push(serde_json::Value::String(w.to_string()));
    }

    let note = match top_winner {
        Some(w) => format!(
            "The element matching \"{}\" overlaps this target and is intercepting clicks. \
             Check its position/size/z-index relative to the target. The tool does not name \
             the source component — use the grep targets to locate it in source or CMS.",
            w
        ),
        None => "An overlapping element is intercepting clicks on this target. Check the \
                 position/size/z-index of nearby elements. The tool does not name the source \
                 component — use the grep targets to locate it in source or CMS."
            .to_string(),
    };

    serde_json::json!({
        "action": "restore_clickable_area",
        "findBy": {
            "grep": grep_targets,
            "near": near
        },
        "note": note
    })
}

fn build_message(anchors: &Anchors, adjusted_old: f64, adjusted_new: f64, top_winner: Option<&str>) -> String {
    let near = anchors.nearest_heading.as_deref().unwrap_or("");
    let near_part = if !near.is_empty() {
        format!(" near \"{}\"", near)
    } else {
        String::new()
    };
    let winner_part = match top_winner {
        Some(w) => format!(" (top overlap: {})", w),
        None => String::new(),
    };
    format!(
        "Clickable area regressed{}: {:.0}% -> {:.0}% of sampled points hit{}",
        near_part,
        adjusted_old * 100.0,
        adjusted_new * 100.0,
        winner_part
    )
}

/// Derive `clickable_area_regressed` issues from matched pairs whose bundles
/// both carry sampled hit-test data (design brief steps 1-10).
///
/// Emission order: matched pairs processed in (old.seq_index, old.id) order —
/// total-order, byte-deterministic.
pub fn clickable_area_issues(
    old_bundle: &CaptureBundle,
    new_bundle: &CaptureBundle,
    match_outcome: &MatchOutcome,
    viewport: &str,
    profile: &SeverityResolver,
    env_mismatch: bool,
) -> Vec<Issue> {
    let mut issues: Vec<Issue> = Vec::new();

    let old_hit_tests = match old_bundle.hit_tests.as_ref() {
        Some(m) if !m.is_empty() => m,
        _ => return issues,
    };
    let new_hit_tests = match new_bundle.hit_tests.as_ref() {
        Some(m) if !m.is_empty() => m,
        _ => return issues,
    };

    let mut matched_pairs: Vec<&crate::matching::MatchedPair> = match_outcome
        .pairs
        .iter()
        .filter(|p| p.band == MatchBand::Matched)
        .collect();
    matched_pairs.sort_by(|a, b| {
        let oa = &old_bundle.page.nodes[a.old_idx];
        let ob = &old_bundle.page.nodes[b.old_idx];
        oa.seq_index
            .cmp(&ob.seq_index)
            .then_with(|| oa.id.cmp(&ob.id))
    });

    for pair in matched_pairs {
        let old_node = &old_bundle.page.nodes[pair.old_idx];
        let new_node = &new_bundle.page.nodes[pair.new_idx];

        let old_entry = match old_hit_tests.get(&old_node.id) {
            Some(e) => e,
            None => continue,
        };
        let new_entry = match new_hit_tests.get(&new_node.id) {
            Some(e) => e,
            None => continue,
        };

        if old_entry.status != HitTestStatus::Sampled || new_entry.status != HitTestStatus::Sampled
        {
            continue;
        }
        let (old_points, new_points) = match (&old_entry.points, &new_entry.points) {
            (Some(o), Some(n)) => (o, n),
            _ => continue,
        };

        let tally = match tally_points(old_points, new_points) {
            Some(t) => t,
            None => continue,
        };

        if (tally.denominator as usize) < MIN_HIT_DENOMINATOR {
            continue;
        }

        let adjusted_old = tally.hits_old as f64 / tally.denominator as f64;
        let adjusted_new = tally.hits_new as f64 / tally.denominator as f64;

        if !(adjusted_old >= CLICKABLE_OLD_FLOOR && (adjusted_old - adjusted_new) > CLICKABLE_DELTA)
        {
            continue;
        }

        let old_anchors = node_to_anchors(old_node);
        let top_winner = top_miss_winner(&tally.new_miss_winners);

        let severity =
            profile.severity_for(&IssueType::ClickableAreaRegressed, &IssueCategory::Visual);

        let mut confidence = compute_confidence(
            base_confidence::CLICKABLE_AREA_REGRESSED,
            env_mismatch,
            &old_bundle.determinism,
            &new_bundle.determinism,
        );
        if settle_penalty_applies(&old_bundle.determinism, &new_bundle.determinism) {
            confidence = round4(confidence * CLICKABLE_SETTLE_DEMOTION);
        }

        let id = compute_issue_id(
            &IssueType::ClickableAreaRegressed,
            viewport,
            &old_anchors,
            None,
        );

        let old_ev = serde_json::json!({
            "hitFraction": format!("{:.4}", round4(adjusted_old)),
            "rawHits": format!("{}/{}", tally.hits_old, tally.denominator),
        });
        let new_ev = serde_json::json!({
            "hitFraction": format!("{:.4}", round4(adjusted_new)),
            "rawHits": format!("{}/{}", tally.hits_new, tally.denominator),
            "missWinners": format_miss_winners(&tally.new_miss_winners),
        });
        let evidence = serde_json::json!({
            "old": old_ev,
            "new": new_ev,
            "excludedPoints": tally.excluded.to_string(),
        });

        let remediation = build_remediation(&old_anchors, top_winner.as_deref());
        let message = build_message(&old_anchors, adjusted_old, adjusted_new, top_winner.as_deref());

        issues.push(Issue {
            id,
            issue_type: IssueType::ClickableAreaRegressed,
            category: IssueCategory::Visual,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_bundle.page.lang.clone(),
            goal: None,
            message,
            locator: Locator {
                anchors: old_anchors,
                css_selector_old: old_node.css_selector.clone(),
                css_selector_new: new_node.css_selector.clone(),
                bbox_old: Some(old_node.bbox),
                bbox_new: Some(new_node.bbox),
                seq_index_old: Some(old_node.seq_index),
                seq_index_new: Some(new_node.seq_index),
            },
            evidence,
            remediation: Some(remediation),
        });
    }

    issues
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        A11yInfo, CaptureDeterminism, Environment, HitTestEntry, HitTestPoint, HitTestSkipReason,
        NetworkInfo, NodeAnchors, PageModel, Screenshots, StepStatus, StyleCandidates,
        ViewportConfig,
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

    /// Builds a bundle with the given determinism and hit-test map.
    fn make_bundle_full(
        url: &str,
        nodes: Vec<SemanticNode>,
        det: CaptureDeterminism,
        hit_tests: Option<BTreeMap<String, HitTestEntry>>,
    ) -> CaptureBundle {
        CaptureBundle {
            schema_version: "1.1".to_string(),
            captured_at: "2026-01-01T00:00:00Z".to_string(),
            viewport: make_viewport_cfg(),
            environment: make_env(),
            determinism: det,
            page: make_page(url, nodes),
            computed_styles: BTreeMap::new(),
            screenshots: Screenshots {
                full_page: "desktop/old.png".to_string(),
                viewport: "desktop/old-vp.png".to_string(),
            },
            style_candidates: StyleCandidates::default(),
            hit_tests,
            pseudo_elements: None,
            pseudo_truncated: None,
        }
    }

    fn make_bundle(
        url: &str,
        nodes: Vec<SemanticNode>,
        hit_tests: Option<BTreeMap<String, HitTestEntry>>,
    ) -> CaptureBundle {
        make_bundle_full(url, nodes, make_det(), hit_tests)
    }

    fn make_node(
        id: &str,
        seq_index: u32,
        text: Option<&str>,
        href: Option<&str>,
        landmark: Option<&str>,
    ) -> SemanticNode {
        SemanticNode {
            id: id.to_string(),
            kind: "link".to_string(),
            role: Some("link".to_string()),
            text: text.map(str::to_string),
            acc_name: None,
            href: href.map(str::to_string),
            image_alt: None,
            bbox: [10, 20, 150, 40],
            seq_index,
            anchors: NodeAnchors {
                text: text.map(str::to_string),
                role: Some("link".to_string()),
                href: href.map(str::to_string),
                alt: None,
                aria_label: None,
                nearest_heading: None,
                landmark: landmark.map(str::to_string),
                ordinal_in_landmark: Some(1),
            },
            css_selector: Some(format!("#{}", id)),
            raw_href: href.map(str::to_string),
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

    fn profile() -> SeverityResolver {
        SeverityResolver::from_profile(ParityProfile::ContentStructure)
    }

    fn pt(o: HitTestOutcome, winner: Option<&str>) -> HitTestPoint {
        HitTestPoint {
            o,
            winner: winner.map(str::to_string),
        }
    }

    fn sampled(points: Vec<HitTestPoint>) -> HitTestEntry {
        HitTestEntry {
            status: HitTestStatus::Sampled,
            skip_reason: None,
            grid_size: Some(5),
            points: Some(points),
        }
    }

    fn skipped(reason: HitTestSkipReason) -> HitTestEntry {
        HitTestEntry {
            status: HitTestStatus::Skipped,
            skip_reason: Some(reason),
            grid_size: None,
            points: None,
        }
    }

    /// 25 points, all `Hit`.
    fn all_hit(n: usize) -> Vec<HitTestPoint> {
        (0..n).map(|_| pt(HitTestOutcome::Hit, None)).collect()
    }

    fn hit_map(entries: &[(&str, HitTestEntry)]) -> BTreeMap<String, HitTestEntry> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    // -----------------------------------------------------------------------
    // format_miss_winners / top_miss_winner
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_miss_winners_count_desc_then_selector_asc() {
        let mut winners = BTreeMap::new();
        winners.insert("b_sel".to_string(), 5);
        winners.insert("a_sel".to_string(), 5);
        winners.insert("c_sel".to_string(), 1);
        assert_eq!(
            format_miss_winners(&winners),
            "a_sel (x5); b_sel (x5); c_sel (x1)"
        );
    }

    #[test]
    fn test_format_miss_winners_truncates_to_top_3() {
        let mut winners = BTreeMap::new();
        winners.insert("img.sibling-photo".to_string(), 10);
        winners.insert(".overlay-banner".to_string(), 8);
        winners.insert(".nav-fixed".to_string(), 3);
        winners.insert(".footer-block".to_string(), 1);
        assert_eq!(
            format_miss_winners(&winners),
            "img.sibling-photo (x10); .overlay-banner (x8); .nav-fixed (x3)"
        );
    }

    #[test]
    fn test_top_miss_winner_picks_highest_count() {
        let mut winners = BTreeMap::new();
        winners.insert("low".to_string(), 1);
        winners.insert("high".to_string(), 9);
        assert_eq!(top_miss_winner(&winners), Some("high".to_string()));
    }

    #[test]
    fn test_format_miss_winners_empty() {
        let winners: BTreeMap<String, u32> = BTreeMap::new();
        assert_eq!(format_miss_winners(&winners), "");
        assert_eq!(top_miss_winner(&winners), None);
    }

    // -----------------------------------------------------------------------
    // tally_points: exclusion / both-miss-drop arithmetic (design brief steps 1-2)
    // -----------------------------------------------------------------------

    /// U7 scenario: parity rule — both sides occluded identically (12/25 hits,
    /// same 13 miss indices on both sides) -> denominator drops to 12, both
    /// adjusted fractions are 1.0 (no residual delta).
    #[test]
    fn test_tally_both_sides_identical_occlusion_no_delta() {
        let mut old_points = all_hit(25);
        let mut new_points = all_hit(25);
        for i in 12..25 {
            old_points[i] = pt(HitTestOutcome::Miss, Some(".decor"));
            new_points[i] = pt(HitTestOutcome::Miss, Some(".decor"));
        }
        let tally = tally_points(&old_points, &new_points).expect("comparable");
        assert_eq!(tally.excluded, 0);
        assert_eq!(tally.denominator, 12);
        assert_eq!(tally.hits_old, 12);
        assert_eq!(tally.hits_new, 12);
    }

    /// U7 scenario: pill CTA — corners `clipped` on both sides excludes 4 of 25
    /// points, denominator 21, both sides fully hit on the rest.
    #[test]
    fn test_tally_pill_corners_clipped_both_sides() {
        let mut old_points = all_hit(25);
        let mut new_points = all_hit(25);
        for i in [0usize, 4, 20, 24] {
            old_points[i] = pt(HitTestOutcome::Clipped, None);
            new_points[i] = pt(HitTestOutcome::Clipped, None);
        }
        let tally = tally_points(&old_points, &new_points).expect("comparable");
        assert_eq!(tally.excluded, 4);
        assert_eq!(tally.denominator, 21);
        assert_eq!(tally.hits_old, 21);
        assert_eq!(tally.hits_new, 21);
    }

    /// U7 scenario: same pill CTA, but new drops to 10/21 on the surviving
    /// (non-corner) points.
    #[test]
    fn test_tally_pill_new_drops_to_10_of_21() {
        let mut old_points = all_hit(25);
        let mut new_points = all_hit(25);
        for i in [0usize, 4, 20, 24] {
            old_points[i] = pt(HitTestOutcome::Clipped, None);
            new_points[i] = pt(HitTestOutcome::Clipped, None);
        }
        // Of the 21 surviving indices, miss 11 of them on the new side.
        let mut missed = 0;
        for i in 0..25 {
            if [0usize, 4, 20, 24].contains(&i) {
                continue;
            }
            if missed < 11 {
                new_points[i] = pt(HitTestOutcome::Miss, Some("div.overlay"));
                missed += 1;
            }
        }
        let tally = tally_points(&old_points, &new_points).expect("comparable");
        assert_eq!(tally.excluded, 4);
        assert_eq!(tally.denominator, 21);
        assert_eq!(tally.hits_old, 21);
        assert_eq!(tally.hits_new, 10);
    }

    /// U7 scenario: smaller/rounder port — old corners hit, new corners
    /// `clipped` to the (smaller) parent; interior fully hit on both sides.
    /// The clipped-on-new-only corners still get excluded (either side
    /// excluded is enough).
    #[test]
    fn test_tally_smaller_rounder_port_new_clipped_corners() {
        let old_points = all_hit(25); // old corners are hits (full-size button)
        let mut new_points = all_hit(25);
        for i in [0usize, 4, 20, 24] {
            new_points[i] = pt(HitTestOutcome::Clipped, None);
        }
        let tally = tally_points(&old_points, &new_points).expect("comparable");
        assert_eq!(tally.excluded, 4);
        assert_eq!(tally.denominator, 21);
        assert_eq!(tally.hits_old, 21);
        assert_eq!(tally.hits_new, 21);
    }

    /// U7 scenario: asymmetric heights — old-only `offViewport` points must be
    /// excluded regardless of what the new side recorded at the same index
    /// (never a "phantom miss" for new).
    #[test]
    fn test_tally_asymmetric_offviewport_excludes_regardless_of_new_outcome() {
        let mut old_points = all_hit(25);
        let mut new_points = all_hit(25);
        for i in 20..25 {
            old_points[i] = pt(HitTestOutcome::OffViewport, None);
            // If exclusion didn't happen first, this would look like a new-side
            // miss and wrongly drag the new fraction down.
            new_points[i] = pt(HitTestOutcome::Miss, Some("div.decoy"));
        }
        let tally = tally_points(&old_points, &new_points).expect("comparable");
        assert_eq!(tally.excluded, 5);
        assert_eq!(tally.denominator, 20);
        assert_eq!(tally.hits_old, 20);
        assert_eq!(tally.hits_new, 20);
        assert!(
            tally.new_miss_winners.is_empty(),
            "excluded indices must never contribute to new_miss_winners"
        );
    }

    // -----------------------------------------------------------------------
    // clickable_area_issues: integration scenarios (design brief U7 tests)
    // -----------------------------------------------------------------------

    /// Motivating defect: old fully clickable (25/25), new mostly occluded by a
    /// sibling image (3/25) -> one Error `clickable_area_regressed` issue, with
    /// the top-3 miss winners in evidence and the top winner in remediation.
    #[test]
    fn test_motivating_defect_fires_with_winners_in_evidence_and_remediation() {
        let old_node = make_node("n_cta", 3, Some("Get started"), Some("/signup"), Some("main"));
        let new_node = make_node("n_cta", 3, Some("Get started"), Some("/signup"), Some("main"));

        let old_points = all_hit(25);
        // 3 hits, 22 misses distributed across winners:
        // img.sibling-photo x10, .overlay-banner x8, .nav-fixed x3, .footer-block x1
        let mut new_points = all_hit(3);
        new_points.extend(std::iter::repeat_with(|| pt(HitTestOutcome::Miss, Some("img.sibling-photo"))).take(10));
        new_points.extend(std::iter::repeat_with(|| pt(HitTestOutcome::Miss, Some(".overlay-banner"))).take(8));
        new_points.extend(std::iter::repeat_with(|| pt(HitTestOutcome::Miss, Some(".nav-fixed"))).take(3));
        new_points.extend(std::iter::repeat_with(|| pt(HitTestOutcome::Miss, Some(".footer-block"))).take(1));
        assert_eq!(new_points.len(), 25);

        let old_hit_tests = hit_map(&[("n_cta", sampled(old_points))]);
        let new_hit_tests = hit_map(&[("n_cta", sampled(new_points))]);

        let old_bundle = make_bundle("http://old.example.com/", vec![old_node], Some(old_hit_tests));
        let new_bundle = make_bundle("http://new.example.com/", vec![new_node], Some(new_hit_tests));
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = clickable_area_issues(
            &old_bundle,
            &new_bundle,
            &outcome,
            "desktop",
            &profile(),
            false,
        );

        assert_eq!(issues.len(), 1, "exactly one clickable_area_regressed issue");
        let issue = &issues[0];
        assert_eq!(issue.issue_type, IssueType::ClickableAreaRegressed);
        assert_eq!(issue.category, IssueCategory::Visual);
        assert_eq!(issue.severity, crate::contract::IssueSeverity::Error);
        assert_eq!(issue.goal, None);
        assert_eq!(issue.confidence, 0.9);

        assert_eq!(issue.evidence["old"]["hitFraction"], "1.0000");
        assert_eq!(issue.evidence["old"]["rawHits"], "25/25");
        assert_eq!(issue.evidence["new"]["hitFraction"], "0.1200");
        assert_eq!(issue.evidence["new"]["rawHits"], "3/25");
        assert_eq!(issue.evidence["excludedPoints"], "0");
        assert_eq!(
            issue.evidence["new"]["missWinners"],
            "img.sibling-photo (x10); .overlay-banner (x8); .nav-fixed (x3)"
        );

        let remediation = issue.remediation.as_ref().expect("remediation present");
        assert_eq!(remediation["action"], "restore_clickable_area");
        let grep = remediation["findBy"]["grep"]
            .as_array()
            .expect("grep array");
        let grep_strs: Vec<&str> = grep.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(grep_strs.contains(&"\"/signup\""));
        assert!(grep_strs.contains(&"Get started"));
        assert!(grep_strs.contains(&"img.sibling-photo"));
        assert!(remediation["note"]
            .as_str()
            .unwrap()
            .contains("img.sibling-photo"));
    }

    /// U7 scenario: both sides occluded identically (12/25, same miss indices)
    /// -> no issue (the parity rule, not raw occlusion).
    #[test]
    fn test_identical_occlusion_both_sides_no_issue() {
        let old_node = make_node("n1", 0, None, None, None);
        let new_node = make_node("n1", 0, None, None, None);

        let mut old_points = all_hit(25);
        let mut new_points = all_hit(25);
        for i in 12..25 {
            old_points[i] = pt(HitTestOutcome::Miss, Some(".decor"));
            new_points[i] = pt(HitTestOutcome::Miss, Some(".decor"));
        }

        let old_bundle = make_bundle(
            "http://old.example.com/",
            vec![old_node],
            Some(hit_map(&[("n1", sampled(old_points))])),
        );
        let new_bundle = make_bundle(
            "http://new.example.com/",
            vec![new_node],
            Some(hit_map(&[("n1", sampled(new_points))])),
        );
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = clickable_area_issues(&old_bundle, &new_bundle, &outcome, "desktop", &profile(), false);
        assert!(issues.is_empty(), "identical occlusion must not fire");
    }

    /// U7 scenario: pill CTA — corners clipped both sides (denominator 21,
    /// 21/21 both sides) -> no issue; new drops to 10/21 -> fires.
    #[test]
    fn test_pill_cta_no_issue_then_fires_on_drop() {
        let corners = [0usize, 4, 20, 24];
        let build_points = |new_miss_count: usize| {
            let mut old_points = all_hit(25);
            let mut new_points = all_hit(25);
            for &i in &corners {
                old_points[i] = pt(HitTestOutcome::Clipped, None);
                new_points[i] = pt(HitTestOutcome::Clipped, None);
            }
            let mut missed = 0;
            for i in 0..25 {
                if corners.contains(&i) {
                    continue;
                }
                if missed < new_miss_count {
                    new_points[i] = pt(HitTestOutcome::Miss, Some("div.overlay"));
                    missed += 1;
                }
            }
            (old_points, new_points)
        };

        let make_pair_bundles = |new_miss_count: usize| {
            let (old_points, new_points) = build_points(new_miss_count);
            let old_node = make_node("n1", 0, None, None, None);
            let new_node = make_node("n1", 0, None, None, None);
            let old_bundle = make_bundle(
                "http://old.example.com/",
                vec![old_node],
                Some(hit_map(&[("n1", sampled(old_points))])),
            );
            let new_bundle = make_bundle(
                "http://new.example.com/",
                vec![new_node],
                Some(hit_map(&[("n1", sampled(new_points))])),
            );
            (old_bundle, new_bundle)
        };

        // No drop: 21/21 both sides -> no issue.
        let (old_bundle, new_bundle) = make_pair_bundles(0);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);
        let issues = clickable_area_issues(&old_bundle, &new_bundle, &outcome, "desktop", &profile(), false);
        assert!(issues.is_empty(), "21/21 both sides must not fire");

        // Drop to 10/21 -> fires.
        let (old_bundle, new_bundle) = make_pair_bundles(11);
        let issues = clickable_area_issues(&old_bundle, &new_bundle, &outcome, "desktop", &profile(), false);
        assert_eq!(issues.len(), 1, "10/21 must fire");
        assert_eq!(issues[0].evidence["new"]["rawHits"], "10/21");
    }

    /// U7 scenario: smaller/rounder port must NOT fire `clickable_area_regressed`
    /// (that regression class belongs to the style channel).
    #[test]
    fn test_smaller_rounder_port_no_issue() {
        let old_node = make_node("n1", 0, None, None, None);
        let new_node = make_node("n1", 0, None, None, None);

        let old_points = all_hit(25);
        let mut new_points = all_hit(25);
        for i in [0usize, 4, 20, 24] {
            new_points[i] = pt(HitTestOutcome::Clipped, None);
        }

        let old_bundle = make_bundle(
            "http://old.example.com/",
            vec![old_node],
            Some(hit_map(&[("n1", sampled(old_points))])),
        );
        let new_bundle = make_bundle(
            "http://new.example.com/",
            vec![new_node],
            Some(hit_map(&[("n1", sampled(new_points))])),
        );
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = clickable_area_issues(&old_bundle, &new_bundle, &outcome, "desktop", &profile(), false);
        assert!(issues.is_empty());
    }

    /// U7 scenario: adjusted-floor case — raw old hits 22/25 with 3 both-side
    /// misses dropped -> adjusted old fraction is 1.0 (eligible), and the
    /// detector still fires when the new side additionally regresses on
    /// surviving points.
    #[test]
    fn test_adjusted_floor_eligible_despite_raw_22_of_25() {
        let old_node = make_node("n1", 0, None, None, None);
        let new_node = make_node("n1", 0, None, None, None);

        let both_miss = [5usize, 10, 15];
        let mut old_points = all_hit(25);
        let mut new_points = all_hit(25);
        for &i in &both_miss {
            old_points[i] = pt(HitTestOutcome::Miss, Some(".decor"));
            new_points[i] = pt(HitTestOutcome::Miss, Some(".decor"));
        }
        // 5 additional new-side-only misses among the surviving 22 points.
        let mut missed = 0;
        for i in 0..25 {
            if both_miss.contains(&i) {
                continue;
            }
            if missed < 5 {
                new_points[i] = pt(HitTestOutcome::Miss, Some("div.overlay"));
                missed += 1;
            }
        }

        let old_bundle = make_bundle(
            "http://old.example.com/",
            vec![old_node],
            Some(hit_map(&[("n1", sampled(old_points))])),
        );
        let new_bundle = make_bundle(
            "http://new.example.com/",
            vec![new_node],
            Some(hit_map(&[("n1", sampled(new_points))])),
        );
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = clickable_area_issues(&old_bundle, &new_bundle, &outcome, "desktop", &profile(), false);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].evidence["old"]["hitFraction"], "1.0000");
        assert_eq!(issues[0].evidence["old"]["rawHits"], "22/22");
        assert_eq!(issues[0].evidence["new"]["rawHits"], "17/22");
    }

    /// U7 scenario: old adjusted fraction 0.85 (below the 0.9 floor) never
    /// fires, regardless of how low the new side is.
    #[test]
    fn test_old_below_floor_never_fires() {
        let old_node = make_node("n1", 0, None, None, None);
        let new_node = make_node("n1", 0, None, None, None);

        // Partition the 25 points so the both-side-miss drop (design brief
        // step 2) doesn't interact with the deliberate old-side misses:
        //   indices 0..5   : both old and new miss -> dropped (not counted)
        //   indices 5..8    : old miss, new hit     -> counted (old-only miss)
        //   indices 8..25   : old hit, new miss      -> counted (new-only miss)
        // denominator = 3 + 17 = 20; hits_old = 17 -> adjustedOld = 17/20 = 0.85;
        // hits_new = 3 -> adjustedNew = 3/20 = 0.15 (a huge delta, but the old
        // floor must still suppress the issue).
        let mut old_points = all_hit(25);
        let mut new_points = all_hit(25);
        for i in 0..5 {
            old_points[i] = pt(HitTestOutcome::Miss, Some(".decor"));
            new_points[i] = pt(HitTestOutcome::Miss, Some(".decor"));
        }
        for i in 5..8 {
            old_points[i] = pt(HitTestOutcome::Miss, Some(".decor"));
        }
        for i in 8..25 {
            new_points[i] = pt(HitTestOutcome::Miss, Some("div.overlay"));
        }

        let old_bundle = make_bundle(
            "http://old.example.com/",
            vec![old_node],
            Some(hit_map(&[("n1", sampled(old_points))])),
        );
        let new_bundle = make_bundle(
            "http://new.example.com/",
            vec![new_node],
            Some(hit_map(&[("n1", sampled(new_points))])),
        );
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = clickable_area_issues(&old_bundle, &new_bundle, &outcome, "desktop", &profile(), false);
        assert!(issues.is_empty(), "old adjusted 0.85 must never fire");
    }

    /// U7 scenario: one side `skipped(tooSmall)`, the other measured -> no
    /// issue, no junk delta.
    #[test]
    fn test_one_side_skipped_too_small_no_issue() {
        let old_node = make_node("n1", 0, None, None, None);
        let new_node = make_node("n1", 0, None, None, None);

        let old_bundle = make_bundle(
            "http://old.example.com/",
            vec![old_node],
            Some(hit_map(&[("n1", sampled(all_hit(25)))])),
        );
        let new_bundle = make_bundle(
            "http://new.example.com/",
            vec![new_node],
            Some(hit_map(&[("n1", skipped(HitTestSkipReason::TooSmall))])),
        );
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = clickable_area_issues(&old_bundle, &new_bundle, &outcome, "desktop", &profile(), false);
        assert!(issues.is_empty());
    }

    /// U7 error path: old bundle lacks the `hitTests` channel entirely ->
    /// zero clickable issues (the capability_mismatch warning is a separate,
    /// orchestrate-level concern — see orchestrate.rs tests).
    #[test]
    fn test_old_bundle_missing_channel_zero_issues() {
        let old_node = make_node("n1", 0, None, None, None);
        let new_node = make_node("n1", 0, None, None, None);

        let old_bundle = make_bundle("http://old.example.com/", vec![old_node], None);
        let new_bundle = make_bundle(
            "http://new.example.com/",
            vec![new_node],
            Some(hit_map(&[("n1", sampled(all_hit(25)))])),
        );
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = clickable_area_issues(&old_bundle, &new_bundle, &outcome, "desktop", &profile(), false);
        assert!(issues.is_empty());
    }

    /// U7 error path: both bundles lack the channel (frozen-pair replay) ->
    /// zero clickable issues.
    #[test]
    fn test_both_bundles_missing_channel_zero_issues() {
        let old_node = make_node("n1", 0, None, None, None);
        let new_node = make_node("n1", 0, None, None, None);

        let old_bundle = make_bundle("http://old.example.com/", vec![old_node], None);
        let new_bundle = make_bundle("http://new.example.com/", vec![new_node], None);
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = clickable_area_issues(&old_bundle, &new_bundle, &outcome, "desktop", &profile(), false);
        assert!(issues.is_empty());
    }

    fn motivating_defect_bundles() -> (CaptureBundle, CaptureBundle, MatchOutcome) {
        let old_node = make_node("n_cta", 3, Some("Get started"), Some("/signup"), Some("main"));
        let new_node = make_node("n_cta", 3, Some("Get started"), Some("/signup"), Some("main"));
        let old_points = all_hit(25);
        let mut new_points = all_hit(3);
        new_points.extend(std::iter::repeat_with(|| pt(HitTestOutcome::Miss, Some("img.sibling-photo"))).take(22));
        let old_bundle = make_bundle(
            "http://old.example.com/",
            vec![old_node],
            Some(hit_map(&[("n_cta", sampled(old_points))])),
        );
        let new_bundle = make_bundle(
            "http://new.example.com/",
            vec![new_node],
            Some(hit_map(&[("n_cta", sampled(new_points))])),
        );
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);
        (old_bundle, new_bundle, outcome)
    }

    /// Id stability: re-analyzing the same fixture twice yields the same issue id.
    #[test]
    fn test_id_stability_same_fixture_reanalyzed() {
        let (old_bundle, new_bundle, outcome) = motivating_defect_bundles();
        let issues1 = clickable_area_issues(&old_bundle, &new_bundle, &outcome, "desktop", &profile(), false);
        let issues2 = clickable_area_issues(&old_bundle, &new_bundle, &outcome, "desktop", &profile(), false);
        assert_eq!(issues1.len(), 1);
        assert_eq!(issues2.len(), 1);
        assert_eq!(issues1[0].id, issues2[0].id);
        assert!(issues1[0].id.starts_with("issue_"));
    }

    /// Confidence demotion: absent settle/quiescence fields (pre-settle
    /// bundles) mean NO demotion — base confidence unchanged.
    #[test]
    fn test_confidence_no_demotion_when_settle_fields_absent() {
        let (old_bundle, new_bundle, outcome) = motivating_defect_bundles();
        let issues = clickable_area_issues(&old_bundle, &new_bundle, &outcome, "desktop", &profile(), false);
        assert_eq!(issues[0].confidence, base_confidence::CLICKABLE_AREA_REGRESSED);
    }

    /// Confidence demotion: `quiescence == timeout` on either side demotes by
    /// the CLICKABLE_SETTLE_DEMOTION multiplier.
    #[test]
    fn test_confidence_demoted_on_quiescence_timeout() {
        let (old_bundle, mut new_bundle, outcome) = motivating_defect_bundles();
        new_bundle.determinism.quiescence = Some(QuiescenceStatus::Timeout);

        let issues = clickable_area_issues(&old_bundle, &new_bundle, &outcome, "desktop", &profile(), false);
        assert_eq!(issues.len(), 1);
        let expected = round4(base_confidence::CLICKABLE_AREA_REGRESSED * CLICKABLE_SETTLE_DEMOTION);
        assert_eq!(issues[0].confidence, expected);
    }

    /// Confidence demotion: `settle == failed` on either side demotes.
    #[test]
    fn test_confidence_demoted_on_settle_failed() {
        let (mut old_bundle, new_bundle, outcome) = motivating_defect_bundles();
        old_bundle.determinism.settle = Some(StepStatus::Failed);

        let issues = clickable_area_issues(&old_bundle, &new_bundle, &outcome, "desktop", &profile(), false);
        assert_eq!(issues.len(), 1);
        let expected = round4(base_confidence::CLICKABLE_AREA_REGRESSED * CLICKABLE_SETTLE_DEMOTION);
        assert_eq!(issues[0].confidence, expected);
    }

    /// Confidence demotion: `settle == skipped` on either side demotes.
    #[test]
    fn test_confidence_demoted_on_settle_skipped() {
        let (mut old_bundle, new_bundle, outcome) = motivating_defect_bundles();
        old_bundle.determinism.settle = Some(StepStatus::Skipped);

        let issues = clickable_area_issues(&old_bundle, &new_bundle, &outcome, "desktop", &profile(), false);
        assert_eq!(issues.len(), 1);
        let expected = round4(base_confidence::CLICKABLE_AREA_REGRESSED * CLICKABLE_SETTLE_DEMOTION);
        assert_eq!(issues[0].confidence, expected);
    }

    /// Denominator floor: a comparable-but-tiny surviving sample (< 9) must
    /// not fire even though the raw ratios would otherwise qualify.
    #[test]
    fn test_min_denominator_floor_suppresses_issue() {
        let old_node = make_node("n1", 0, None, None, None);
        let new_node = make_node("n1", 0, None, None, None);

        // Exclude (clipped) all but 5 points on both sides -> denominator 5 < 9.
        let mut old_points = all_hit(25);
        let mut new_points = all_hit(25);
        for i in 5..25 {
            old_points[i] = pt(HitTestOutcome::Clipped, None);
            new_points[i] = pt(HitTestOutcome::Clipped, None);
        }
        for i in 0..5 {
            new_points[i] = pt(HitTestOutcome::Miss, Some("div.overlay"));
        }

        let old_bundle = make_bundle(
            "http://old.example.com/",
            vec![old_node],
            Some(hit_map(&[("n1", sampled(old_points))])),
        );
        let new_bundle = make_bundle(
            "http://new.example.com/",
            vec![new_node],
            Some(hit_map(&[("n1", sampled(new_points))])),
        );
        let outcome = make_outcome(vec![make_matched_pair(0, 0)]);

        let issues = clickable_area_issues(&old_bundle, &new_bundle, &outcome, "desktop", &profile(), false);
        assert!(issues.is_empty(), "denominator below floor must suppress the issue");
    }
}
