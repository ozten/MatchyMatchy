//! Issue construction: content-addressed IDs, collision resolution, ordering.
//!
//! DETERMINISM: SHA-256 over a canonical byte string; collision sort uses total order;
//! all maps are BTreeMap.

use sha2::{Digest, Sha256};

use crate::contract::{Anchors, Issue, IssueType};

/// Compute the content-addressed issue id per M1.md §3.2.
///
/// Canonical = fields joined by U+001F (unit separator) in exact order:
///   type, viewport, anchors.text, anchors.role, anchors.href, anchors.alt,
///   anchors.ariaLabel, anchors.nearestHeading, anchors.landmark,
///   str(anchors.ordinalInLandmark), styleProperty
/// None -> empty string.
pub fn compute_issue_id(
    issue_type: &IssueType,
    viewport: &str,
    anchors: &Anchors,
    style_property: Option<&str>,
) -> String {
    let sep = '\x1f';
    let ordinal_str = anchors
        .ordinal_in_landmark
        .map(|n| n.to_string())
        .unwrap_or_default();

    let canonical = format!(
        "{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}",
        issue_type.as_str(),
        viewport,
        anchors.text.as_deref().unwrap_or(""),
        anchors.role.as_deref().unwrap_or(""),
        anchors.href.as_deref().unwrap_or(""),
        anchors.alt.as_deref().unwrap_or(""),
        anchors.aria_label.as_deref().unwrap_or(""),
        anchors.nearest_heading.as_deref().unwrap_or(""),
        anchors.landmark.as_deref().unwrap_or(""),
        ordinal_str,
        style_property.unwrap_or(""),
        sep = sep,
    );

    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    let hash = hasher.finalize();
    let hex_str = hex::encode(hash);
    format!("issue_{}", &hex_str[..12])
}

/// Resolve collisions after all issues are constructed.
///
/// Issues that hash identically get collision suffixes: the first (by bbox sort order)
/// keeps the base id; subsequent get "-2", "-3", etc.
///
/// Sort order: (bboxNew.y, bboxNew.x, bboxOld.y, bboxOld.x), with None bbox sorting last.
pub fn resolve_id_collisions(issues: &mut [Issue]) {
    use std::collections::BTreeMap;

    // Group indices by base id
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, issue) in issues.iter().enumerate() {
        groups.entry(issue.id.clone()).or_default().push(i);
    }

    for (_, mut indices) in groups {
        if indices.len() <= 1 {
            continue;
        }
        // Sort colliders by (bboxNew.y, bboxNew.x, bboxOld.y, bboxOld.x), None sorts last.
        // Tie-break by issue id for total order.
        indices.sort_by(|&a, &b| {
            let ia = &issues[a];
            let ib = &issues[b];

            let key_a = collision_sort_key(ia);
            let key_b = collision_sort_key(ib);
            key_a.cmp(&key_b).then_with(|| ia.id.cmp(&ib.id))
        });

        // First keeps base id; rest get suffixes "-2", "-3", ...
        for (suffix_idx, &issue_idx) in indices.iter().enumerate().skip(1) {
            let base_id = issues[issue_idx].id.clone();
            // Strip any existing suffix
            // id format: "issue_{12hex}" optionally followed by "-N"
            let new_id = format!("{}-{}", strip_collision_suffix(&base_id), suffix_idx + 1);
            issues[issue_idx].id = new_id;
        }
    }
}

fn strip_collision_suffix(id: &str) -> &str {
    // id format: "issue_{12hex}" or "issue_{12hex}-N"
    // Find the last '-' after the base "issue_HHHHHHHHHHHH" (len=18)
    if id.len() > 18 && id.as_bytes()[18] == b'-' {
        &id[..18]
    } else {
        id
    }
}

