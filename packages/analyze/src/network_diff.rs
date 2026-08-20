//! Network and console diff (M7.md §2).
//!
//! Emits `network_error` issues for new-only failing requests (failed==true or status>=400),
//! and `console_error` issues for new-only error-level console messages.
//!
//! DETERMINISM: BTreeSet for membership; collect-then-sort by stable key; no HashMap.

use std::collections::{BTreeMap, BTreeSet};

use url::Url;

use crate::config::base_confidence;
use crate::contract::{
    Anchors, CaptureBundle, Issue, IssueCategory, IssueType, Locator, SemanticNode,
};
use crate::issue::compute_issue_id;
use crate::scoring::{compute_confidence, ParityProfile};

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Normalize a request URL relative to `own_final_url`'s page directory.
///
/// Same-site requests are keyed relative to the directory of `own_final_url`'s path
/// (everything up to and including the last `/`).  This lets old and new pages that
/// serve the same assets from different path-prefix mounts (e.g. `/` vs
/// `/products/connect/branded-call/`) produce the same key for a shared asset, so a
/// 404 on both sides is correctly correlated and suppressed.
///
/// Key computation (same-site):
///   base_dir = directory of own_final_url.path  (last-slash rule: "/x/y/" → "/x/y/";
///                                                 "/x/y" → "/x/";  "/" or "" → "/")
///   path = url.path
///   if path starts_with base_dir  → rel = path[base_dir.len()..]
///   else if path starts_with "/"  → rel = path[1..]  (same-origin, outside page dir)
///   else                          → rel = path
///   return rel + (query ? "?"+query : "")
///
/// Third-party URLs (different origin) are returned unchanged (absolute).
/// Falls back to returning `url` as-is on any parse error.
fn request_key(url: &str, own_final_url: &str) -> String {
    // Parse both URLs; fall back to raw url on any error.
    let own = match Url::parse(own_final_url) {
        Ok(u) => u,
        Err(_) => return url.to_string(),
    };
    let req = match Url::parse(url) {
        Ok(u) => u,
        Err(_) => return url.to_string(),
    };

    // Compare origins (scheme + host + port).
    let same_site = own.scheme() == req.scheme()
        && own.host() == req.host()
        && own.port_or_known_default() == req.port_or_known_default();

    if !same_site {
        return url.to_string();
    }

    // Compute base_dir: path of own_final_url up to and including the last '/'.
    let own_path = own.path();
    let base_dir: &str = match own_path.rfind('/') {
        Some(idx) => &own_path[..=idx], // includes the trailing '/'
        None => "/",
    };
    // Guard: base_dir must be non-empty (should always hold after rfind above).
    let base_dir = if base_dir.is_empty() { "/" } else { base_dir };

    let req_path = req.path();
    let rel = if let Some(stripped) = req_path.strip_prefix(base_dir) {
        stripped
    } else if let Some(stripped) = req_path.strip_prefix('/') {
        stripped
    } else {
        req_path
    };

    match req.query() {
        Some(q) => format!("{}?{}", rel, q),
        None => rel.to_string(),
    }
}

/// Extract filename from a URL (for remediation grep target).
fn filename_from_url(url: &str) -> String {
    match Url::parse(url) {
        Ok(u) => u
            .path_segments()
            .and_then(|mut segs| segs.next_back())
            .unwrap_or("")
            .to_string(),
        Err(_) => url.rsplit('/').next().unwrap_or(url).to_string(),
    }
}

/// Build a Locator anchored to a single new-page node (network error case).
fn single_node_locator(anchors: Anchors, node: &SemanticNode) -> Locator {
    Locator {
        anchors,
        css_selector_old: None,
        css_selector_new: node.css_selector.clone(),
        bbox_old: None,
        bbox_new: Some(node.bbox),
        seq_index_old: None,
        seq_index_new: Some(node.seq_index),
    }
}

