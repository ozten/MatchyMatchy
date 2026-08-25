//! Deterministic issue clustering (spec §7.4, M8.md §2).
//!
//! DETERMINISM:
//! - All grouping uses BTreeMap so key order is stable.
//! - Member id vecs are sorted ascending before cluster construction.
//! - Final array sort is a total order: (count DESC, id ASC).
//! - No HashMap or float comparisons anywhere in this file.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use crate::contract::{Cluster, IssueCategory, IssueType};

/// Compute a stable 12-hex cluster id from a canonical string.
///
/// canonical = "{issue_type}\x1f{kind}\x1f{shared_key}"
/// Returns "cluster_{12hex}".
fn sha12(canonical: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    let hash = hasher.finalize();
    let hex_str = hex::encode(hash);
    format!("cluster_{}", &hex_str[..12])
}

/// Extract the style property from an issue's remediation field if the issue is
/// in the Style category and has a "property" string in its remediation map.
fn style_property(issue: &crate::contract::Issue) -> Option<String> {
    if issue.category != IssueCategory::Style {
        return None;
    }
    let rem = issue.remediation.as_ref()?;
    let prop = rem.get("property")?.as_str()?;
    if prop.is_empty() {
        None
    } else {
        Some(prop.to_string())
    }
}

/// Cluster issues by (type + style property) then (type + landmark).
///
/// Property clustering takes precedence: an issue placed in a property cluster is removed
/// from the pool before landmark clustering runs. Each issue belongs to at most one cluster.
///
/// `pre_claimed` contains issue ids already claimed by region rollups; these are excluded
/// from both Pass 1 and Pass 2 so saturated-region members never enter a cluster (AE3).
///
/// Returns clusters ordered by (member_count DESC, id ASC).
pub fn cluster_issues(
    kept: &[crate::contract::Issue],
    cluster_min: usize,
    pre_claimed: &std::collections::BTreeSet<String>,
) -> Vec<Cluster> {
    // -----------------------------------------------------------------------
    // Pass 1: Property clustering
    // -----------------------------------------------------------------------
    // Group issues with a style property by (IssueType, property).
    let mut prop_groups: BTreeMap<(IssueType, String), Vec<String>> = BTreeMap::new();
    for issue in kept {
        if pre_claimed.contains(&issue.id) {
            continue;
        }
        if let Some(p) = style_property(issue) {
            prop_groups
                .entry((issue.issue_type.clone(), p))
                .or_default()
                .push(issue.id.clone());
        }
    }

    let mut clusters: Vec<Cluster> = Vec::new();
    // claimed_ids: initialized from pre_claimed, then extended with ids placed into a property cluster.
    let mut claimed_ids: BTreeSet<String> = pre_claimed.clone();

    for ((issue_type, prop), mut ids) in prop_groups {
        if ids.len() < cluster_min {
            // Sub-min: not clustered; ids stay unclaimed.
            continue;
        }
        ids.sort();
        let n = ids.len();
        let canonical = format!("{}\x1f{}\x1f{}", issue_type.as_str(), "prop", prop);
        let cluster_id = sha12(&canonical);
        for id in &ids {
            claimed_ids.insert(id.clone());
        }
        clusters.push(Cluster {
            id: cluster_id,
            issue_ids: ids,
            shared_property: Some(prop.clone()),
            shared_landmark: None,
            summary: Some(format!(
                "{} {} issues share {}",
                n,
                issue_type.as_str(),
                prop
            )),
        });
    }

    // -----------------------------------------------------------------------
    // Pass 2: Landmark clustering (over unclaimed issues only)
    // -----------------------------------------------------------------------
    let mut lm_groups: BTreeMap<(IssueType, String), Vec<String>> = BTreeMap::new();
    for issue in kept {
        if claimed_ids.contains(&issue.id) {
            continue;
        }
        if let Some(lm) = issue.locator.anchors.landmark.as_ref() {
            if lm.is_empty() {
                continue;
            }
            lm_groups
                .entry((issue.issue_type.clone(), lm.clone()))
                .or_default()
                .push(issue.id.clone());
        }
    }

    for ((issue_type, lm), mut ids) in lm_groups {
        if ids.len() < cluster_min {
            continue;
        }
        ids.sort();
        let n = ids.len();
        let canonical = format!("{}\x1f{}\x1f{}", issue_type.as_str(), "lm", lm);
        let cluster_id = sha12(&canonical);
        clusters.push(Cluster {
            id: cluster_id,
            issue_ids: ids,
            shared_property: None,
            shared_landmark: Some(lm.clone()),
            summary: Some(format!("{} {} issues in {}", n, issue_type.as_str(), lm)),
        });
    }

    // -----------------------------------------------------------------------
    // Sort: (member_count DESC, id ASC) — total order.
    // -----------------------------------------------------------------------
    clusters.sort_by(|a, b| {
        b.issue_ids
            .len()
            .cmp(&a.issue_ids.len())
            .then_with(|| a.id.cmp(&b.id))
    });

    clusters
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{Anchors, IssueCategory, IssueSeverity, IssueType, Locator};

    /// Build a minimal Issue for testing.
    fn make_issue(
        id: &str,
        issue_type: IssueType,
        category: IssueCategory,
        landmark: Option<&str>,
        remediation_property: Option<&str>,
    ) -> crate::contract::Issue {
        let remediation = remediation_property.map(|p| serde_json::json!({ "property": p }));
        crate::contract::Issue {
            id: id.to_string(),
            issue_type,
            category,
            severity: IssueSeverity::Warning,
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
            remediation,
        }
    }

    /// Property precedence: 4 issues share (style_changed, "font-family") and also share
    /// landmark "main" → exactly ONE cluster with sharedProperty=font-family, 4 members.
    /// No landmark cluster for those same issues.
    #[test]
    fn test_property_precedence_over_landmark() {
        let issues: Vec<crate::contract::Issue> = (0..4)
            .map(|i| {
                make_issue(
                    &format!("issue_{:012}", i),
                    IssueType::StyleChanged,
                    IssueCategory::Style,
                    Some("main"),
                    Some("font-family"),
                )
            })
            .collect();

        let clusters = cluster_issues(&issues, 3, &std::collections::BTreeSet::new());

        assert_eq!(clusters.len(), 1, "expected exactly one cluster");
        let c = &clusters[0];
        assert_eq!(
            c.shared_property.as_deref(),
            Some("font-family"),
            "cluster must be by property"
        );
        assert!(c.shared_landmark.is_none(), "no landmark cluster expected");
        assert_eq!(c.issue_ids.len(), 4);
    }

    /// Sub-min not clustered by property (2 < 3), but if they share a landmark with a
    /// 3rd issue of the same type, a landmark cluster forms.
    #[test]
    fn test_sub_min_property_falls_through_to_landmark() {
        // 2 style issues sharing property "color" (below cluster_min=3)
        // + 1 more issue without a property — all 3 share landmark "main"
        let i0 = make_issue(
            "issue_000000000000",
            IssueType::StyleChanged,
            IssueCategory::Style,
            Some("main"),
            Some("color"),
        );
        let i1 = make_issue(
            "issue_000000000001",
            IssueType::StyleChanged,
            IssueCategory::Style,
            Some("main"),
            Some("color"),
        );
        // No property, but same type and same landmark
        let i2 = make_issue(
            "issue_000000000002",
            IssueType::StyleChanged,
            IssueCategory::Style,
            Some("main"),
            None, // no property
        );

        let issues = vec![i0, i1, i2];
        let clusters = cluster_issues(&issues, 3, &std::collections::BTreeSet::new());

        // No property cluster (only 2 share "color")
        let prop_clusters: Vec<_> = clusters
            .iter()
            .filter(|c| c.shared_property.is_some())
            .collect();
        assert_eq!(
            prop_clusters.len(),
            0,
            "sub-min property group must not become a property cluster"
        );

        // Landmark cluster with 3 members (all 3 share landmark "main")
        let lm_clusters: Vec<_> = clusters
            .iter()
            .filter(|c| c.shared_landmark.is_some())
            .collect();
        assert_eq!(lm_clusters.len(), 1, "expected one landmark cluster");
        assert_eq!(lm_clusters[0].issue_ids.len(), 3);
        assert_eq!(lm_clusters[0].shared_landmark.as_deref(), Some("main"));
    }

    /// Landmark cluster: 3 visual_region_changed (category Visual, no remediation.property)
    /// sharing landmark "main" → one landmark cluster with sharedLandmark=main.
    #[test]
    fn test_landmark_cluster() {
        let issues: Vec<crate::contract::Issue> = (0..3)
            .map(|i| {
                make_issue(
                    &format!("issue_{:012}", i),
                    IssueType::VisualRegionChanged,
                    IssueCategory::Visual,
                    Some("main"),
                    None,
                )
            })
            .collect();

        let clusters = cluster_issues(&issues, 3, &std::collections::BTreeSet::new());

        assert_eq!(clusters.len(), 1);
        let c = &clusters[0];
        assert!(c.shared_property.is_none());
        assert_eq!(c.shared_landmark.as_deref(), Some("main"));
        assert_eq!(c.issue_ids.len(), 3);
        // Summary format
        let summary = c.summary.as_deref().unwrap_or("");
        assert!(
            summary.contains("visual_region_changed"),
            "summary must contain type: {}",
            summary
        );
        assert!(
            summary.contains("main"),
            "summary must contain landmark: {}",
            summary
        );
    }

    /// Determinism: same input in two orders → identical clusters (ids + member order).
    #[test]
    fn test_determinism_under_shuffled_input() {
        let make = |id: &str| {
            make_issue(
                id,
                IssueType::StyleChanged,
                IssueCategory::Style,
                Some("footer"),
                Some("font-size"),
            )
        };

        let issues_forward: Vec<_> = vec![
            make("issue_aaaaaaaaaaaa"),
            make("issue_bbbbbbbbbbbb"),
            make("issue_cccccccccccc"),
        ];
        let issues_reversed: Vec<_> = vec![
            make("issue_cccccccccccc"),
            make("issue_bbbbbbbbbbbb"),
            make("issue_aaaaaaaaaaaa"),
        ];

        let c1 = cluster_issues(&issues_forward, 3, &std::collections::BTreeSet::new());
        let c2 = cluster_issues(&issues_reversed, 3, &std::collections::BTreeSet::new());

        assert_eq!(c1.len(), c2.len(), "cluster count must match");
        for (a, b) in c1.iter().zip(c2.iter()) {
            assert_eq!(a.id, b.id, "cluster ids must match");
            assert_eq!(a.issue_ids, b.issue_ids, "member ids must match");
            assert_eq!(a.shared_property, b.shared_property);
            assert_eq!(a.shared_landmark, b.shared_landmark);
        }
    }

    /// Cluster id is stable / content-addressed: same (type, kind, key) → same id across calls.
    #[test]
    fn test_cluster_id_content_addressed() {
        let issues: Vec<_> = (0..3)
            .map(|i| {
                make_issue(
                    &format!("issue_{:012x}", i),
                    IssueType::StyleChanged,
                    IssueCategory::Style,
                    None,
                    Some("font-family"),
                )
            })
            .collect();

        let c1 = cluster_issues(&issues, 3, &std::collections::BTreeSet::new());
        let c2 = cluster_issues(&issues, 3, &std::collections::BTreeSet::new());
        assert_eq!(c1.len(), 1);
        assert_eq!(c2.len(), 1);
        assert_eq!(c1[0].id, c2[0].id, "cluster id must be stable across calls");
        assert!(
            c1[0].id.starts_with("cluster_"),
            "id must start with 'cluster_'"
        );
        assert_eq!(c1[0].id.len(), 20, "cluster_ + 12 hex = 20 chars");
    }

    /// Member ids are sorted ascending in the cluster.
    #[test]
    fn test_member_ids_sorted() {
        // Insert in reverse order to verify sorting
        let issues = vec![
            make_issue(
                "issue_zzz",
                IssueType::StyleChanged,
                IssueCategory::Style,
                None,
                Some("color"),
            ),
            make_issue(
                "issue_aaa",
                IssueType::StyleChanged,
                IssueCategory::Style,
                None,
                Some("color"),
            ),
            make_issue(
                "issue_mmm",
                IssueType::StyleChanged,
                IssueCategory::Style,
                None,
                Some("color"),
            ),
        ];

        let clusters = cluster_issues(&issues, 3, &std::collections::BTreeSet::new());
        assert_eq!(clusters.len(), 1);
        let ids = &clusters[0].issue_ids;
        assert_eq!(ids, &["issue_aaa", "issue_mmm", "issue_zzz"]);
    }
}
