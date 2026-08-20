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

/// Compute the content-addressed issue id per spec §7.1 (amended, U2).
///
/// Canonical = fields joined by U+001F (unit separator) in exact order:
///   type, viewport, anchors.text, anchors.role, anchors.href (normalized), anchors.alt,
///   anchors.ariaLabel, anchors.landmark, [nearestHeading — see below], styleProperty
/// None -> empty string.
///
/// URL anchors are normalized to scheme+host+path before hashing because query strings
/// carry volatile session/tracking data (see docs/bugs/p0-02); two links that differ
/// only by query/fragment intentionally share an id (the document-order collision suffix
/// disambiguates if both occur — see `resolve_id_collisions`).
///
/// **Unconditionally excluded:** `ordinalInLandmark`. It shifts whenever a sibling is
/// inserted or removed near an unrelated defect, so it never contributes to the hash —
/// this was one of the two documented co-causes of the p01 2/129 id-survival failure
/// (docs/bugs/p0-02; only a last-resort collision disambiguator, see
/// `resolve_id_collisions`, ever uses document position, and even there it is
/// `seqIndex`, not `ordinalInLandmark`).
///
/// **Conditionally excluded:** `nearestHeading`. On live pages it is computed from
/// "first visible heading", which itself shifts with load/visibility state between
/// re-captures — the other documented co-cause of the same p01 failure. It is identity-
/// grade (included in the hash) **only** when `text`, `href`, `alt`, and `ariaLabel` are
/// ALL absent/empty for this issue: a bare decorative element has no other identity
/// signal, so without `nearestHeading` its hash would be nearly empty and every such
/// element would collide. Whenever any of those four strong/medium anchors is present,
/// `nearestHeading` contributes NOTHING to the hash, even if it changes.
pub fn compute_issue_id(
    issue_type: &IssueType,
    viewport: &str,
    anchors: &Anchors,
    style_property: Option<&str>,
) -> String {
    let sep = '\x1f';

    let href_stable = anchors
        .href
        .as_deref()
        .map(id_stable_url)
        .unwrap_or_default();

    let has_strong_or_medium_anchor = anchors
        .text
        .as_deref()
        .map(|s| !s.is_empty())
        .unwrap_or(false)
        || !href_stable.is_empty()
        || anchors
            .alt
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
        || anchors
            .aria_label
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false);
    let nearest_heading_slot = if has_strong_or_medium_anchor {
        ""
    } else {
        anchors.nearest_heading.as_deref().unwrap_or("")
    };

    // styleProperty slot: the CSS property name for style-category issues. U10 will also
    // route which-pseudo through this same slot ("::before" / "::before.background-color")
    // — the slot is shared with which-pseudo, nothing more.
    let canonical = format!(
        "{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}",
        issue_type.as_str(),
        viewport,
        anchors.text.as_deref().unwrap_or(""),
        anchors.role.as_deref().unwrap_or(""),
        href_stable,
        anchors.alt.as_deref().unwrap_or(""),
        anchors.aria_label.as_deref().unwrap_or(""),
        anchors.landmark.as_deref().unwrap_or(""),
        nearest_heading_slot,
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
/// Issues that hash identically get collision suffixes: the first (by document order —
/// see `collision_sort_key`) keeps the base id; subsequent get "-2", "-3", etc.
///
/// Sort order: (seqIndexOld ascending, None sorts last), then (seqIndexNew ascending,
/// None sorts last), then the existing final tie-break on the pre-suffix id — which,
/// because every issue in a colliding group shares that same pre-suffix id, is a no-op
/// that falls through to Rust's stable `sort_by` preserving insertion order. Never bbox
/// pixels: bbox jitters between re-captures of a live page (viewport reflow, ad-block
/// noise) while seqIndex is stable for a defect that still exists in the same document
/// position — this was the instability source the collision suffix used to inherit.
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

/// Returns a sortable key for collision resolution: document order within the colliding
/// set, never bbox pixels. `(seqIndexOld, seqIndexNew)`, each ascending with `None`
/// sorting last (encoded as `(1, 0)` vs. `(0, n)` so `Some` always precedes `None`).
fn collision_sort_key(issue: &Issue) -> ((u8, u32), (u8, u32)) {
    (
        seq_sort_key(issue.locator.seq_index_old),
        seq_sort_key(issue.locator.seq_index_new),
    )
}

fn seq_sort_key(v: Option<u32>) -> (u8, u32) {
    match v {
        Some(n) => (0, n),
        None => (1, 0),
    }
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
        make_issue_with_seq(
            issue_type, viewport, anchors, None, None, bbox_old, bbox_new,
        )
    }

    /// Full-control helper: same shape as `make_issue_with_bbox` but also sets
    /// `seq_index_old`/`seq_index_new` on the locator, needed to exercise the
    /// document-order collision suffixing (U2).
    fn make_issue_with_seq(
        issue_type: IssueType,
        viewport: &str,
        anchors: Anchors,
        seq_index_old: Option<u32>,
        seq_index_new: Option<u32>,
        bbox_old: Option<[i32; 4]>,
        bbox_new: Option<[i32; 4]>,
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
                seq_index_old,
                seq_index_new,
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
    fn test_collision_suffix_ignores_bbox_uses_document_order() {
        // Two issues with identical hash inputs (no text anchor) and no seq_index on
        // either side: collision resolution must NOT depend on bbox pixels — insertion
        // (document) order wins, bbox is irrelevant to suffix assignment.
        let anchors = make_anchors(None);
        let mut issues = vec![
            make_issue_with_bbox(
                IssueType::VisualRegionChanged,
                "desktop",
                anchors.clone(),
                Some([100, 200, 50, 50]), // bbox_new y=200 — would have sorted 2nd, pre-fix
                Some([100, 200, 50, 50]),
            ),
            make_issue_with_bbox(
                IssueType::VisualRegionChanged,
                "desktop",
                anchors.clone(),
                Some([100, 50, 50, 50]), // bbox_new y=50 — would have sorted 1st, pre-fix
                Some([100, 50, 50, 50]),
            ),
        ];

        // They should have the same base id.
        let base_id = issues[0].id.clone();
        assert_eq!(issues[0].id, issues[1].id);

        resolve_id_collisions(&mut issues);

        // After resolution, ids must be distinct.
        assert_ne!(issues[0].id, issues[1].id);
        // Neither side carries a seq_index, so insertion order (index 0 first) wins the
        // document-order tie-break, regardless of which one has the lower bbox y.
        assert_eq!(
            issues[0].id, base_id,
            "insertion-first issue keeps the base id even though its bbox.y is larger"
        );
        assert_eq!(issues[1].id, format!("{}-2", base_id));
    }

    // -----------------------------------------------------------------------
    // U2: ordinalInLandmark / nearestHeading identity-boundary tests
    // -----------------------------------------------------------------------

    /// (a) Ordinal-shift survival: sibling removal/insertion shifts `ordinalInLandmark`
    /// for every surviving issue (the p01 disease). The id must not move.
    #[test]
    fn test_ordinal_in_landmark_never_affects_id() {
        let mut a1 = make_anchors(Some("Get started"));
        a1.href = Some("/signup".to_string());
        a1.landmark = Some("main".to_string());
        a1.ordinal_in_landmark = Some(3);

        let mut a2 = a1.clone();
        a2.ordinal_in_landmark = Some(97); // simulates every prior sibling removed

        let id1 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &a1, None);
        let id2 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &a2, None);
        assert_eq!(id1, id2, "ordinalInLandmark shift must never affect the id");

        // Also true when ordinal disappears entirely.
        let mut a3 = a1.clone();
        a3.ordinal_in_landmark = None;
        let id3 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &a3, None);
        assert_eq!(
            id1, id3,
            "ordinalInLandmark presence vs. absence must not affect the id"
        );
    }

    /// (b) nearestHeading-shift survival: when a strong anchor (text, here) is present,
    /// rewriting nearestHeading (simulating a re-capture visibility shift) must not move
    /// the id.
    #[test]
    fn test_nearest_heading_excluded_when_strong_anchor_present() {
        let mut a1 = make_anchors(Some("Get started"));
        a1.landmark = Some("main".to_string());
        a1.nearest_heading = Some("Build faster".to_string());

        let mut a2 = a1.clone();
        a2.nearest_heading = Some("Totally different heading".to_string());

        let id1 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &a1, None);
        let id2 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &a2, None);
        assert_eq!(
            id1, id2,
            "nearestHeading must not affect id when text/href/alt/ariaLabel is present"
        );
    }

    /// (b, converse) A bare decorative element carrying none of text/href/alt/ariaLabel
    /// has no other identity signal, so nearestHeading DOES change its id — it is the
    /// documented last-resort disambiguator, not fully excluded.
    #[test]
    fn test_nearest_heading_is_last_resort_disambiguator_when_bare() {
        let mut a1 = make_anchors(None); // text/role/href/alt/aria_label all None
        a1.landmark = Some("main".to_string());
        a1.nearest_heading = Some("Build faster".to_string());

        let mut a2 = a1.clone();
        a2.nearest_heading = Some("Totally different heading".to_string());

        let id1 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &a1, None);
        let id2 = compute_issue_id(&IssueType::VisualRegionChanged, "desktop", &a2, None);
        assert_ne!(
            id1, id2,
            "bare decorative anchors have no other identity signal; nearestHeading must matter"
        );
    }

    /// (c) Identical-twin collisions: three same-type issues with identical identity
    /// anchors but distinct seq_index_old get three distinct suffixed ids, stable when
    /// bboxes are jittered arbitrarily (never bbox-sorted).
    #[test]
    fn test_identical_twins_suffix_by_document_order_stable_under_bbox_jitter() {
        let anchors = make_anchors(Some("Read more"));

        let build = |bbox: [i32; 4]| {
            vec![
                make_issue_with_seq(
                    IssueType::VisualRegionChanged,
                    "desktop",
                    anchors.clone(),
                    Some(10),
                    Some(10),
                    Some(bbox),
                    Some(bbox),
                ),
                make_issue_with_seq(
                    IssueType::VisualRegionChanged,
                    "desktop",
                    anchors.clone(),
                    Some(20),
                    Some(20),
                    Some([bbox[0], bbox[1] - 500, bbox[2], bbox[3]]), // hostile: lower y
                    Some([bbox[0], bbox[1] - 500, bbox[2], bbox[3]]),
                ),
                make_issue_with_seq(
                    IssueType::VisualRegionChanged,
                    "desktop",
                    anchors.clone(),
                    Some(30),
                    Some(30),
                    Some([bbox[0], bbox[1] + 9000, bbox[2], bbox[3]]), // hostile: huge y
                    Some([bbox[0], bbox[1] + 9000, bbox[2], bbox[3]]),
                ),
            ]
        };

        let mut run1 = build([1, 1000, 10, 10]);
        let base_id = run1[0].id.clone();
        assert_eq!(run1[1].id, base_id);
        assert_eq!(run1[2].id, base_id);

        resolve_id_collisions(&mut run1);
        assert_eq!(run1[0].id, base_id, "seq_index_old=10 keeps the base id");
        assert_eq!(run1[1].id, format!("{}-2", base_id));
        assert_eq!(run1[2].id, format!("{}-3", base_id));

        // Re-derive from scratch with completely different (arbitrarily jittered) bboxes:
        // the suffix assignment must be identical, because it is driven by seq_index_old,
        // never by bbox pixels.
        let mut run2 = build([500, 42, 10, 10]);
        resolve_id_collisions(&mut run2);
        assert_eq!(run2[0].id, base_id);
        assert_eq!(run2[1].id, format!("{}-2", base_id));
        assert_eq!(run2[2].id, format!("{}-3", base_id));
    }

    /// (c) Removing the middle twin keeps the FIRST twin's id unchanged (document-order
    /// suffixing means only the removed/added twin's slot shifts).
    #[test]
    fn test_removing_middle_twin_keeps_first_twins_id_unchanged() {
        let anchors = make_anchors(Some("Read more"));

        let mut all_three = vec![
            make_issue_with_seq(
                IssueType::VisualRegionChanged,
                "desktop",
                anchors.clone(),
                Some(10),
                Some(10),
                None,
                None,
            ),
            make_issue_with_seq(
                IssueType::VisualRegionChanged,
                "desktop",
                anchors.clone(),
                Some(20),
                Some(20),
                None,
                None,
            ),
            make_issue_with_seq(
                IssueType::VisualRegionChanged,
                "desktop",
                anchors.clone(),
                Some(30),
                Some(30),
                None,
                None,
            ),
        ];
        resolve_id_collisions(&mut all_three);
        let first_id_before = all_three[0].id.clone();
        assert!(
            !first_id_before.contains('-') || first_id_before.rsplit('-').next() != Some("2"),
            "sanity: first twin should hold the unsuffixed base id"
        );

        // Remove the middle twin (seq_index_old = 20) and re-resolve from scratch.
        let mut without_middle = vec![
            make_issue_with_seq(
                IssueType::VisualRegionChanged,
                "desktop",
                anchors.clone(),
                Some(10),
                Some(10),
                None,
                None,
            ),
            make_issue_with_seq(
                IssueType::VisualRegionChanged,
                "desktop",
                anchors.clone(),
                Some(30),
                Some(30),
                None,
                None,
            ),
        ];
        resolve_id_collisions(&mut without_middle);

        assert_eq!(
            without_middle[0].id, first_id_before,
            "the first (lowest seq_index_old) twin's id must be unaffected by removing a sibling twin"
        );
    }

    /// (e) Pseudo-slot distinctness: same anchors, distinct styleProperty slot values
    /// (which-pseudo prefixes, per U10) must produce three distinct ids.
    #[test]
    fn test_pseudo_style_property_slot_distinguishes_which_pseudo() {
        let anchors = make_anchors(Some("corner tick"));
        let id_before = compute_issue_id(
            &IssueType::StyleChanged,
            "desktop",
            &anchors,
            Some("::before"),
        );
        let id_after = compute_issue_id(
            &IssueType::StyleChanged,
            "desktop",
            &anchors,
            Some("::after"),
        );
        let id_before_bg = compute_issue_id(
            &IssueType::StyleChanged,
            "desktop",
            &anchors,
            Some("::before.background-image"),
        );

        assert_ne!(id_before, id_after, "::before vs ::after must differ");
        assert_ne!(
            id_before, id_before_bg,
            "::before vs ::before.background-image must differ"
        );
        assert_ne!(
            id_after, id_before_bg,
            "::after vs ::before.background-image must differ"
        );
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
