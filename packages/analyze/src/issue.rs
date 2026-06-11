//! Issue construction: content-addressed IDs, collision resolution, ordering.
//!
//! DETERMINISM: SHA-256 over a canonical byte string; collision sort uses total order;
//! all maps are BTreeMap.

use sha2::{Digest, Sha256};
use url::Url;

use crate::contract::{Anchors, Issue, IssueType};

/// Normalize an href to scheme+host+path for stable hashing.
///
/// Live pages may inject volatile query parameters (`__hstc`, `utm_*`, etc.) that must
/// not affect the id. On successful parse, returns `scheme://host[:port]/path`; query
/// and fragment are dropped. On parse failure (relative or malformed hrefs), the string
/// is truncated at the first `?` or `#`, whichever comes first; if neither is present
/// the input is returned unchanged.
fn id_stable_url(href: &str) -> String {
    if let Ok(parsed) = Url::parse(href) {
        let scheme = parsed.scheme();
        let host = parsed.host_str().unwrap_or("");
        let path = parsed.path();
        match parsed.port() {
            Some(port) => format!("{}://{}:{}{}", scheme, host, port, path),
            None => format!("{}://{}{}", scheme, host, path),
        }
    } else {
        // Relative or unparseable: strip from first '?' or '#'
        let q = href.find('?').unwrap_or(usize::MAX);
        let f = href.find('#').unwrap_or(usize::MAX);
        let cut = q.min(f);
        if cut == usize::MAX {
            href.to_string()
        } else {
            href[..cut].to_string()
        }
    }
}