/// Build a null locator (page-level, no node).
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

/// Returns true if a NetworkRequest is considered failing.
fn is_failing(status: Option<u32>, failed: bool) -> bool {
    failed || status.unwrap_or(0) >= 400
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Emit `network_error` and `console_error` issues for new-only failures/messages.
///
/// Emission order: all `network_error` (sorted by request key) then all `console_error`
/// (sorted by message text). Both sub-lists are deterministic regardless of input order.
pub fn network_console_issues(
    old_bundle: &CaptureBundle,
    new_bundle: &CaptureBundle,
    viewport: &str,
    profile: &ParityProfile,
    env_mismatch: bool,
) -> Vec<Issue> {
    let old_det = &old_bundle.determinism;
    let new_det = &new_bundle.determinism;
    let new_final_url = &new_bundle.page.final_url;
    let old_final_url = &old_bundle.page.final_url;

    let mut issues: Vec<Issue> = Vec::new();

    // ------------------------------------------------------------------
    // network_error: new-only failing requests
    // ------------------------------------------------------------------

    // Build old failing key set
    let old_failing: BTreeSet<String> = old_bundle
        .page
        .network
        .requests
        .iter()
        .filter(|r| is_failing(r.status, r.failed))
        .map(|r| request_key(&r.url, old_final_url))
        .collect();

    // Build a lookup from key → first old request (for evidence)
    // We want the matching OLD request (even if not failing) for the old evidence side.
    // Build: key → first old request url+status (by order in the array; stable because we
    // collect into BTreeMap and first-seen wins by iteration into a BTreeMap only recording
    // first occurrence — we iterate once in order).
    let old_request_by_key: BTreeMap<String, (String, Option<u32>)> = {
        let mut m: BTreeMap<String, (String, Option<u32>)> = BTreeMap::new();
        for r in &old_bundle.page.network.requests {
            let k = request_key(&r.url, old_final_url);
            m.entry(k).or_insert_with(|| (r.url.clone(), r.status));
        }
        m
    };

    // Collect new-only failing requests, deduplicated by key.
    // BTreeMap: key → first new request for that key.
    let mut new_failing_by_key: BTreeMap<String, &crate::contract::NetworkRequest> =
        BTreeMap::new();
    for r in &new_bundle.page.network.requests {
        if !is_failing(r.status, r.failed) {
            continue;
        }
        let k = request_key(&r.url, new_final_url);
        if old_failing.contains(&k) {
            continue; // present on both sides → suppress
        }
        // first-seen wins for dedup (stable because BTreeMap preserves key uniqueness)
        new_failing_by_key.entry(k).or_insert(r);
    }

    // Emit one issue per unique key, sorted by key (BTreeMap iteration is sorted).
    for (key, req) in &new_failing_by_key {
        let url = &req.url;
        let status = req.status;
        let failed = req.failed;
        let request_type = &req.request_type;

        // Find anchor node: first new-page node whose src or href equals the absolute URL.
        let anchor_node: Option<&SemanticNode> = new_bundle.page.nodes.iter().find(|n| {
            n.src.as_deref() == Some(url.as_str()) || n.href.as_deref() == Some(url.as_str())
        });

        let (anchors, locator) = match anchor_node {
            Some(node) => {
                let a = node_to_anchors(node);
                let loc = single_node_locator(a.clone(), node);
                (a, loc)
            }
            None => {
                let a = Anchors::null();
                let loc = null_locator(a.clone());
                (a, loc)
            }
        };

        // Old evidence: the matching old request (if any), even if it wasn't failing.
        let old_evidence = match old_request_by_key.get(key.as_str()) {
            Some((old_url, old_status)) => serde_json::json!({
                "url": old_url,
                "status": old_status
            }),
            None => serde_json::Value::Null,
        };

        let evidence = serde_json::json!({
            "old": old_evidence,
            "new": {
                "url": url,
                "status": status,
                "failed": failed,
                "type": request_type
            }
        });

        let filename = filename_from_url(url);
        let near = anchors.nearest_heading.clone();
        let note = match status {
            Some(s) => format!(
                "Asset returned HTTP {} on the new page (it loaded on the old page). \
                 Restore the asset or fix its path. The grep target may hit repo source or \
                 CMS content; the anchors identify the element either way.",
                s
            ),
            None => "Asset failed to load on the new page (it loaded on the old page). \
                     Restore the asset or fix its path. The grep target may hit repo source or \
                     CMS content; the anchors identify the element either way."
                .to_string(),
        };

        let remediation = serde_json::json!({
            "action": "restore_asset",
            "findBy": { "grep": [filename], "near": near },
            "from": url,
            "to": null,
            "note": note
        });

        let severity = profile.severity_for(&IssueType::NetworkError, &IssueCategory::Technical);
        let confidence = compute_confidence(
            base_confidence::NETWORK_ERROR,
            env_mismatch,
            old_det,
            new_det,
        );
        let id = compute_issue_id(&IssueType::NetworkError, viewport, &anchors, None);

        let status_display = status
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        issues.push(Issue {
            id,
            issue_type: IssueType::NetworkError,
            category: IssueCategory::Technical,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_bundle.page.lang.clone(),
            goal: Some("G7".to_string()),
            message: format!(
                "Network error: {} returned HTTP {} on the new page",
                url, status_display
            ),
            locator,
            evidence,
            remediation: Some(remediation),
        });
    }

    // ------------------------------------------------------------------
    // console_error: new-only error-level console messages
    // ------------------------------------------------------------------

    // Build old error text set (same filter: level=="error", exclude "Failed to load resource")
    let old_errors: BTreeSet<String> = old_bundle
        .page
        .console
        .iter()
        .filter(|e| e.level == "error" && !e.text.starts_with("Failed to load resource"))
        .map(|e| e.text.clone())
        .collect();

    // Collect new-only error messages, deduplicated by text.
    let mut new_errors_by_text: BTreeMap<String, &crate::contract::ConsoleEntry> = BTreeMap::new();
    for entry in &new_bundle.page.console {
        if entry.level != "error" {
            continue;
        }
        if entry.text.starts_with("Failed to load resource") {
            continue;
        }
        if old_errors.contains(&entry.text) {
            continue;
        }
        // first-seen wins for dedup
        new_errors_by_text
            .entry(entry.text.clone())
            .or_insert(entry);
    }

    // Emit one issue per unique text, sorted by text (BTreeMap iteration is sorted).
    for (text, entry) in &new_errors_by_text {
        let null_anchors = Anchors::null();
        let id = compute_issue_id(&IssueType::ConsoleError, viewport, &null_anchors, None);

        let evidence = serde_json::json!({
            "old": null,
            "new": {
                "level": entry.level,
                "text": text
            }
        });

        let remediation = serde_json::json!({
            "action": "investigate_console_error",
            "findBy": { "grep": [text] },
            "note": "The new page logged a console error not present on the old page. \
                     The grep target is the message text; it may live in repo source or a \
                     bundled dependency. The tool does not name the component."
        });

        let severity = profile.severity_for(&IssueType::ConsoleError, &IssueCategory::Technical);
        let confidence = compute_confidence(
            base_confidence::CONSOLE_ERROR,
            env_mismatch,
            old_det,
            new_det,
        );
        let locator = null_locator(null_anchors.clone());

        issues.push(Issue {
            id,
            issue_type: IssueType::ConsoleError,
            category: IssueCategory::Technical,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_bundle.page.lang.clone(),
            goal: None,
            message: format!("Console error: {}", text),
            locator,
            evidence,
            remediation: Some(remediation),
        });
    }

    issues
}

/// Convert a SemanticNode's NodeAnchors to Issue Anchors (mirrors semantic_diff helper).
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

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        A11yInfo, CaptureDeterminism, ConsoleEntry, Environment, NetworkInfo, NetworkRequest,
        NodeAnchors, PageModel, Screenshots, SemanticNode, StepStatus, StyleCandidates,
        ViewportConfig,
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

    fn make_bundle(
        url: &str,
        requests: Vec<NetworkRequest>,
        console: Vec<ConsoleEntry>,
        nodes: Vec<SemanticNode>,
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
                network: NetworkInfo { requests },
                console,
                a11y: A11yInfo { violations: vec![] },
                link_probes: vec![],
            },
            computed_styles: BTreeMap::new(),
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

    fn make_image_node(id: &str, src: &str, nearest_heading: Option<&str>) -> SemanticNode {
        SemanticNode {
            id: id.to_string(),
            kind: "image".to_string(),
            role: None,
            text: None,
            acc_name: None,
            href: None,
            image_alt: Some("test image".to_string()),
            bbox: [10, 20, 100, 80],
            seq_index: 0,
            anchors: NodeAnchors {
                text: None,
                role: None,
                href: None,
                alt: Some("test image".to_string()),
                aria_label: None,
                nearest_heading: nearest_heading.map(str::to_string),
                landmark: None,
                ordinal_in_landmark: None,
            },
            css_selector: Some(format!("img#{}", id)),
            raw_href: None,
            src: Some(src.to_string()),
            natural_width: None,
            natural_height: None,
            loaded: Some(false),
            heading_level: None,
        }
    }

    // --- network_error tests ---

    /// New page has a 404 image request absent (at 200) on old → exactly one network_error
    /// anchored to the image node.
    #[test]
    fn test_network_error_new_only_404() {
        let img_url = "http://localhost:3001/assets/logo.png";
        let old_img_url = "http://localhost:3000/assets/logo.png";

        let old_requests = vec![NetworkRequest {
            url: old_img_url.to_string(),
            status: Some(200),
            request_type: Some("image".to_string()),
            failed: false,
        }];
        let new_requests = vec![NetworkRequest {
            url: img_url.to_string(),
            status: Some(404),
            request_type: Some("image".to_string()),
            failed: false,
        }];

        let img_node = make_image_node("img1", img_url, Some("Performance Analytics"));

        let old_bundle = make_bundle("http://localhost:3000/", old_requests, vec![], vec![]);
        let new_bundle = make_bundle(
            "http://localhost:3001/",
            new_requests,
            vec![],
            vec![img_node],
        );

        let profile = ParityProfile::ContentStructure;
        let issues = network_console_issues(&old_bundle, &new_bundle, "desktop", &profile, false);

        let net_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::NetworkError)
            .collect();

        assert_eq!(net_issues.len(), 1, "should emit exactly one network_error");
        let issue = &net_issues[0];
        assert_eq!(issue.goal, Some("G7".to_string()));
        // Anchor should reference the image node (has alt and nearest_heading)
        assert_eq!(
            issue.locator.anchors.nearest_heading,
            Some("Performance Analytics".to_string())
        );
        assert!(
            issue.locator.css_selector_new.is_some(),
            "should have css_selector_new"
        );
    }

    /// A request failing on BOTH old and new → suppressed (zero issues).
    #[test]
    fn test_network_error_both_sides_suppressed() {
        let old_url = "http://localhost:3000/missing.png";
        let new_url = "http://localhost:3001/missing.png";

        let old_bundle = make_bundle(
            "http://localhost:3000/",
            vec![NetworkRequest {
                url: old_url.to_string(),
                status: Some(404),
                request_type: None,
                failed: false,
            }],
            vec![],
            vec![],
        );
        let new_bundle = make_bundle(
            "http://localhost:3001/",
            vec![NetworkRequest {
                url: new_url.to_string(),
                status: Some(404),
                request_type: None,
                failed: false,
            }],
            vec![],
            vec![],
        );

        let profile = ParityProfile::ContentStructure;
        let issues = network_console_issues(&old_bundle, &new_bundle, "desktop", &profile, false);
        let net_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::NetworkError)
            .collect();
        assert_eq!(
            net_issues.len(),
            0,
            "symmetric failure on both sides must be suppressed"
        );
    }

    /// status==0 and failed==false → NOT a failure.
    #[test]
    fn test_network_error_status_zero_not_failed_ignored() {
        let new_bundle = make_bundle(
            "http://localhost:3001/",
            vec![NetworkRequest {
                url: "http://localhost:3001/script.js".to_string(),
                status: Some(0),
                request_type: None,
                failed: false,
            }],
            vec![],
            vec![],
        );
        let old_bundle = make_bundle("http://localhost:3000/", vec![], vec![], vec![]);

        let profile = ParityProfile::ContentStructure;
        let issues = network_console_issues(&old_bundle, &new_bundle, "desktop", &profile, false);
        let net_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::NetworkError)
            .collect();
        assert_eq!(
            net_issues.len(),
            0,
            "status=0 + !failed must not emit network_error"
        );
    }

    // --- console_error tests ---

    /// New page has error-level message absent on old → one console_error with text in
    /// evidence.new.
    #[test]
    fn test_console_error_new_only() {
        let old_bundle = make_bundle("http://localhost:3000/", vec![], vec![], vec![]);
        let new_bundle = make_bundle(
            "http://localhost:3001/",
            vec![],
            vec![ConsoleEntry {
                level: "error".to_string(),
                text: "Widget failed to initialize".to_string(),
            }],
            vec![],
        );

        let profile = ParityProfile::ContentStructure;
        let issues = network_console_issues(&old_bundle, &new_bundle, "desktop", &profile, false);

        let console_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::ConsoleError)
            .collect();
        assert_eq!(
            console_issues.len(),
            1,
            "should emit exactly one console_error"
        );
        let issue = &console_issues[0];
        // text must be in evidence.new
        let text_in_new = issue.evidence["new"]["text"].as_str().unwrap_or("");
        assert!(
            text_in_new.contains("Widget failed to initialize"),
            "text should be in evidence.new"
        );
    }

    /// "Failed to load resource: ..." console error → NOT emitted (network differ's job).
    #[test]
    fn test_console_error_resource_load_excluded() {
        let old_bundle = make_bundle("http://localhost:3000/", vec![], vec![], vec![]);
        let new_bundle = make_bundle(
            "http://localhost:3001/",
            vec![],
            vec![ConsoleEntry {
                level: "error".to_string(),
                text: "Failed to load resource: the server responded with a status of 404"
                    .to_string(),
            }],
            vec![],
        );

        let profile = ParityProfile::ContentStructure;
        let issues = network_console_issues(&old_bundle, &new_bundle, "desktop", &profile, false);
        let console_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::ConsoleError)
            .collect();
        assert_eq!(
            console_issues.len(),
            0,
            "'Failed to load resource' must not emit console_error"
        );
    }

    /// A console error present on both sides → not emitted.
    #[test]
    fn test_console_error_on_both_sides_suppressed() {
        let entry = ConsoleEntry {
            level: "error".to_string(),
            text: "Something went wrong".to_string(),
        };
        let old_bundle = make_bundle(
            "http://localhost:3000/",
            vec![],
            vec![entry.clone()],
            vec![],
        );
        let new_bundle = make_bundle("http://localhost:3001/", vec![], vec![entry], vec![]);

        let profile = ParityProfile::ContentStructure;
        let issues = network_console_issues(&old_bundle, &new_bundle, "desktop", &profile, false);
        let console_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::ConsoleError)
            .collect();
        assert_eq!(
            console_issues.len(),
            0,
            "error present on both sides must be suppressed"
        );
    }

    // --- determinism test ---

    /// Building the same inputs twice yields identical issue id/order.
    #[test]
    fn test_network_determinism() {
        let img_url_1 = "http://localhost:3001/assets/a.png";
        let img_url_2 = "http://localhost:3001/assets/b.png";

        let new_requests = vec![
            NetworkRequest {
                url: img_url_1.to_string(),
                status: Some(404),
                request_type: Some("image".to_string()),
                failed: false,
            },
            NetworkRequest {
                url: img_url_2.to_string(),
                status: Some(500),
                request_type: Some("image".to_string()),
                failed: false,
            },
        ];

        let old_bundle = make_bundle("http://localhost:3000/", vec![], vec![], vec![]);
        let new_bundle = make_bundle("http://localhost:3001/", new_requests, vec![], vec![]);

        let profile = ParityProfile::ContentStructure;
        let issues1 = network_console_issues(&old_bundle, &new_bundle, "desktop", &profile, false);
        let issues2 = network_console_issues(&old_bundle, &new_bundle, "desktop", &profile, false);

        assert_eq!(issues1.len(), issues2.len());
        for (a, b) in issues1.iter().zip(issues2.iter()) {
            assert_eq!(a.id, b.id, "ids must be identical on repeated calls");
            assert_eq!(
                a.issue_type, b.issue_type,
                "issue_type must be identical on repeated calls"
            );
        }
    }

    // --- path-prefix mount correlation tests (v14/v15/v16 regression guard) ---

    /// Old page at root `/`, new page at path-prefix `/products/connect/branded-call/`.
    ///
    /// (a) Asset 404s on BOTH sides (same relative path from each page's directory)
    ///     → ZERO network_error (symmetric failure, suppressed).
    /// (b) Additionally a new-only 404 (old at 200) → exactly ONE network_error.
    #[test]
    fn test_path_prefix_mount_correlation() {
        // Shared dangling asset — same relative path from each page's base directory.
        // old: http://localhost:3000/assets/images/x.png  (404, failed=true)
        // new: http://localhost:3014/products/connect/branded-call/assets/images/x.png  (404)
        //
        // New-only asset:
        // old: http://localhost:3000/assets/images/only-new.png  (200, not failing)
        // new: http://localhost:3014/products/connect/branded-call/assets/images/only-new.png (404)

        let old_final_url = "http://localhost:3000/";
        let new_final_url = "http://localhost:3014/products/connect/branded-call/";

        let old_requests = vec![
            NetworkRequest {
                url: "http://localhost:3000/assets/images/x.png".to_string(),
                status: Some(404),
                request_type: Some("image".to_string()),
                failed: true,
            },
            NetworkRequest {
                url: "http://localhost:3000/assets/images/only-new.png".to_string(),
                status: Some(200),
                request_type: Some("image".to_string()),
                failed: false,
            },
        ];
        let new_requests = vec![
            NetworkRequest {
                url: "http://localhost:3014/products/connect/branded-call/assets/images/x.png"
                    .to_string(),
                status: Some(404),
                request_type: Some("image".to_string()),
                failed: true,
            },
            NetworkRequest {
                url:
                    "http://localhost:3014/products/connect/branded-call/assets/images/only-new.png"
                        .to_string(),
                status: Some(404),
                request_type: Some("image".to_string()),
                failed: false,
            },
        ];

        // Build bundles manually so we can set different final_urls.
        let make_det_local = || CaptureDeterminism {
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
        };

        let old_bundle = CaptureBundle {
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
            determinism: make_det_local(),
            page: PageModel {
                url: old_final_url.to_string(),
                final_url: old_final_url.to_string(),
                redirect_chain: vec![],
                status_code: 200,
                title: None,
                meta_description: None,
                canonical: None,
                lang: Some("en".to_string()),
                page_height: 2000,
                nodes: vec![],
                landmarks: vec![],
                landmark_rects: None,
                network: NetworkInfo {
                    requests: old_requests,
                },
                console: vec![],
                a11y: A11yInfo { violations: vec![] },
                link_probes: vec![],
            },
            computed_styles: BTreeMap::new(),
            screenshots: Screenshots {
                full_page: "desktop/old.png".to_string(),
                viewport: "desktop/old-vp.png".to_string(),
            },
            style_candidates: StyleCandidates::default(),
            hit_tests: None,
            pseudo_elements: None,
            pseudo_truncated: None,
        };

        let new_bundle = CaptureBundle {
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
            determinism: make_det_local(),
            page: PageModel {
                url: new_final_url.to_string(),
                final_url: new_final_url.to_string(),
                redirect_chain: vec![],
                status_code: 200,
                title: None,
                meta_description: None,
                canonical: None,
                lang: Some("en".to_string()),
                page_height: 2000,
                nodes: vec![],
                landmarks: vec![],
                landmark_rects: None,
                network: NetworkInfo {
                    requests: new_requests,
                },
                console: vec![],
                a11y: A11yInfo { violations: vec![] },
                link_probes: vec![],
            },
            computed_styles: BTreeMap::new(),
            screenshots: Screenshots {
                full_page: "desktop/new.png".to_string(),
                viewport: "desktop/new-vp.png".to_string(),
            },
            style_candidates: StyleCandidates::default(),
            hit_tests: None,
            pseudo_elements: None,
            pseudo_truncated: None,
        };

        let profile = ParityProfile::ContentStructure;
        let issues = network_console_issues(&old_bundle, &new_bundle, "desktop", &profile, false);

        let net_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::NetworkError)
            .collect();

        // (a) x.png fails on both sides → suppressed
        // (b) only-new.png fails on new only → exactly one network_error
        assert_eq!(
            net_issues.len(),
            1,
            "x.png must be suppressed (both-sides 404); only-new.png must emit one network_error"
        );
        assert!(
            net_issues[0].evidence["new"]["url"]
                .as_str()
                .unwrap_or("")
                .contains("only-new.png"),
            "the emitted issue must be for only-new.png"
        );
    }

    /// Verify request_key directly: page-directory-relative keying for same-site URLs.
    #[test]
    fn test_request_key_page_dir_relative() {
        // Root page: base_dir = "/"
        assert_eq!(
            request_key(
                "http://localhost:3000/assets/images/x.png",
                "http://localhost:3000/"
            ),
            "assets/images/x.png"
        );

        // Path-prefix page: base_dir = "/products/connect/branded-call/"
        assert_eq!(
            request_key(
                "http://localhost:3014/products/connect/branded-call/assets/images/x.png",
                "http://localhost:3014/products/connect/branded-call/"
            ),
            "assets/images/x.png"
        );

        // Same-origin but outside page dir (absolute /assets/ while page is /x/y/) → strip leading "/"
        assert_eq!(
            request_key(
                "http://localhost:3014/assets/images/x.png",
                "http://localhost:3014/products/connect/branded-call/"
            ),
            "assets/images/x.png"
        );

        // No trailing slash on own_final_url: "/es_MX/products/connect/branded-call"
        // → base_dir = "/es_MX/products/connect/"
        // asset at "/es_MX/products/connect/assets/images/x.png" → rel = "assets/images/x.png"
        assert_eq!(
            request_key(
                "http://localhost:3015/es_MX/products/connect/assets/images/x.png",
                "http://localhost:3015/es_MX/products/connect/branded-call"
            ),
            "assets/images/x.png"
        );

        // Third-party URL → returned unchanged
        assert_eq!(
            request_key(
                "https://cdn.example.com/font.woff2",
                "http://localhost:3000/"
            ),
            "https://cdn.example.com/font.woff2"
        );

        // Query string preserved
        assert_eq!(
            request_key(
                "http://localhost:3000/assets/x.png?v=2",
                "http://localhost:3000/"
            ),
            "assets/x.png?v=2"
        );
    }
}