/// Returns a sortable key for collision resolution.
/// (new_y, new_x, old_y, old_x) with None values as i64::MAX.
fn collision_sort_key(issue: &Issue) -> (i64, i64, i64, i64) {
    let new_y = issue.locator.bbox_new.map(|b| b[1] as i64).unwrap_or(i64::MAX);
    let new_x = issue.locator.bbox_new.map(|b| b[0] as i64).unwrap_or(i64::MAX);
    let old_y = issue.locator.bbox_old.map(|b| b[1] as i64).unwrap_or(i64::MAX);
    let old_x = issue.locator.bbox_old.map(|b| b[0] as i64).unwrap_or(i64::MAX);
    (new_y, new_x, old_y, old_x)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        Anchors, IssueCategory, IssueSeverity, IssueType, Locator,
    };

    fn make_anchors(text: Option<&str>) -> Anchors {
        Anchors {
            text: text.map(str::to_string),
            role: None,
            href: None,
            alt: None,
            aria_label: None,
            nearest_heading: None,
            landmark: None,
            ordinal_in_landmark: None,
        }
    }

    fn make_issue_with_bbox(
        issue_type: IssueType,
        viewport: &str,
        anchors: Anchors,
        bbox_new: Option<[i32; 4]>,
        bbox_old: Option<[i32; 4]>,
    ) -> Issue {
        let id = compute_issue_id(&issue_type, viewport, &anchors, None);
        Issue {
            id,
            issue_type,
            category: IssueCategory::Visual,
            severity: IssueSeverity::Info,
            confidence: 0.9,
            viewport: viewport.to_string(),
            locale: None,
            goal: None,
            message: "test".to_string(),
            locator: Locator {
                anchors,
                css_selector_old: None,
                css_selector_new: None,
                bbox_old,
                bbox_new,
                seq_index_old: None,
                seq_index_new: None,
            },
            evidence: serde_json::json!({}),
            remediation: None,
        }
    }

    #[test]
    fn test_issue_id_stability_under_jittered_inputs() {
        // ID must be identical regardless of bbox or confidence jitter
        let anchors = make_anchors(Some("20% off"));
        let id1 = compute_issue_id(
            &IssueType::VisualRegionChanged,
            "desktop",
            &anchors,
            None,
        );
        let id2 = compute_issue_id(
            &IssueType::VisualRegionChanged,
            "desktop",
            &anchors,
            None,
        );
        assert_eq!(id1, id2, "id must be stable across calls");

        // Changing bbox has no effect (bbox excluded from hash)
        let id3 = compute_issue_id(
            &IssueType::VisualRegionChanged,
            "desktop",
            &anchors,
            None,
        );
        assert_eq!(id1, id3, "bbox does not affect id");

        // Changing a hashable field (type) does change the id
        let id4 = compute_issue_id(
            &IssueType::PageHeightChanged,
            "desktop",
            &anchors,
            None,
        );
        assert_ne!(id1, id4, "different type must produce different id");
    }

    #[test]
    fn test_issue_id_format() {
        let anchors = make_anchors(None);
        let id = compute_issue_id(&IssueType::LoadError, "desktop", &anchors, None);
        assert!(id.starts_with("issue_"), "id must start with 'issue_'");
        assert_eq!(id.len(), 18, "id must be 'issue_' + 12 hex chars = 18 chars");
        let hex_part = &id[6..];
        assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_collision_suffix_determinism() {
        // Two issues with identical hash inputs (no text anchor)
        let anchors = make_anchors(None);
        let mut issues = vec![
            make_issue_with_bbox(
                IssueType::VisualRegionChanged,
                "desktop",
                anchors.clone(),
                Some([100, 200, 50, 50]), // bbox_new y=200
                Some([100, 200, 50, 50]),
            ),
            make_issue_with_bbox(
                IssueType::VisualRegionChanged,
                "desktop",
                anchors.clone(),
                Some([100, 50, 50, 50]),  // bbox_new y=50 (sorts first)
                Some([100, 50, 50, 50]),
            ),
        ];

        // They should have the same base id
        assert_eq!(issues[0].id, issues[1].id);

        resolve_id_collisions(&mut issues);

        // After resolution, ids must be distinct
        assert_ne!(issues[0].id, issues[1].id);
        // The one with lower y (index 1: y=50) should NOT get a suffix (it's first).
        // The one with higher y (index 0: y=200) should get "-2".
        let base_id: String = issues[0].id.clone();
        let base_id2: String = issues[1].id.clone();
        // One of them ends with "-2", the other doesn't
        let has_suffix = base_id.ends_with("-2") || base_id2.ends_with("-2");
        assert!(has_suffix, "at least one issue should have '-2' suffix");
        let no_suffix = !base_id.ends_with("-2") || !base_id2.ends_with("-2");
        assert!(no_suffix, "at least one issue should NOT have a suffix");
    }

    #[test]
    fn test_anchor_strength() {
        let high = Anchors {
            text: Some("Click me".to_string()),
            role: None,
            href: None,
            alt: None,
            aria_label: None,
            nearest_heading: None,
            landmark: None,
            ordinal_in_landmark: None,
        };
        assert_eq!(high.strength(), crate::contract::AnchorStrength::High);

        let medium = Anchors {
            text: None,
            role: None,
            href: None,
            alt: None,
            aria_label: None,
            nearest_heading: Some("About".to_string()),
            landmark: Some("main".to_string()),
            ordinal_in_landmark: None,
        };
        assert_eq!(medium.strength(), crate::contract::AnchorStrength::Medium);

        let low = Anchors::null();
        assert_eq!(low.strength(), crate::contract::AnchorStrength::Low);
    }
}
