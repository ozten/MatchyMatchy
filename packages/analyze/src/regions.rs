//! Structural-saturation rollup: compute per-ARIA-landmark `Region` work items.
//!
//! DETERMINISM:
//! - All grouping uses BTreeMap so key order is stable.
//! - Member id vecs are sorted ascending.
//! - Final array sort is a total order: (saturation DESC, id ASC).
//! - sha256 hashed from landmark name only — ordinal-independent, stable across re-captures.
//!
//! CALIBRATION (p01-hiya-number-registration, desktop, recorded 2026-06-17):
//!   contentinfo: 51 old nodes, 44 structural issues → saturation 0.863 → emits
//!   main:        60 old nodes,  1 structural issue  → saturation 0.017 → does not emit
//!   banner:       4 old nodes  (below MIN_NODE_COUNT=10)          → does not emit
//!   navigation:   8 old nodes  (below MIN_NODE_COUNT=10)          → does not emit
//! Exactly one rollup on p01. Constants frozen at these values.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::contract::{Issue, IssueSeverity, IssueType, Region};

/// Minimum structural-saturation ratio required to emit a region rollup.
/// Calibrated on p01: contentinfo=0.86 (emits), main=0.02 (does not).
const SATURATION_THRESHOLD: f64 = 0.6;

/// Minimum old-side node count in the landmark required to emit a region rollup.
/// Calibrated on p01: banner=4, navigation=8 (both below floor); contentinfo=51, main=60 (above).
const MIN_NODE_COUNT: u32 = 10;

/// Returns true iff this issue type counts toward the structural saturation numerator.
///
/// Structural = node loss / structural modification. Excludes style/content-modification/
/// hygiene/technical/a11y types, and deliberately excludes `ChangedLinkTarget` /
/// `ChangedLinkText` (node modification, not node loss — key technical decision).
fn is_structural(t: &IssueType) -> bool {
    matches!(
        t,
        IssueType::MissingTitle
            | IssueType::MissingMetaDescription
            | IssueType::MissingH1
            | IssueType::MissingText
            | IssueType::MissingLink
            | IssueType::MissingImage
            | IssueType::MissingForm
            | IssueType::MissingFormField
            | IssueType::MissingSubmit
            | IssueType::MissingButton
            | IssueType::MissingAltText
            | IssueType::BrokenLink
            | IssueType::BrokenImage
            | IssueType::HeadingStructureChanged
            | IssueType::ComponentReordered
            | IssueType::ComponentSwapped
    )
}

/// Compute a stable region id from the landmark name alone.
///
/// Format: `region_` + first 12 hex chars of sha256("region\x1f{landmark}").
/// Landmark-only key means the id is ordinal-independent and survives re-captures
/// even as member issue ids churn (R5).
fn region_id(landmark: &str) -> String {
    let canonical = format!("region\x1f{}", landmark);
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    let hex_str = hex::encode(hasher.finalize());
    format!("region_{}", &hex_str[..12])
}