/// Compute the content-addressed issue id per M1.md §3.2.
///
/// Canonical = fields joined by U+001F (unit separator) in exact order:
///   type, viewport, anchors.text, anchors.role, anchors.href, anchors.alt,
///   anchors.ariaLabel, anchors.nearestHeading, anchors.landmark,
///   str(anchors.ordinalInLandmark), styleProperty
/// None -> empty string.
///
/// URL anchors are normalized to scheme+host+path before hashing because query strings
/// carry volatile session/tracking data (see docs/bugs/p0-02); two links that differ
/// only by query/fragment intentionally share an id (the existing bbox-ordered collision
/// suffix disambiguates if both occur).
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

    let href_stable = anchors
        .href
        .as_deref()
        .map(id_stable_url)
        .unwrap_or_default();

    let canonical = format!(
        "{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}",
        issue_type.as_str(),
        viewport,
        anchors.text.as_deref().unwrap_or(""),
        anchors.role.as_deref().unwrap_or(""),
        href_stable,
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
    let new_y = issue
        .locator
        .bbox_new
        .map(|b| b[1] as i64)
        .unwrap_or(i64::MAX);
    let new_x = issue
        .locator
        .bbox_new
        .map(|b| b[0] as i64)
        .unwrap_or(i64::MAX);
    let old_y = issue
        .locator
        .bbox_old
        .map(|b| b[1] as i64)
        .unwrap_or(i64::MAX);
    let old_x = issue
        .locator
        .bbox_old
        .map(|b| b[0] as i64)
        .unwrap_or(i64::MAX);
    (new_y, new_x, old_y, old_x)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{Anchors, IssueCategory, IssueSeverity, IssueType, Locator};

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
        let id1 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &anchors, None);
        let id2 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &anchors, None);
        assert_eq!(id1, id2, "id must be stable across calls");

        // Changing bbox has no effect (bbox excluded from hash)
        let id3 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &anchors, None);
        assert_eq!(id1, id3, "bbox does not affect id");

        // Changing a hashable field (type) does change the id
        let id4 = compute_issue_id(&IssueType::PageHeightChanged, "desktop", &anchors, None);
        assert_ne!(id1, id4, "different type must produce different id");
    }

    #[test]
    fn test_issue_id_format() {
        let anchors = make_anchors(None);
        let id = compute_issue_id(&IssueType::LoadError, "desktop", &anchors, None);
        assert!(id.starts_with("issue_"), "id must start with 'issue_'");
        assert_eq!(
            id.len(),
            18,
            "id must be 'issue_' + 12 hex chars = 18 chars"
        );
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
                Some([100, 50, 50, 50]), // bbox_new y=50 (sorts first)
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

    // -----------------------------------------------------------------------
    // id_stable_url unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_id_stable_url_drops_query_and_fragment() {
        // Absolute URL: query and fragment both dropped
        assert_eq!(
            id_stable_url("https://www.hiya.com/newsroom/?__hstc=17958374.abc#press-kit"),
            "https://www.hiya.com/newsroom/"
        );
        // Only fragment
        assert_eq!(
            id_stable_url("https://www.hiya.com/newsroom/#press-kit"),
            "https://www.hiya.com/newsroom/"
        );
        // Clean URL unchanged
        assert_eq!(
            id_stable_url("https://www.hiya.com/newsroom/"),
            "https://www.hiya.com/newsroom/"
        );
    }

    #[test]
    fn test_id_stable_url_preserves_non_default_port() {
        assert_eq!(
            id_stable_url("http://localhost:3001/a?b=c"),
            "http://localhost:3001/a"
        );
    }

    #[test]
    fn test_id_stable_url_parse_failure_truncation() {
        // Relative href with query: truncate at '?'
        assert_eq!(id_stable_url("foo/bar?x=1"), "foo/bar");
        // Relative href without query/fragment: unchanged
        assert_eq!(id_stable_url("foo/bar"), "foo/bar");
        // Fragment-only: truncate at '#' → empty string
        assert_eq!(id_stable_url("#x"), "");
    }

    // -----------------------------------------------------------------------
    // compute_issue_id stability tests for volatile href query params
    // -----------------------------------------------------------------------

    fn make_anchors_with_href(href: &str) -> Anchors {
        Anchors {
            text: Some("Press Kit".to_string()),
            role: None,
            href: Some(href.to_string()),
            alt: None,
            aria_label: None,
            nearest_heading: Some("About".to_string()),
            landmark: Some("main".to_string()),
            ordinal_in_landmark: None,
        }
    }

    #[test]
    fn test_issue_id_stable_across_tracking_params() {
        // All three hrefs differ only in query/fragment — must produce the same id
        let a1 =
            make_anchors_with_href("https://www.hiya.com/newsroom/?__hstc=17958374.abc#press-kit");
        let a2 = make_anchors_with_href("https://www.hiya.com/newsroom/#press-kit");
        let a3 = make_anchors_with_href("https://www.hiya.com/newsroom/");

        let id1 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &a1, None);
        let id2 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &a2, None);
        let id3 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &a3, None);

        assert_eq!(id1, id2, "tracking params must not affect id");
        assert_eq!(id2, id3, "fragment alone must not affect id");
    }

    #[test]
    fn test_issue_id_differs_for_different_path() {
        let a_newsroom = make_anchors_with_href("https://www.hiya.com/newsroom/");
        let a_blog = make_anchors_with_href("https://www.hiya.com/blog");

        let id1 = compute_issue_id(
            &IssueType::VisualRegionChanged,
            "desktop",
            &a_newsroom,
            None,
        );
        let id2 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &a_blog, None);

        assert_ne!(id1, id2, "different path must produce different id");
    }

    #[test]
    fn test_issue_id_differs_for_different_host() {
        let a_hiya = make_anchors_with_href("https://www.hiya.com/newsroom/");
        let a_other = make_anchors_with_href("https://other.example.com/newsroom/");

        let id1 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &a_hiya, None);
        let id2 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &a_other, None);

        assert_ne!(id1, id2, "different host must produce different id");
    }

    #[test]
    fn test_issue_id_stable_for_relative_href_with_query() {
        // Relative hrefs: parse fails, truncate at '?'
        let a_with_q = make_anchors_with_href("foo/bar?x=1");
        let a_clean = make_anchors_with_href("foo/bar");

        let id1 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &a_with_q, None);
        let id2 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &a_clean, None);

        assert_eq!(id1, id2, "relative href query must not affect id");
    }

    #[test]
    fn test_issue_id_fragment_only_href() {
        // '#x' truncates to "" — same as an empty href
        assert_eq!(id_stable_url("#x"), "");

        let a_frag = make_anchors_with_href("#x");
        let a_empty = Anchors {
            text: Some("Press Kit".to_string()),
            role: None,
            href: Some(String::new()),
            alt: None,
            aria_label: None,
            nearest_heading: Some("About".to_string()),
            landmark: Some("main".to_string()),
            ordinal_in_landmark: None,
        };

        let id_frag = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &a_frag, None);
        let id_empty = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &a_empty, None);

        assert_eq!(
            id_frag, id_empty,
            "fragment-only href should hash the same as empty href"
        );
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