/// Compute structural-saturation region rollups for all ARIA landmarks.
///
/// For each real landmark (not None / "(none)"):
///   structural_count = count of kept issues of structural type anchored there
///   old_node_count   = from old_landmark_node_counts; skip if 0 (denominator guard)
///   denominator      = max(old_node_count, structural_count)  — broken_* burst guard
///   saturation       = (structural_count / denominator).clamp(0.0, 1.0)
///   emit iff saturation >= SATURATION_THRESHOLD AND old_node_count >= MIN_NODE_COUNT
///
/// member_issue_ids = ALL kept issues anchored to the landmark (structural + style), sorted asc.
/// severity         = worst member severity (max by rank()).
/// Returns Vec<Region> sorted by (saturation DESC, id ASC).
pub fn compute_regions(
    kept: &[Issue],
    old_landmark_node_counts: &BTreeMap<String, u32>,
) -> Vec<Region> {
    // Group ALL kept issues by their landmark into a BTreeMap (deterministic order).
    // Issues with no landmark (None) are skipped immediately.
    let mut groups: BTreeMap<String, Vec<&Issue>> = BTreeMap::new();
    for issue in kept {
        if let Some(lm) = &issue.locator.anchors.landmark {
            // Defensively skip the literal "(none)" key too.
            if lm == "(none)" {
                continue;
            }
            groups.entry(lm.clone()).or_default().push(issue);
        }
    }

    let mut regions: Vec<Region> = Vec::new();

    for (landmark, members) in groups {
        let structural_count = members
            .iter()
            .filter(|i| is_structural(&i.issue_type))
            .count() as u32;

        let old_node_count = old_landmark_node_counts
            .get(&landmark)
            .copied()
            .unwrap_or(0);

        // Denominator guard: skip if no old nodes for this landmark.
        if old_node_count == 0 {
            continue;
        }

        // broken_* burst guard: denominator can't be less than old_node_count or structural_count.
        let denominator = old_node_count.max(structural_count);
        let saturation = (structural_count as f64 / denominator as f64).clamp(0.0, 1.0);

        // Emit gate: saturation threshold AND raw old-node count floor.
        if saturation < SATURATION_THRESHOLD || old_node_count < MIN_NODE_COUNT {
            continue;
        }

        // member_issue_ids: ALL members (structural + style), sorted ascending.
        let mut member_issue_ids: Vec<String> = members.iter().map(|i| i.id.clone()).collect();
        member_issue_ids.sort();

        // severity: worst member (max by rank()).
        let severity = members
            .iter()
            .map(|i| &i.severity)
            .max_by_key(|s| s.rank())
            .cloned()
            .unwrap_or(IssueSeverity::Info);

        let id = region_id(&landmark);

        let summary = format!(
            "{} region: {}/{} old nodes structurally changed, {} issues claimed",
            landmark,
            structural_count,
            old_node_count,
            member_issue_ids.len()
        );

        regions.push(Region {
            id,
            landmark,
            saturation,
            structural_count,
            old_node_count,
            member_issue_ids,
            severity,
            summary,
        });
    }

    // Sort: saturation DESC, then id ASC (total order for byte-stability).
    regions.sort_by(|a, b| {
        b.saturation
            .partial_cmp(&a.saturation)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });

    regions
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{Anchors, IssueCategory, IssueSeverity, IssueType, Locator};

    /// Build a minimal Issue for testing.
    fn make_issue(
        id: &str,
        issue_type: IssueType,
        severity: IssueSeverity,
        landmark: Option<&str>,
    ) -> Issue {
        Issue {
            id: id.to_string(),
            issue_type,
            category: IssueCategory::Content,
            severity,
            confidence: 0.9,
            viewport: "desktop".to_string(),
            locale: None,
            goal: None,
            message: "test".to_string(),
            locator: Locator {
                anchors: Anchors {
                    text: None,
                    role: None,
                    href: None,
                    alt: None,
                    aria_label: None,
                    nearest_heading: None,
                    landmark: landmark.map(str::to_string),
                    ordinal_in_landmark: None,
                },
                css_selector_old: None,
                css_selector_new: None,
                bbox_old: None,
                bbox_new: None,
                seq_index_old: None,
                seq_index_new: None,
            },
            evidence: serde_json::json!({}),
            remediation: None,
        }
    }

    // -----------------------------------------------------------------------
    // AE1: Happy path — contentinfo rolls up, main does not
    // -----------------------------------------------------------------------

    /// AE1: contentinfo with 44 structural issues over 51 old nodes → one region.
    /// main with 1 structural over 60 old nodes → no region.
    #[test]
    fn test_ae1_contentinfo_rolls_up_main_does_not() {
        let mut kept: Vec<Issue> = Vec::new();

        // 44 structural (MissingText) in contentinfo
        for i in 0..44u32 {
            kept.push(make_issue(
                &format!("ci_s_{:04}", i),
                IssueType::MissingText,
                IssueSeverity::Error,
                Some("contentinfo"),
            ));
        }
        // A few style issues in contentinfo too (to verify member_issue_ids includes them)
        kept.push(make_issue(
            "ci_style_0000",
            IssueType::StyleChanged,
            IssueSeverity::Warning,
            Some("contentinfo"),
        ));

        // 1 structural in main (BrokenLink)
        kept.push(make_issue(
            "main_s_0000",
            IssueType::BrokenLink,
            IssueSeverity::Error,
            Some("main"),
        ));

        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        counts.insert("contentinfo".to_string(), 51);
        counts.insert("main".to_string(), 60);

        let regions = compute_regions(&kept, &counts);

        // Exactly one region: contentinfo
        assert_eq!(regions.len(), 1, "exactly one region must be emitted");
        let r = &regions[0];
        assert_eq!(r.landmark, "contentinfo");

        // saturation = 44 / max(51, 44) = 44/51 ≈ 0.863
        let expected_sat = 44.0_f64 / 51.0_f64;
        assert!(
            (r.saturation - expected_sat).abs() < 1e-9,
            "saturation must be ≈ {:.6}, got {:.6}",
            expected_sat,
            r.saturation
        );

        assert_eq!(r.structural_count, 44);
        assert_eq!(r.old_node_count, 51);

        // severity = worst member = Error (from the 44 MissingText issues)
        assert_eq!(r.severity, IssueSeverity::Error);

        // member_issue_ids includes both structural AND style members (44 + 1 = 45)
        assert_eq!(
            r.member_issue_ids.len(),
            45,
            "all 45 members (44 structural + 1 style) must be claimed"
        );

        // member_issue_ids must be sorted ascending
        let mut sorted = r.member_issue_ids.clone();
        sorted.sort();
        assert_eq!(r.member_issue_ids, sorted, "member_issue_ids must be sorted");

        // region id matches the pattern region_{12hex}
        assert!(
            r.id.starts_with("region_"),
            "id must start with 'region_'"
        );
        assert_eq!(r.id.len(), 7 + 12, "id must be region_ + 12 hex chars");
    }

    // -----------------------------------------------------------------------
    // AE2: Below-count gate
    // -----------------------------------------------------------------------

    /// AE2: banner at high structural ratio but old_node_count 4 → no region.
    #[test]
    fn test_ae2_below_min_node_count() {
        let kept = vec![
            make_issue("bn_s_0000", IssueType::MissingText, IssueSeverity::Error, Some("banner")),
            make_issue("bn_s_0001", IssueType::MissingText, IssueSeverity::Error, Some("banner")),
            make_issue("bn_s_0002", IssueType::MissingText, IssueSeverity::Error, Some("banner")),
            make_issue("bn_s_0003", IssueType::MissingText, IssueSeverity::Error, Some("banner")),
        ];
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        // 4 old nodes — below MIN_NODE_COUNT=10
        counts.insert("banner".to_string(), 4);

        let regions = compute_regions(&kept, &counts);
        assert!(
            regions.is_empty(),
            "banner below MIN_NODE_COUNT must not emit a region"
        );
    }

    // -----------------------------------------------------------------------
    // Boundary tests
    // -----------------------------------------------------------------------

    /// saturation exactly 0.6 with exactly 10 old nodes → emitted.
    #[test]
    fn test_boundary_exactly_at_threshold_emitted() {
        // 6 structural / 10 old nodes = 0.6 exactly
        let kept: Vec<Issue> = (0..6u32)
            .map(|i| {
                make_issue(
                    &format!("lm_s_{:04}", i),
                    IssueType::MissingText,
                    IssueSeverity::Warning,
                    Some("navigation"),
                )
            })
            .collect();
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        counts.insert("navigation".to_string(), 10);

        let regions = compute_regions(&kept, &counts);
        assert_eq!(
            regions.len(),
            1,
            "saturation 0.6 with 10 old nodes must emit one region"
        );
        let sat = regions[0].saturation;
        assert!(
            (sat - 0.6).abs() < 1e-9,
            "saturation must be exactly 0.6, got {}",
            sat
        );
    }

    /// 9 old nodes (else identical: 6 structural / 9 = 0.667 ≥ 0.6) → not emitted (MIN_NODE_COUNT).
    #[test]
    fn test_boundary_nine_old_nodes_not_emitted() {
        let kept: Vec<Issue> = (0..6u32)
            .map(|i| {
                make_issue(
                    &format!("lm_s_{:04}", i),
                    IssueType::MissingText,
                    IssueSeverity::Warning,
                    Some("navigation"),
                )
            })
            .collect();
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        counts.insert("navigation".to_string(), 9);

        let regions = compute_regions(&kept, &counts);
        assert!(
            regions.is_empty(),
            "9 old nodes must not emit (below MIN_NODE_COUNT=10)"
        );
    }

    /// saturation 0.59 → not emitted even with 10 old nodes.
    #[test]
    fn test_boundary_below_saturation_not_emitted() {
        // 59 structural / 100 old nodes = 0.59
        let kept: Vec<Issue> = (0..59u32)
            .map(|i| {
                make_issue(
                    &format!("lm_s_{:04}", i),
                    IssueType::MissingText,
                    IssueSeverity::Warning,
                    Some("complementary"),
                )
            })
            .collect();
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        counts.insert("complementary".to_string(), 100);

        let regions = compute_regions(&kept, &counts);
        assert!(
            regions.is_empty(),
            "saturation 0.59 must not emit a region"
        );
    }

    // -----------------------------------------------------------------------
    // Exclusion: ChangedLinkTarget does not count toward structural_count
    // -----------------------------------------------------------------------

    /// ChangedLinkTarget issues anchored to a landmark must NOT count toward structural_count.
    #[test]
    fn test_exclusion_changed_link_target_not_structural() {
        // 10 old nodes + 8 ChangedLinkTarget → structural_count = 0 → saturation = 0 → no region
        let kept: Vec<Issue> = (0..8u32)
            .map(|i| {
                make_issue(
                    &format!("nav_clt_{:04}", i),
                    IssueType::ChangedLinkTarget,
                    IssueSeverity::Warning,
                    Some("navigation"),
                )
            })
            .collect();
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        counts.insert("navigation".to_string(), 10);

        let regions = compute_regions(&kept, &counts);
        assert!(
            regions.is_empty(),
            "ChangedLinkTarget must not count as structural; no region should be emitted"
        );
    }

    // -----------------------------------------------------------------------
    // (none) excluded
    // -----------------------------------------------------------------------

    /// Issues with landmark None never produce a region, even with a large "(none)" entry.
    #[test]
    fn test_none_landmark_never_produces_region() {
        // Many structural issues with no landmark
        let kept: Vec<Issue> = (0..50u32)
            .map(|i| {
                make_issue(
                    &format!("none_s_{:04}", i),
                    IssueType::MissingText,
                    IssueSeverity::Error,
                    None, // no landmark
                )
            })
            .collect();
        // Also add an entry for "(none)" in counts (defensive check)
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        counts.insert("(none)".to_string(), 50);

        let regions = compute_regions(&kept, &counts);
        assert!(
            regions.is_empty(),
            "issues with None landmark must never produce a region"
        );
    }

    // -----------------------------------------------------------------------
    // Denominator-0 guard
    // -----------------------------------------------------------------------

    /// A landmark with structural issues but absent from old_landmark_node_counts → skipped, no panic.
    #[test]
    fn test_denominator_zero_skipped_no_panic() {
        let kept = vec![make_issue(
            "foo_s_0000",
            IssueType::MissingText,
            IssueSeverity::Error,
            Some("form"),
        )];
        // "form" absent from counts → old_node_count = 0 → skip
        let counts: BTreeMap<String, u32> = BTreeMap::new();

        let regions = compute_regions(&kept, &counts);
        assert!(
            regions.is_empty(),
            "landmark with zero old nodes must be skipped without panic"
        );
    }

    // -----------------------------------------------------------------------
    // Broken-burst clamp
    // -----------------------------------------------------------------------

    /// 11 old nodes + 15 BrokenLink issues → saturation clamped to 1.0 via max(11,15)=15.
    #[test]
    fn test_broken_burst_clamp() {
        let kept: Vec<Issue> = (0..15u32)
            .map(|i| {
                make_issue(
                    &format!("main_bl_{:04}", i),
                    IssueType::BrokenLink,
                    IssueSeverity::Error,
                    Some("main"),
                )
            })
            .collect();
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        counts.insert("main".to_string(), 11);

        let regions = compute_regions(&kept, &counts);
        assert_eq!(regions.len(), 1, "burst clamp must still emit the region");
        let r = &regions[0];
        // saturation = 15 / max(11, 15) = 15/15 = 1.0 (not 15/11=1.36)
        assert!(
            (r.saturation - 1.0).abs() < 1e-9,
            "saturation must be clamped to 1.0 via denominator max, got {}",
            r.saturation
        );
        // old_node_count field reads raw old count (11, not the denominator)
        assert_eq!(r.old_node_count, 11, "old_node_count must be the raw count");
        assert_eq!(r.structural_count, 15);
    }

    // -----------------------------------------------------------------------
    // member_issue_ids includes both structural and style members, sorted
    // -----------------------------------------------------------------------

    #[test]
    fn test_member_issue_ids_includes_structural_and_style_sorted() {
        // Need enough old nodes and saturation to emit.
        // Use 10 structural issues so saturation=10/10=1.0
        let mut kept2: Vec<Issue> = Vec::new();
        for i in 0..10u32 {
            kept2.push(make_issue(
                &format!("s_{:04}", i),
                IssueType::MissingText,
                IssueSeverity::Error,
                Some("main"),
            ));
        }
        // Add a style member with id that sorts before "s_"
        kept2.push(make_issue(
            "a_style_0001",
            IssueType::StyleChanged,
            IssueSeverity::Warning,
            Some("main"),
        ));

        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        counts.insert("main".to_string(), 10);

        let regions = compute_regions(&kept2, &counts);
        assert_eq!(regions.len(), 1);
        let r = &regions[0];
        assert_eq!(
            r.member_issue_ids.len(),
            11,
            "11 members (10 structural + 1 style)"
        );
        // Must be sorted ascending
        let mut sorted = r.member_issue_ids.clone();
        sorted.sort();
        assert_eq!(
            r.member_issue_ids, sorted,
            "member_issue_ids must be sorted ascending"
        );
        // "a_style_0001" must be present
        assert!(
            r.member_issue_ids.contains(&"a_style_0001".to_string()),
            "style member must appear in member_issue_ids"
        );
    }

    // -----------------------------------------------------------------------
    // Determinism: shuffled input → byte-identical output
    // -----------------------------------------------------------------------

    #[test]
    fn test_determinism_shuffled_input_identical_output() {
        let mut base: Vec<Issue> = (0..20u32)
            .map(|i| {
                make_issue(
                    &format!("det_s_{:04}", i),
                    IssueType::MissingText,
                    IssueSeverity::Error,
                    Some("contentinfo"),
                )
            })
            .collect();
        base.push(make_issue(
            "det_style_0000",
            IssueType::StyleChanged,
            IssueSeverity::Warning,
            Some("contentinfo"),
        ));

        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        counts.insert("contentinfo".to_string(), 25);

        // Forward order
        let r1 = compute_regions(&base, &counts);

        // Reverse order (simulates different insertion order)
        let mut shuffled = base.clone();
        shuffled.reverse();
        let r2 = compute_regions(&shuffled, &counts);

        assert_eq!(r1.len(), r2.len(), "same number of regions");
        for (a, b) in r1.iter().zip(r2.iter()) {
            assert_eq!(a.id, b.id, "region id must be identical regardless of input order");
            assert_eq!(
                a.member_issue_ids, b.member_issue_ids,
                "member_issue_ids must be identical (sorted)"
            );
            assert_eq!(a.landmark, b.landmark);
            assert!(
                (a.saturation - b.saturation).abs() < 1e-12,
                "saturation must be identical"
            );
        }
    }

    /// Region id reproducible from landmark alone: shuffle member ids → same region id.
    #[test]
    fn test_region_id_stable_across_member_shuffle() {
        let id_a = region_id("contentinfo");
        let id_b = region_id("contentinfo");
        assert_eq!(id_a, id_b, "region_id must be purely landmark-derived");
        assert!(id_a.starts_with("region_"));
        assert_eq!(id_a.len(), 19, "region_ (7) + 12 hex = 19 chars");
        // Different landmark → different id
        let id_main = region_id("main");
        assert_ne!(id_a, id_main, "different landmarks must produce different ids");
    }

    // -----------------------------------------------------------------------
    // Severity worst-member
    // -----------------------------------------------------------------------

    /// Mixed warning+error members → region severity = error (worst).
    #[test]
    fn test_severity_worst_member() {
        let mut kept: Vec<Issue> = Vec::new();
        // 8 structural Warning members + 3 Error members in contentinfo
        for i in 0..8u32 {
            kept.push(make_issue(
                &format!("warn_s_{:04}", i),
                IssueType::MissingText,
                IssueSeverity::Warning,
                Some("contentinfo"),
            ));
        }
        for i in 0..3u32 {
            kept.push(make_issue(
                &format!("err_s_{:04}", i),
                IssueType::MissingLink,
                IssueSeverity::Error,
                Some("contentinfo"),
            ));
        }

        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        // 11 structural / 11 old nodes = 1.0 → emits
        counts.insert("contentinfo".to_string(), 11);

        let regions = compute_regions(&kept, &counts);
        assert_eq!(regions.len(), 1);
        assert_eq!(
            regions[0].severity,
            IssueSeverity::Error,
            "worst-member severity must be Error"
        );
    }
}
