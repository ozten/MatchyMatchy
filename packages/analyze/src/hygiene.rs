//! URL & locale hygiene checks for M2 (M2.md §5.2, §5.3).
//!
//! `hygiene_issues` is a pure function: (old_bundle, new_bundle, viewport, profile) → HygieneOutcome.
//!
//! DETERMINISM: all maps are BTreeMap, sorts use total order ending in stable keys.
//! Float aggregations happen in fixed sorted order.

use std::collections::BTreeMap;

use url::Url;

use crate::config::{base_confidence, TrailingSlashPolicy, DEFAULT_TRAILING_SLASH_POLICY};
use crate::contract::{
    Anchors, CaptureBundle, Issue, IssueCategory, IssueType, Locator, SemanticNode,
};
use crate::egress::{check_probe_url, EgressDecision};
use crate::issue::compute_issue_id;
use crate::locale::detect_locale_in_path;
use crate::scoring::ParityProfile;

/// Output of `hygiene_issues`.
pub struct HygieneOutcome {
    pub issues: Vec<Issue>,
    /// When true, analyze_viewport discards all other issues and uses only these.
    pub short_circuit: bool,
}

/// Run all hygiene checks in fixed emission order (M2.md §5.2).
pub fn hygiene_issues(
    old: &CaptureBundle,
    new: &CaptureBundle,
    viewport: &str,
    profile: &ParityProfile,
) -> HygieneOutcome {
    let new_lang = new.page.lang.clone();
    let mut issues: Vec<Issue> = Vec::new();

    // -------------------------------------------------------------------------
    // Check 1: Status parity (short-circuit)
    // -------------------------------------------------------------------------
    if let Some(issue) = check_status_parity(old, new, viewport, profile, &new_lang) {
        return HygieneOutcome {
            issues: vec![issue],
            short_circuit: true,
        };
    }

    // -------------------------------------------------------------------------
    // Check 2: Trailing slash on new page's final URL
    // -------------------------------------------------------------------------
    if let Some(issue) = check_trailing_slash_page(old, new, viewport, profile, &new_lang) {
        issues.push(issue);
    }

    // -------------------------------------------------------------------------
    // Check 3: Redirect chain on new page
    // -------------------------------------------------------------------------
    if let Some(issue) = check_redirect_chain_page(new, viewport, profile, &new_lang) {
        issues.push(issue);
    }

    // -------------------------------------------------------------------------
    // Check 4: Protocol downgrade (page level + per-link)
    // -------------------------------------------------------------------------
    if let Some(issue) = check_protocol_downgrade_page(old, new, viewport, profile, &new_lang) {
        issues.push(issue);
    }
    let mut link_downgrade_issues =
        check_per_link_protocol_downgrade(old, new, viewport, profile, &new_lang);
    issues.append(&mut link_downgrade_issues);

    // -------------------------------------------------------------------------
    // Check 5: Canonical mismatch
    // -------------------------------------------------------------------------
    if let Some(issue) = check_canonical(old, new, viewport, profile, &new_lang) {
        issues.push(issue);
    }

    // -------------------------------------------------------------------------
    // Check 6: Locale path
    // -------------------------------------------------------------------------
    let mut locale_issues = check_locale_path(new, viewport, profile, &new_lang);
    issues.append(&mut locale_issues);

    // -------------------------------------------------------------------------
    // Check 7: Per-link trailing slash (same-site links on new page)
    // -------------------------------------------------------------------------
    let mut link_slash_issues =
        check_per_link_trailing_slash(old, new, viewport, profile, &new_lang);
    issues.append(&mut link_slash_issues);

    // -------------------------------------------------------------------------
    // Check 8: Per-link redirect chains (from link_probes)
    // -------------------------------------------------------------------------
    let mut link_redirect_issues =
        check_per_link_redirect_chains(new, viewport, profile, &new_lang);
    issues.append(&mut link_redirect_issues);

    HygieneOutcome {
        issues,
        short_circuit: false,
    }
}

// ---------------------------------------------------------------------------
// Check 1: Status parity
// ---------------------------------------------------------------------------

fn check_status_parity(
    old: &CaptureBundle,
    new: &CaptureBundle,
    viewport: &str,
    profile: &ParityProfile,
    new_lang: &Option<String>,
) -> Option<Issue> {
    let old_status = old.page.status_code;
    let new_status = new.page.status_code;

    // old 2xx AND new not-2xx (0 counts as non-2xx)
    let old_2xx = (200..300).contains(&old_status);
    let new_non_2xx = !(200..300).contains(&new_status);

    if !old_2xx || !new_non_2xx {
        return None;
    }

    let severity = profile.severity_for(&IssueType::StatusCodeMismatch, &IssueCategory::Technical);
    let confidence = base_confidence::STATUS_CODE_MISMATCH;

    let old_final = &old.page.final_url;
    let new_final = &new.page.final_url;
    let new_path = url_path_query(new_final).unwrap_or_else(|| "/".to_string());

    let anchors = Anchors {
        text: None,
        role: None,
        href: Some(new_path.clone()),
        alt: None,
        aria_label: None,
        nearest_heading: None,
        landmark: None,
        ordinal_in_landmark: None,
    };

    let id = compute_issue_id(&IssueType::StatusCodeMismatch, viewport, &anchors, None);

    let evidence = serde_json::json!({
        "old": { "statusCode": old_status, "url": old_final },
        "new": { "statusCode": new_status, "url": new_final }
    });

    let remediation = serde_json::json!({
        "action": "fix_route",
        "findBy": { "grep": [new_path] },
        "from": new_status.to_string(),
        "to": "200",
        "note": "New page returns a non-2xx status while old page returned 2xx. Fix the route or content to return 200."
    });

    Some(Issue {
        id,
        issue_type: IssueType::StatusCodeMismatch,
        category: IssueCategory::Technical,
        severity,
        confidence,
        viewport: viewport.to_string(),
        locale: new_lang.clone(),
        goal: None,
        message: format!(
            "Status code mismatch: old={} new={}",
            old_status, new_status
        ),
        locator: Locator {
            anchors,
            css_selector_old: None,
            css_selector_new: None,
            bbox_old: None,
            bbox_new: None,
            seq_index_old: None,
            seq_index_new: None,
        },
        evidence,
        remediation: Some(remediation),
    })
}

// ---------------------------------------------------------------------------
// Check 2: Trailing slash (page level)
// ---------------------------------------------------------------------------

/// Check trailing slash policy on the new page's final URL.
pub fn check_trailing_slash_url(
    url_str: &str,
    old_url_str: Option<&str>,
    policy: TrailingSlashPolicy,
) -> bool {
    // Returns true if a trailing-slash issue should be emitted.
    let path = match Url::parse(url_str) {
        Ok(u) => u.path().to_string(),
        Err(_) => return false,
    };

    // Root path is exempt
    if path == "/" {
        return false;
    }

    match policy {
        TrailingSlashPolicy::Never => path.ends_with('/'),
        TrailingSlashPolicy::Always => !path.ends_with('/'),
        TrailingSlashPolicy::Preserve => {
            let new_has_slash = path.ends_with('/');
            if let Some(old) = old_url_str {
                let old_path = match Url::parse(old) {
                    Ok(u) => u.path().to_string(),
                    Err(_) => return false,
                };
                let old_has_slash = old_path.ends_with('/') && old_path != "/";
                new_has_slash != old_has_slash
            } else {
                false
            }
        }
    }
}

fn check_trailing_slash_page(
    old: &CaptureBundle,
    new: &CaptureBundle,
    viewport: &str,
    profile: &ParityProfile,
    new_lang: &Option<String>,
) -> Option<Issue> {
    let policy = DEFAULT_TRAILING_SLASH_POLICY;
    let new_final = &new.page.final_url;
    let old_final = &old.page.final_url;

    if !check_trailing_slash_url(new_final, Some(old_final), policy) {
        return None;
    }

    build_trailing_slash_issue(new_final, None, policy, viewport, profile, new_lang)
}

fn build_trailing_slash_issue(
    url_str: &str,
    node: Option<&SemanticNode>,
    policy: TrailingSlashPolicy,
    viewport: &str,
    profile: &ParityProfile,
    new_lang: &Option<String>,
) -> Option<Issue> {
    let parsed = Url::parse(url_str).ok()?;
    let path = parsed.path().to_string();
    let query = parsed
        .query()
        .map(|q| format!("?{}", q))
        .unwrap_or_default();
    let full_path_query = format!("{}{}", path, query);

    let policy_str = match policy {
        TrailingSlashPolicy::Never => "never",
        TrailingSlashPolicy::Always => "always",
        TrailingSlashPolicy::Preserve => "preserve",
    };

    // "to" = without trailing slash for Never, with slash for Always
    let path_without = path.trim_end_matches('/');
    let (from_path, to_path) = match policy {
        TrailingSlashPolicy::Never => (
            format!("{}{}", path, query),
            format!("{}{}", path_without, query),
        ),
        TrailingSlashPolicy::Always => (
            format!("{}{}", path, query),
            format!("{}/{}", path_without, query),
        ),
        TrailingSlashPolicy::Preserve => (
            format!("{}{}", path, query),
            // Remove trailing slash for Preserve when new has slash but old doesn't
            format!("{}{}", path_without, query),
        ),
    };

    let (anchors, css_selector_new, bbox_new, seq_index_new) = if let Some(n) = node {
        let a = Anchors {
            text: n.anchors.text.clone(),
            role: n.anchors.role.clone(),
            href: n.anchors.href.clone(),
            alt: n.anchors.alt.clone(),
            aria_label: n.anchors.aria_label.clone(),
            nearest_heading: n.anchors.nearest_heading.clone(),
            landmark: n.anchors.landmark.clone(),
            ordinal_in_landmark: n.anchors.ordinal_in_landmark,
        };
        (a, n.css_selector.clone(), Some(n.bbox), Some(n.seq_index))
    } else {
        let a = Anchors {
            text: None,
            role: None,
            href: Some(full_path_query.clone()),
            alt: None,
            aria_label: None,
            nearest_heading: None,
            landmark: None,
            ordinal_in_landmark: None,
        };
        (a, None, None, None)
    };

    let id = compute_issue_id(&IssueType::UrlTrailingSlash, viewport, &anchors, None);
    let severity = profile.severity_for(&IssueType::UrlTrailingSlash, &IssueCategory::Hygiene);
    let confidence = base_confidence::HYGIENE;

    let evidence = serde_json::json!({
        "new": { "url": url_str, "path": path },
        "policy": policy_str
    });

    let remediation = serde_json::json!({
        "action": "rewrite_url",
        "findBy": { "grep": [from_path] },
        "from": from_path,
        "to": to_path,
        "note": format!("Trailing slash policy is '{}'. Rewrite the URL to conform.", policy_str)
    });

    Some(Issue {
        id,
        issue_type: IssueType::UrlTrailingSlash,
        category: IssueCategory::Hygiene,
        severity,
        confidence,
        viewport: viewport.to_string(),
        locale: new_lang.clone(),
        goal: Some("G5".to_string()),
        message: format!(
            "URL has a trailing slash that should not be present: {}",
            path
        ),
        locator: Locator {
            anchors,
            css_selector_old: None,
            css_selector_new,
            bbox_old: None,
            bbox_new,
            seq_index_old: None,
            seq_index_new,
        },
        evidence,
        remediation: Some(remediation),
    })
}

// ---------------------------------------------------------------------------
// Check 3: Redirect chain (page level)
// ---------------------------------------------------------------------------

fn check_redirect_chain_page(
    new: &CaptureBundle,
    viewport: &str,
    profile: &ParityProfile,
    new_lang: &Option<String>,
) -> Option<Issue> {
    let chain = &new.page.redirect_chain;
    // chain.len() > 1 means at least 2 hops
    if chain.len() <= 1 {
        return None;
    }

    let requested_url = chain.first().map(|s| s.as_str()).unwrap_or(&new.page.url);
    let final_url = &new.page.final_url;
    let hops = chain.len();

    let requested_path = url_path_query(requested_url).unwrap_or_else(|| "/".to_string());
    let anchors = Anchors {
        text: None,
        role: None,
        href: Some(requested_path.clone()),
        alt: None,
        aria_label: None,
        nearest_heading: None,
        landmark: None,
        ordinal_in_landmark: None,
    };

    build_redirect_chain_issue(
        requested_url,
        chain,
        final_url,
        hops,
        None,
        anchors,
        viewport,
        profile,
        new_lang,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_redirect_chain_issue(
    requested_url: &str,
    chain: &[String],
    final_url: &str,
    hops: usize,
    _node: Option<&SemanticNode>,
    anchors: Anchors,
    viewport: &str,
    profile: &ParityProfile,
    new_lang: &Option<String>,
) -> Option<Issue> {
    let requested_path = url_path_query(requested_url).unwrap_or_else(|| "/".to_string());
    let id = compute_issue_id(&IssueType::UrlRedirectChain, viewport, &anchors, None);
    let severity = profile.severity_for(&IssueType::UrlRedirectChain, &IssueCategory::Hygiene);
    let confidence = base_confidence::HYGIENE;

    let evidence = serde_json::json!({
        "new": {
            "requestedUrl": requested_url,
            "redirectChain": chain,
            "finalUrl": final_url,
            "hops": hops
        }
    });

    let remediation = serde_json::json!({
        "action": "update_link_target",
        "findBy": { "grep": [requested_path] },
        "from": requested_url,
        "to": final_url,
        "note": "Update references to point directly to the final URL, avoiding the redirect chain."
    });

    Some(Issue {
        id,
        issue_type: IssueType::UrlRedirectChain,
        category: IssueCategory::Hygiene,
        severity,
        confidence,
        viewport: viewport.to_string(),
        locale: new_lang.clone(),
        goal: Some("G5".to_string()),
        message: format!("URL redirect chain has {} hops: {}", hops, requested_url),
        locator: Locator {
            anchors,
            css_selector_old: None,
            css_selector_new: None,
            bbox_old: None,
            bbox_new: None,
            seq_index_old: None,
            seq_index_new: None,
        },
        evidence,
        remediation: Some(remediation),
    })
}

// ---------------------------------------------------------------------------
// Check 4: Protocol downgrade (page level + per-link)
// ---------------------------------------------------------------------------

fn check_protocol_downgrade_page(
    old: &CaptureBundle,
    new: &CaptureBundle,
    viewport: &str,
    profile: &ParityProfile,
    new_lang: &Option<String>,
) -> Option<Issue> {
    let new_scheme = url_scheme(&new.page.final_url)?;
    let old_scheme = url_scheme(&old.page.final_url)?;

    if new_scheme == "http" && old_scheme == "https" {
        let new_url = &new.page.final_url;
        let old_url = &old.page.final_url;
        let https_url = new_url.replacen("http://", "https://", 1);
        let url_sans_scheme = new_url
            .trim_start_matches("http://")
            .trim_start_matches("https://");

        let path = url_path_query(new_url).unwrap_or_else(|| "/".to_string());
        let anchors = Anchors {
            text: None,
            role: None,
            href: Some(path),
            alt: None,
            aria_label: None,
            nearest_heading: None,
            landmark: None,
            ordinal_in_landmark: None,
        };

        let id = compute_issue_id(&IssueType::UrlProtocolDowngrade, viewport, &anchors, None);
        let severity =
            profile.severity_for(&IssueType::UrlProtocolDowngrade, &IssueCategory::Hygiene);
        let confidence = base_confidence::HYGIENE;

        let evidence = serde_json::json!({
            "old": { "url": old_url },
            "new": { "url": new_url }
        });

        let remediation = serde_json::json!({
            "action": "rewrite_url",
            "findBy": { "grep": [url_sans_scheme] },
            "from": new_url,
            "to": https_url,
            "note": "New page uses HTTP where old page used HTTPS. Update to HTTPS."
        });

        return Some(Issue {
            id,
            issue_type: IssueType::UrlProtocolDowngrade,
            category: IssueCategory::Hygiene,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_lang.clone(),
            goal: Some("G5".to_string()),
            message: format!("Protocol downgrade: old={} new={}", old_url, new_url),
            locator: Locator {
                anchors,
                css_selector_old: None,
                css_selector_new: None,
                bbox_old: None,
                bbox_new: None,
                seq_index_old: None,
                seq_index_new: None,
            },
            evidence,
            remediation: Some(remediation),
        });
    }
    None
}

/// Per-link protocol downgrade (M2.md §5.2 item 4b).
///
/// For each same-site http link on the new page whose host-stripped URL (path+query)
/// has an https twin in the old page links → emit url_protocol_downgrade with link node anchors.
fn check_per_link_protocol_downgrade(
    old: &CaptureBundle,
    new: &CaptureBundle,
    viewport: &str,
    profile: &ParityProfile,
    new_lang: &Option<String>,
) -> Vec<Issue> {
    // Build host-stripped → scheme map for old-page links.
    // Key = path+query; value = scheme.
    let mut old_link_schemes: BTreeMap<String, String> = BTreeMap::new();
    for node in &old.page.nodes {
        if let Some(href) = &node.href {
            if let Ok(u) = Url::parse(href) {
                let scheme = u.scheme().to_string();
                if let Some(hs) = host_stripped(href) {
                    // If multiple old links have the same host-stripped URL, prefer https
                    old_link_schemes
                        .entry(hs)
                        .and_modify(|s| {
                            if scheme == "https" {
                                *s = scheme.clone();
                            }
                        })
                        .or_insert(scheme);
                }
            }
        }
    }

    // Get new page host+port for same-site determination
    let new_host = match Url::parse(&new.page.final_url) {
        Ok(u) => u.host_str().unwrap_or("").to_string(),
        Err(_) => return vec![],
    };
    let new_port = match Url::parse(&new.page.final_url) {
        Ok(u) => u.port(),
        Err(_) => None,
    };

    let mut issues = Vec::new();

    // Collect same-site http links on new page, sorted for determinism
    let mut candidates: Vec<&SemanticNode> = new
        .page
        .nodes
        .iter()
        .filter(|node| {
            if let Some(href) = &node.href {
                if let Ok(u) = Url::parse(href) {
                    let link_host = u.host_str().unwrap_or("");
                    let link_port = u.port();
                    let same_site =
                        link_host.eq_ignore_ascii_case(&new_host) && link_port == new_port;
                    return same_site && u.scheme() == "http";
                }
            }
            false
        })
        .collect();

    candidates.sort_by(|a, b| a.seq_index.cmp(&b.seq_index).then_with(|| a.id.cmp(&b.id)));

    for node in candidates {
        let href = match &node.href {
            Some(h) => h,
            None => continue,
        };

        let hs = match host_stripped(href) {
            Some(s) => s,
            None => continue,
        };

        // Check if old page has the same host-stripped URL with https scheme
        if old_link_schemes.get(&hs).map(|s| s.as_str()) != Some("https") {
            continue;
        }

        let https_url = href.replacen("http://", "https://", 1);
        let url_sans_scheme = href
            .trim_start_matches("http://")
            .trim_start_matches("https://");

        let anchors = Anchors {
            text: node.anchors.text.clone(),
            role: node.anchors.role.clone(),
            href: node.anchors.href.clone(),
            alt: node.anchors.alt.clone(),
            aria_label: node.anchors.aria_label.clone(),
            nearest_heading: node.anchors.nearest_heading.clone(),
            landmark: node.anchors.landmark.clone(),
            ordinal_in_landmark: node.anchors.ordinal_in_landmark,
        };

        let id = compute_issue_id(&IssueType::UrlProtocolDowngrade, viewport, &anchors, None);
        let severity =
            profile.severity_for(&IssueType::UrlProtocolDowngrade, &IssueCategory::Hygiene);
        let confidence = base_confidence::HYGIENE;

        let evidence = serde_json::json!({
            "old": { "url": https_url },
            "new": { "url": href }
        });

        let remediation = serde_json::json!({
            "action": "rewrite_url",
            "findBy": { "grep": [url_sans_scheme] },
            "from": href,
            "to": https_url,
            "note": "Link uses HTTP where old page had HTTPS for the same path. Update to HTTPS."
        });

        issues.push(Issue {
            id,
            issue_type: IssueType::UrlProtocolDowngrade,
            category: IssueCategory::Hygiene,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_lang.clone(),
            goal: Some("G5".to_string()),
            message: format!("Per-link protocol downgrade: {} should be https", href),
            locator: Locator {
                anchors,
                css_selector_old: None,
                css_selector_new: node.css_selector.clone(),
                bbox_old: None,
                bbox_new: Some(node.bbox),
                seq_index_old: None,
                seq_index_new: Some(node.seq_index),
            },
            evidence,
            remediation: Some(remediation),
        });
    }

    issues
}

// ---------------------------------------------------------------------------
// Check 5: Canonical mismatch
// ---------------------------------------------------------------------------

fn check_canonical(
    old: &CaptureBundle,
    new: &CaptureBundle,
    viewport: &str,
    profile: &ParityProfile,
    new_lang: &Option<String>,
) -> Option<Issue> {
    let new_canonical_raw = new.page.canonical.as_deref()?;

    // Parity suppression: if old canonical (raw) == new canonical (raw), suppress
    if let Some(old_canonical_raw) = old.page.canonical.as_deref() {
        if old_canonical_raw == new_canonical_raw {
            return None;
        }
    }

    let new_final = &new.page.final_url;
    let base = Url::parse(new_final).ok()?;

    // Resolve canonical against new final URL
    let resolved = base.join(new_canonical_raw).ok()?;

    // Normalize: strip fragment; strip non-root trailing slash under Never/Preserve
    let canonical_resolved = normalize_url_for_compare(&resolved, DEFAULT_TRAILING_SLASH_POLICY);
    let final_normalized = {
        let u = Url::parse(new_final).ok()?;
        normalize_url_for_compare(&u, DEFAULT_TRAILING_SLASH_POLICY)
    };

    if canonical_resolved == final_normalized {
        return None;
    }

    let path_query = url_path_query(new_final).unwrap_or_else(|| "/".to_string());
    let anchors = Anchors {
        text: None,
        role: None,
        href: Some(path_query),
        alt: None,
        aria_label: None,
        nearest_heading: None,
        landmark: None,
        ordinal_in_landmark: None,
    };

    let id = compute_issue_id(&IssueType::CanonicalMismatch, viewport, &anchors, None);
    let severity = profile.severity_for(&IssueType::CanonicalMismatch, &IssueCategory::Hygiene);
    let confidence = base_confidence::HYGIENE;

    let old_canonical_raw = old.page.canonical.as_deref().unwrap_or("");

    let evidence = serde_json::json!({
        "old": { "canonical": old_canonical_raw },
        "new": {
            "canonical": new_canonical_raw,
            "canonicalResolved": canonical_resolved,
            "finalUrl": new_final
        }
    });

    let remediation = serde_json::json!({
        "action": "update_canonical",
        "findBy": { "grep": ["rel=\"canonical\""] },
        "from": canonical_resolved,
        "to": final_normalized,
        "note": "The canonical URL does not match the final URL. Update the canonical tag to point to the correct URL."
    });

    Some(Issue {
        id,
        issue_type: IssueType::CanonicalMismatch,
        category: IssueCategory::Hygiene,
        severity,
        confidence,
        viewport: viewport.to_string(),
        locale: new_lang.clone(),
        goal: Some("G5".to_string()),
        message: format!(
            "Canonical URL mismatch: canonical resolves to {} but final URL is {}",
            canonical_resolved, final_normalized
        ),
        locator: Locator {
            anchors,
            css_selector_old: None,
            css_selector_new: None,
            bbox_old: None,
            bbox_new: None,
            seq_index_old: None,
            seq_index_new: None,
        },
        evidence,
        remediation: Some(remediation),
    })
}

/// Normalize a URL for canonical comparison:
/// strip fragment; under Never/Preserve, strip non-root trailing slash from path.
fn normalize_url_for_compare(url: &Url, policy: TrailingSlashPolicy) -> String {
    let mut u = url.clone();
    u.set_fragment(None);
    match policy {
        TrailingSlashPolicy::Never | TrailingSlashPolicy::Preserve => {
            let path = u.path().to_string();
            if path != "/" && path.ends_with('/') {
                let trimmed = path.trim_end_matches('/').to_string();
                u.set_path(&trimmed);
            }
        }
        TrailingSlashPolicy::Always => {}
    }
    u.to_string()
}

// ---------------------------------------------------------------------------
// Check 6: Locale path
// ---------------------------------------------------------------------------

fn check_locale_path(
    new: &CaptureBundle,
    viewport: &str,
    profile: &ParityProfile,
    new_lang: &Option<String>,
) -> Vec<Issue> {
    let new_final = &new.page.final_url;
    let path = match Url::parse(new_final) {
        Ok(u) => u.path().to_string(),
        Err(_) => return vec![],
    };

    let (raw_seg, validation) = match detect_locale_in_path(&path) {
        Some(r) => r,
        None => return vec![],
    };

    let mut issues = Vec::new();

    let url_path_q = url_path_query(new_final).unwrap_or_else(|| "/".to_string());

    // Rule 1: separator invalid
    if validation.separator_invalid {
        let corrected_seg = raw_seg.replace('_', "-");
        let corrected_path = path.replace(&raw_seg, &corrected_seg);
        let corrected_url_path = url_path_q.replace(&raw_seg, &corrected_seg);

        let anchors = Anchors {
            text: None,
            role: None,
            href: Some(url_path_q.clone()),
            alt: None,
            aria_label: None,
            nearest_heading: None,
            landmark: None,
            ordinal_in_landmark: None,
        };

        let id = compute_issue_id(&IssueType::LocaleSeparatorInvalid, viewport, &anchors, None);
        let severity =
            profile.severity_for(&IssueType::LocaleSeparatorInvalid, &IssueCategory::Hygiene);
        let confidence = base_confidence::HYGIENE;

        let expected_seg = corrected_seg.clone();
        let evidence = serde_json::json!({
            "new": {
                "url": new_final,
                "localeSegment": raw_seg,
                "expected": expected_seg
            }
        });

        let _ = corrected_path; // used for clarity
        let remediation = serde_json::json!({
            "action": "rewrite_url",
            "findBy": { "grep": [raw_seg] },
            "from": url_path_q.clone(),
            "to": corrected_url_path,
            "note": "Locale separator should be a hyphen (-), not underscore (_). Update all references."
        });

        issues.push(Issue {
            id,
            issue_type: IssueType::LocaleSeparatorInvalid,
            category: IssueCategory::Hygiene,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_lang.clone(),
            goal: Some("G6".to_string()),
            message: format!(
                "Locale separator invalid in segment '{}': use hyphen not underscore",
                raw_seg
            ),
            locator: Locator {
                anchors,
                css_selector_old: None,
                css_selector_new: None,
                bbox_old: None,
                bbox_new: None,
                seq_index_old: None,
                seq_index_new: None,
            },
            evidence,
            remediation: Some(remediation),
        });
    }

    // Rule 2: case invalid
    if validation.case_invalid {
        let anchors = Anchors {
            text: None,
            role: None,
            href: Some(url_path_q.clone()),
            alt: None,
            aria_label: None,
            nearest_heading: None,
            landmark: None,
            ordinal_in_landmark: None,
        };

        let id = compute_issue_id(&IssueType::LocaleCaseInvalid, viewport, &anchors, None);
        let severity = profile.severity_for(&IssueType::LocaleCaseInvalid, &IssueCategory::Hygiene);
        let confidence = base_confidence::HYGIENE;

        let corrected_seg = &validation.corrected_segment;
        let corrected_url_path = url_path_q.replace(&raw_seg, corrected_seg);

        let evidence = serde_json::json!({
            "new": {
                "url": new_final,
                "localeSegment": raw_seg,
                "expected": corrected_seg
            }
        });

        let remediation = serde_json::json!({
            "action": "rewrite_url",
            "findBy": { "grep": [raw_seg] },
            "from": url_path_q.clone(),
            "to": corrected_url_path,
            "note": "Locale segment case is invalid. Language code must be lowercase, region code must be uppercase (e.g. es-MX)."
        });

        issues.push(Issue {
            id,
            issue_type: IssueType::LocaleCaseInvalid,
            category: IssueCategory::Hygiene,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_lang.clone(),
            goal: Some("G6".to_string()),
            message: format!(
                "Locale case invalid in segment '{}': expected '{}'",
                raw_seg, corrected_seg
            ),
            locator: Locator {
                anchors,
                css_selector_old: None,
                css_selector_new: None,
                bbox_old: None,
                bbox_new: None,
                seq_index_old: None,
                seq_index_new: None,
            },
            evidence,
            remediation: Some(remediation),
        });
    }

    // Rule 3: unknown
    if validation.unknown {
        let anchors = Anchors {
            text: None,
            role: None,
            href: Some(url_path_q.clone()),
            alt: None,
            aria_label: None,
            nearest_heading: None,
            landmark: None,
            ordinal_in_landmark: None,
        };

        let id = compute_issue_id(&IssueType::LocaleUnknown, viewport, &anchors, None);
        let severity = profile.severity_for(&IssueType::LocaleUnknown, &IssueCategory::Hygiene);
        let confidence = base_confidence::HYGIENE;

        let unknown_subtag = validation.unknown_subtag.as_deref().unwrap_or(&raw_seg);

        let evidence = serde_json::json!({
            "new": {
                "url": new_final,
                "localeSegment": raw_seg,
                "unknownSubtag": unknown_subtag
            }
        });

        issues.push(Issue {
            id,
            issue_type: IssueType::LocaleUnknown,
            category: IssueCategory::Hygiene,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_lang.clone(),
            goal: Some("G6".to_string()),
            message: format!(
                "Unknown locale subtag '{}' in segment '{}'",
                unknown_subtag, raw_seg
            ),
            locator: Locator {
                anchors,
                css_selector_old: None,
                css_selector_new: None,
                bbox_old: None,
                bbox_new: None,
                seq_index_old: None,
                seq_index_new: None,
            },
            evidence,
            remediation: None, // locale_unknown remediation is null
        });
    }

    issues
}

// ---------------------------------------------------------------------------
// Check 7: Per-link trailing slash
// ---------------------------------------------------------------------------

fn check_per_link_trailing_slash(
    old: &CaptureBundle,
    new: &CaptureBundle,
    viewport: &str,
    profile: &ParityProfile,
    new_lang: &Option<String>,
) -> Vec<Issue> {
    let policy = DEFAULT_TRAILING_SLASH_POLICY;

    // Get new page host for same-site determination
    let new_host = match Url::parse(&new.page.final_url) {
        Ok(u) => u.host_str().unwrap_or("").to_string(),
        Err(_) => return vec![],
    };
    let new_port = match Url::parse(&new.page.final_url) {
        Ok(u) => u.port(),
        Err(_) => None,
    };

    // Build a set of old-page host-stripped URLs for parity suppression
    // host-stripped = path + ?query
    let mut old_link_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for node in &old.page.nodes {
        if let Some(href) = &node.href {
            if let Some(hs) = host_stripped(href) {
                old_link_set.insert(hs);
            }
        }
    }

    // Collect new-page same-site link nodes, sort by seqIndex then id for determinism
    // Use BTreeMap keyed by (seqIndex, id) to maintain order
    let mut same_site_nodes: Vec<&SemanticNode> = new
        .page
        .nodes
        .iter()
        .filter(|node| {
            if let Some(href) = &node.href {
                if let Ok(u) = Url::parse(href) {
                    let link_host = u.host_str().unwrap_or("");
                    let link_port = u.port();
                    // Same-site: host (including port) equality
                    return link_host.eq_ignore_ascii_case(&new_host) && link_port == new_port;
                }
            }
            false
        })
        .collect();

    // Sort for determinism
    same_site_nodes.sort_by(|a, b| a.seq_index.cmp(&b.seq_index).then_with(|| a.id.cmp(&b.id)));

    let mut issues = Vec::new();

    for node in same_site_nodes {
        let href = match &node.href {
            Some(h) => h,
            None => continue,
        };

        // Check trailing slash policy
        if !check_trailing_slash_url(href, None, policy) {
            continue;
        }

        // Parity suppression: if old page has a link with identical host-stripped URL, suppress
        if let Some(hs) = host_stripped(href) {
            if old_link_set.contains(&hs) {
                continue;
            }
        }

        if let Some(issue) =
            build_trailing_slash_issue(href, Some(node), policy, viewport, profile, new_lang)
        {
            issues.push(issue);
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// Check 8: Per-link redirect chains (from link_probes)
// ---------------------------------------------------------------------------

fn check_per_link_redirect_chains(
    new: &CaptureBundle,
    viewport: &str,
    profile: &ParityProfile,
    new_lang: &Option<String>,
) -> Vec<Issue> {
    let page_final_url = &new.page.final_url;

    // Build a BTreeMap from host-stripped fragment-stripped href → lowest-seqIndex node
    // This is used to find the owning node for a probe URL
    let mut href_to_node: BTreeMap<String, &SemanticNode> = BTreeMap::new();
    for node in &new.page.nodes {
        if let Some(href) = &node.href {
            // Resolve relative hrefs against new final URL
            let absolute = resolve_href(href, page_final_url);
            // Strip fragment
            let stripped = strip_fragment(&absolute);
            href_to_node
                .entry(stripped)
                .and_modify(|existing| {
                    // Keep lowest seqIndex; tie-break by id
                    if node.seq_index < existing.seq_index
                        || (node.seq_index == existing.seq_index && node.id < existing.id)
                    {
                        *existing = node;
                    }
                })
                .or_insert(node);
        }
    }

    let mut issues = Vec::new();

    // Sort probes by url for determinism
    let mut probes: Vec<_> = new.page.link_probes.iter().collect();
    probes.sort_by(|a, b| a.url.cmp(&b.url));

    for probe in probes {
        // Apply egress guard
        if check_probe_url(&probe.url, page_final_url) != EgressDecision::Allow {
            continue;
        }

        // Only records where skipped == null and error == null
        if probe.skipped.is_some() || probe.error.is_some() {
            continue;
        }

        // Only if redirect_chain.len() > 1
        if probe.redirect_chain.len() <= 1 {
            continue;
        }

        let final_url = probe.final_url.as_deref().unwrap_or(&probe.url);
        let hops = probe.redirect_chain.len();

        // Find owning node: lowest seqIndex whose fragment-stripped resolved href equals probe.url
        let probe_url_stripped = strip_fragment(&probe.url);
        let node = href_to_node.get(&probe_url_stripped).copied();

        let anchors = if let Some(n) = node {
            Anchors {
                text: n.anchors.text.clone(),
                role: n.anchors.role.clone(),
                href: n.anchors.href.clone(),
                alt: n.anchors.alt.clone(),
                aria_label: n.anchors.aria_label.clone(),
                nearest_heading: n.anchors.nearest_heading.clone(),
                landmark: n.anchors.landmark.clone(),
                ordinal_in_landmark: n.anchors.ordinal_in_landmark,
            }
        } else {
            let path = url_path_query(&probe.url).unwrap_or_else(|| "/".to_string());
            Anchors {
                text: None,
                role: None,
                href: Some(path),
                alt: None,
                aria_label: None,
                nearest_heading: None,
                landmark: None,
                ordinal_in_landmark: None,
            }
        };

        let (css_selector_new, bbox_new, seq_index_new) = if let Some(n) = node {
            (n.css_selector.clone(), Some(n.bbox), Some(n.seq_index))
        } else {
            (None, None, None)
        };

        if let Some(mut issue) = build_redirect_chain_issue(
            &probe.url,
            &probe.redirect_chain,
            final_url,
            hops,
            node,
            anchors,
            viewport,
            profile,
            new_lang,
        ) {
            // For per-link issues, add node locator fields
            issue.locator.css_selector_new = css_selector_new;
            issue.locator.bbox_new = bbox_new;
            issue.locator.seq_index_new = seq_index_new;
            issues.push(issue);
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// URL helpers
// ---------------------------------------------------------------------------

/// Extract the scheme from a URL string.
fn url_scheme(url_str: &str) -> Option<String> {
    Url::parse(url_str).ok().map(|u| u.scheme().to_string())
}

/// Extract path + query (no scheme, host, port, fragment).
pub fn url_path_query(url_str: &str) -> Option<String> {
    let u = Url::parse(url_str).ok()?;
    let path = u.path().to_string();
    let query = u.query().map(|q| format!("?{}", q)).unwrap_or_default();
    Some(format!("{}{}", path, query))
}

/// Host-stripped form: path + ?query (no scheme/host/port/fragment).
/// Used for old↔new link parity joins.
pub fn host_stripped(url_str: &str) -> Option<String> {
    url_path_query(url_str)
}

/// Resolve a potentially-relative href against a base URL.
fn resolve_href(href: &str, base: &str) -> String {
    if let Ok(base_url) = Url::parse(base) {
        if let Ok(resolved) = base_url.join(href) {
            return resolved.to_string();
        }
    }
    href.to_string()
}

/// Strip fragment from a URL string.
fn strip_fragment(url_str: &str) -> String {
    if let Ok(mut u) = Url::parse(url_str) {
        u.set_fragment(None);
        return u.to_string();
    }
    url_str.to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        A11yInfo, CaptureDeterminism, Environment, IssueSeverity, LinkProbe, NetworkInfo,
        PageModel, Screenshots, SemanticNode, StepStatus, ViewportConfig,
    };
    use crate::scoring::ParityProfile;

    // -------------------------------------------------------------------------
    // Test helper: build a minimal CaptureBundle
    // -------------------------------------------------------------------------

    fn make_determinism() -> CaptureDeterminism {
        CaptureDeterminism {
            animations_disabled: StepStatus::Ran,
            reduced_motion: StepStatus::Ran,
            time_frozen: StepStatus::Ran,
            random_stubbed: StepStatus::Ran,
            fonts_ready: StepStatus::Ran,
            images_decoded: StepStatus::Ran,
            lazy_load_pass: StepStatus::Ran,
            settled: StepStatus::Ran,
            clicked: vec![],
            hidden: vec![],
            masked: vec![],
            retried_without_time_freeze: false,
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

    fn make_viewport() -> ViewportConfig {
        ViewportConfig {
            name: "desktop".to_string(),
            width: 1440,
            height: 1000,
            dsf: 1.0,
        }
    }

    fn make_page(
        url: &str,
        final_url: &str,
        redirect_chain: Vec<String>,
        status: u32,
    ) -> PageModel {
        PageModel {
            url: url.to_string(),
            final_url: final_url.to_string(),
            redirect_chain,
            status_code: status,
            title: None,
            meta_description: None,
            canonical: None,
            lang: Some("en".to_string()),
            page_height: 1000,
            nodes: vec![],
            landmarks: vec![],
            network: NetworkInfo { requests: vec![] },
            console: vec![],
            a11y: A11yInfo { violations: vec![] },
            link_probes: vec![],
        }
    }

    fn make_bundle(
        url: &str,
        final_url: &str,
        redirect_chain: Vec<String>,
        status: u32,
    ) -> CaptureBundle {
        CaptureBundle {
            schema_version: "1.0".to_string(),
            captured_at: "2026-01-01T00:00:00Z".to_string(),
            viewport: make_viewport(),
            environment: make_env(),
            determinism: make_determinism(),
            page: make_page(url, final_url, redirect_chain, status),
            computed_styles: Default::default(),
            screenshots: Screenshots {
                full_page: "desktop/old.png".to_string(),
                viewport: "desktop/old-vp.png".to_string(),
            },
            style_candidates: Default::default(),
        }
    }

    fn profile() -> ParityProfile {
        ParityProfile::ContentStructure
    }

    // -------------------------------------------------------------------------
    // Trailing slash tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_trailing_slash_never_root_exempt() {
        // Root "/" should not fire under Never policy
        assert!(!check_trailing_slash_url(
            "http://localhost:3000/",
            None,
            TrailingSlashPolicy::Never
        ));
    }

    #[test]
    fn test_trailing_slash_never_fires_on_non_root() {
        // /products/ should fire under Never
        assert!(check_trailing_slash_url(
            "http://localhost:3000/products/",
            None,
            TrailingSlashPolicy::Never
        ));
    }

    #[test]
    fn test_trailing_slash_never_no_slash_ok() {
        // /products (no slash) is fine under Never
        assert!(!check_trailing_slash_url(
            "http://localhost:3000/products",
            None,
            TrailingSlashPolicy::Never
        ));
    }

    #[test]
    fn test_trailing_slash_always_fires_on_missing() {
        // /products should fire under Always
        assert!(check_trailing_slash_url(
            "http://localhost:3000/products",
            None,
            TrailingSlashPolicy::Always
        ));
    }

    #[test]
    fn test_trailing_slash_always_root_exempt() {
        // Root "/" should NOT fire under Always (already has slash)
        assert!(!check_trailing_slash_url(
            "http://localhost:3000/",
            None,
            TrailingSlashPolicy::Always
        ));
    }

    #[test]
    fn test_trailing_slash_always_with_slash_ok() {
        // /products/ is fine under Always
        assert!(!check_trailing_slash_url(
            "http://localhost:3000/products/",
            None,
            TrailingSlashPolicy::Always
        ));
    }

    #[test]
    fn test_trailing_slash_preserve_mismatch() {
        // old has slash, new does not → fires
        assert!(check_trailing_slash_url(
            "http://localhost:3000/products",
            Some("http://localhost:3000/products/"),
            TrailingSlashPolicy::Preserve
        ));
    }

    #[test]
    fn test_trailing_slash_preserve_match_no_slash() {
        // both have no slash → ok
        assert!(!check_trailing_slash_url(
            "http://localhost:3000/products",
            Some("http://localhost:3000/products"),
            TrailingSlashPolicy::Preserve
        ));
    }

    #[test]
    fn test_trailing_slash_preserve_match_with_slash() {
        // both have slash → ok
        assert!(!check_trailing_slash_url(
            "http://localhost:3000/products/",
            Some("http://localhost:3000/products/"),
            TrailingSlashPolicy::Preserve
        ));
    }

    #[test]
    fn test_trailing_slash_query_preservation() {
        // /products/?q=1 should fire (has trailing slash with query)
        assert!(check_trailing_slash_url(
            "http://localhost:3000/products/?q=1",
            None,
            TrailingSlashPolicy::Never
        ));
    }

    // -------------------------------------------------------------------------
    // Redirect chain tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_redirect_chain_zero_hops_no_issue() {
        let new_b = make_bundle(
            "http://localhost:3017/",
            "http://localhost:3017/",
            vec![],
            200,
        );
        let result =
            check_redirect_chain_page(&new_b, "desktop", &profile(), &Some("en".to_string()));
        assert!(result.is_none(), "0-hop chain should not emit issue");
    }

    #[test]
    fn test_redirect_chain_one_hop_no_issue() {
        // 1 entry in chain = only 1 hop, need > 1
        let new_b = make_bundle(
            "http://localhost:3017/start",
            "http://localhost:3017/",
            vec!["http://localhost:3017/start".to_string()],
            200,
        );
        let result =
            check_redirect_chain_page(&new_b, "desktop", &profile(), &Some("en".to_string()));
        assert!(
            result.is_none(),
            "1-hop chain should not emit issue (need > 1)"
        );
    }

    #[test]
    fn test_redirect_chain_two_hops_fires() {
        let new_b = make_bundle(
            "http://localhost:3017/start",
            "http://localhost:3017/",
            vec![
                "http://localhost:3017/start".to_string(),
                "http://localhost:3017/mid".to_string(),
            ],
            200,
        );
        let result =
            check_redirect_chain_page(&new_b, "desktop", &profile(), &Some("en".to_string()));
        assert!(result.is_some(), "2-hop chain should emit issue");

        let issue = result.unwrap();
        assert_eq!(issue.issue_type, IssueType::UrlRedirectChain);
        // Evidence should contain the chain
        let evidence = &issue.evidence;
        assert_eq!(evidence["new"]["hops"], 2);
        assert_eq!(
            evidence["new"]["redirectChain"][0],
            "http://localhost:3017/start"
        );
        assert_eq!(
            evidence["new"]["redirectChain"][1],
            "http://localhost:3017/mid"
        );
    }

    // -------------------------------------------------------------------------
    // Status parity tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_status_parity_200_404_fires() {
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![], 200);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![], 404);
        let result = hygiene_issues(&old_b, &new_b, "desktop", &profile());
        assert!(result.short_circuit, "should short-circuit");
        assert_eq!(result.issues.len(), 1);
        assert_eq!(result.issues[0].issue_type, IssueType::StatusCodeMismatch);
        assert_eq!(result.issues[0].severity, IssueSeverity::Critical);
    }

    #[test]
    fn test_status_parity_404_404_no_issue() {
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![], 404);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![], 404);
        let result = hygiene_issues(&old_b, &new_b, "desktop", &profile());
        assert!(!result.short_circuit, "should not short-circuit");
        assert!(result
            .issues
            .iter()
            .all(|i| i.issue_type != IssueType::StatusCodeMismatch));
    }

    #[test]
    fn test_status_parity_404_200_no_issue() {
        // old non-2xx, new 2xx → improvement, no issue
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![], 404);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![], 200);
        let result = hygiene_issues(&old_b, &new_b, "desktop", &profile());
        assert!(!result.short_circuit);
        assert!(result
            .issues
            .iter()
            .all(|i| i.issue_type != IssueType::StatusCodeMismatch));
    }

    #[test]
    fn test_status_parity_status_zero_counts_non_2xx() {
        // status 0 counts as non-2xx
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![], 200);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![], 0);
        let result = hygiene_issues(&old_b, &new_b, "desktop", &profile());
        assert!(result.short_circuit, "status 0 should count as non-2xx");
    }

    #[test]
    fn test_status_parity_critical_under_both_profiles() {
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![], 200);
        let new_b = make_bundle("http://new.com/", "http://new.com/", vec![], 404);

        let p1 = ParityProfile::ContentStructure;
        let r1 = hygiene_issues(&old_b, &new_b, "desktop", &p1);
        assert_eq!(r1.issues[0].severity, IssueSeverity::Critical);

        let p2 = ParityProfile::StrictVisual;
        let r2 = hygiene_issues(&old_b, &new_b, "desktop", &p2);
        assert_eq!(r2.issues[0].severity, IssueSeverity::Critical);
    }

    // -------------------------------------------------------------------------
    // Protocol downgrade tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_protocol_downgrade_fires() {
        let old_b = make_bundle("https://example.com/", "https://example.com/", vec![], 200);
        let new_b = make_bundle("http://example.com/", "http://example.com/", vec![], 200);
        let result = hygiene_issues(&old_b, &new_b, "desktop", &profile());
        assert!(result
            .issues
            .iter()
            .any(|i| i.issue_type == IssueType::UrlProtocolDowngrade));
    }

    #[test]
    fn test_protocol_downgrade_no_false_positive_both_http() {
        // Both http → no downgrade issue
        let old_b = make_bundle("http://example.com/", "http://example.com/", vec![], 200);
        let new_b = make_bundle("http://example.com/", "http://example.com/", vec![], 200);
        let result = hygiene_issues(&old_b, &new_b, "desktop", &profile());
        assert!(result
            .issues
            .iter()
            .all(|i| i.issue_type != IssueType::UrlProtocolDowngrade));
    }

    // -------------------------------------------------------------------------
    // Canonical mismatch tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_canonical_mismatch_fires() {
        let mut old_b = make_bundle(
            "https://example.com/about",
            "https://example.com/about",
            vec![],
            200,
        );
        let mut new_b = make_bundle(
            "https://example.com/about",
            "https://example.com/about",
            vec![],
            200,
        );
        // Different canonicals (not byte-equal)
        old_b.page.canonical = Some("https://old.example.com/about".to_string());
        new_b.page.canonical = Some("https://different.example.com/about".to_string());
        let result = hygiene_issues(&old_b, &new_b, "desktop", &profile());
        assert!(result
            .issues
            .iter()
            .any(|i| i.issue_type == IssueType::CanonicalMismatch));
    }

    #[test]
    fn test_canonical_raw_parity_suppression() {
        // Same canonical on both sides → suppress
        let mut old_b = make_bundle(
            "https://example.com/about",
            "https://example.com/about",
            vec![],
            200,
        );
        let mut new_b = make_bundle(
            "https://example.com/about",
            "https://example.com/about",
            vec![],
            200,
        );
        old_b.page.canonical = Some("branded-call.html".to_string());
        new_b.page.canonical = Some("branded-call.html".to_string());
        let result = hygiene_issues(&old_b, &new_b, "desktop", &profile());
        assert!(result
            .issues
            .iter()
            .all(|i| i.issue_type != IssueType::CanonicalMismatch));
    }

    #[test]
    fn test_canonical_absolute_match_no_issue() {
        // canonical == finalUrl → no issue
        let mut old_b = make_bundle(
            "https://example.com/about",
            "https://example.com/about",
            vec![],
            200,
        );
        let mut new_b = make_bundle(
            "https://example.com/about",
            "https://example.com/about",
            vec![],
            200,
        );
        old_b.page.canonical = Some("https://example.com/about2".to_string());
        new_b.page.canonical = Some("https://example.com/about".to_string());
        let result = hygiene_issues(&old_b, &new_b, "desktop", &profile());
        assert!(result
            .issues
            .iter()
            .all(|i| i.issue_type != IssueType::CanonicalMismatch));
    }

    #[test]
    fn test_canonical_relative() {
        // relative canonical resolved against finalUrl
        let mut old_b = make_bundle(
            "https://example.com/about",
            "https://example.com/about",
            vec![],
            200,
        );
        let mut new_b = make_bundle(
            "https://example.com/about",
            "https://example.com/about",
            vec![],
            200,
        );
        old_b.page.canonical = Some("other.html".to_string());
        // "other.html" resolves to https://example.com/other.html != https://example.com/about
        new_b.page.canonical = Some("other.html".to_string());
        // Same raw → suppressed
        let result = hygiene_issues(&old_b, &new_b, "desktop", &profile());
        assert!(result
            .issues
            .iter()
            .all(|i| i.issue_type != IssueType::CanonicalMismatch));
    }

    #[test]
    fn test_canonical_trailing_slash_policy_normalization() {
        // canonical with trailing slash, finalUrl without → under Never policy both are
        // stripped so they should match
        let mut old_b = make_bundle(
            "https://example.com/about",
            "https://example.com/about",
            vec![],
            200,
        );
        let mut new_b = make_bundle(
            "https://example.com/about",
            "https://example.com/about",
            vec![],
            200,
        );
        old_b.page.canonical = Some("other-src".to_string());
        // canonical = "https://example.com/about/" and finalUrl = "https://example.com/about"
        // Under Never, both normalize to "https://example.com/about" → no issue
        new_b.page.canonical = Some("https://example.com/about/".to_string());
        let result = hygiene_issues(&old_b, &new_b, "desktop", &profile());
        assert!(result
            .issues
            .iter()
            .all(|i| i.issue_type != IssueType::CanonicalMismatch));
    }

    // -------------------------------------------------------------------------
    // Per-link parity suppression test
    // -------------------------------------------------------------------------

    #[test]
    fn test_per_link_parity_suppression_identical_bundles() {
        // When old and new have the same link with a trailing slash,
        // parity suppression should keep the issue list clean.
        let mut old_b = make_bundle("http://example.com/", "http://example.com/", vec![], 200);
        let mut new_b = make_bundle("http://example.com/", "http://example.com/", vec![], 200);

        let link_node = SemanticNode {
            id: "node_0".to_string(),
            kind: "link".to_string(),
            role: Some("link".to_string()),
            text: Some("Products".to_string()),
            acc_name: Some("Products".to_string()),
            href: Some("http://example.com/products/".to_string()),
            image_alt: None,
            bbox: [0, 0, 100, 30],
            seq_index: 0,
            anchors: crate::contract::NodeAnchors {
                text: Some("Products".to_string()),
                role: Some("link".to_string()),
                href: Some("http://example.com/products/".to_string()),
                alt: None,
                aria_label: None,
                nearest_heading: None,
                landmark: None,
                ordinal_in_landmark: None,
            },
            css_selector: None,
            ..Default::default()
        };

        old_b.page.nodes = vec![link_node.clone()];
        new_b.page.nodes = vec![link_node];

        let result = hygiene_issues(&old_b, &new_b, "desktop", &profile());
        // The trailing-slash link is parity-suppressed because it exists on old side too
        let slash_issues: Vec<_> = result
            .issues
            .iter()
            .filter(|i| i.issue_type == IssueType::UrlTrailingSlash)
            .collect();
        assert!(
            slash_issues.is_empty(),
            "identical link should be parity-suppressed"
        );
    }

    // -------------------------------------------------------------------------
    // Locale path tests (delegated to locale.rs, just integration-level here)
    // -------------------------------------------------------------------------

    #[test]
    fn test_locale_separator_invalid_emits_issue() {
        let mut new_b = make_bundle(
            "http://example.com/es_MX/products",
            "http://example.com/es_MX/products",
            vec![],
            200,
        );
        new_b.page.final_url = "http://example.com/es_MX/products".to_string();
        let old_b = make_bundle("http://example.com/", "http://example.com/", vec![], 200);
        let result = hygiene_issues(&old_b, &new_b, "desktop", &profile());
        assert!(
            result
                .issues
                .iter()
                .any(|i| i.issue_type == IssueType::LocaleSeparatorInvalid),
            "should emit locale_separator_invalid"
        );
    }

    #[test]
    fn test_locale_case_invalid_emits_issue() {
        let mut new_b = make_bundle(
            "http://example.com/es-mx/products",
            "http://example.com/es-mx/products",
            vec![],
            200,
        );
        new_b.page.final_url = "http://example.com/es-mx/products".to_string();
        let old_b = make_bundle("http://example.com/", "http://example.com/", vec![], 200);
        let result = hygiene_issues(&old_b, &new_b, "desktop", &profile());
        assert!(
            result
                .issues
                .iter()
                .any(|i| i.issue_type == IssueType::LocaleCaseInvalid),
            "should emit locale_case_invalid"
        );
    }

    // -------------------------------------------------------------------------
    // Issue ID stability
    // -------------------------------------------------------------------------

    #[test]
    fn test_issue_id_stable_under_bbox_jitter() {
        // Same type + viewport + anchors.href = same ID
        let anchors1 = Anchors {
            text: None,
            role: None,
            href: Some("/products/".to_string()),
            alt: None,
            aria_label: None,
            nearest_heading: None,
            landmark: None,
            ordinal_in_landmark: None,
        };
        let id1 = compute_issue_id(&IssueType::UrlTrailingSlash, "desktop", &anchors1, None);
        let id2 = compute_issue_id(&IssueType::UrlTrailingSlash, "desktop", &anchors1, None);
        assert_eq!(id1, id2, "ID must be stable");
    }

    // -------------------------------------------------------------------------
    // Per-link redirect chain from link_probes
    // -------------------------------------------------------------------------

    #[test]
    fn test_per_link_redirect_chain_from_probes() {
        let mut new_b = make_bundle("http://example.com/", "http://example.com/", vec![], 200);

        new_b.page.link_probes = vec![LinkProbe {
            url: "http://example.com/start".to_string(),
            redirect_chain: vec![
                "http://example.com/start".to_string(),
                "http://example.com/mid".to_string(),
            ],
            final_url: Some("http://example.com/end".to_string()),
            status: Some(200),
            skipped: None,
            error: None,
        }];

        let old_b = make_bundle("http://example.com/", "http://example.com/", vec![], 200);
        let result = hygiene_issues(&old_b, &new_b, "desktop", &profile());
        let chain_issues: Vec<_> = result
            .issues
            .iter()
            .filter(|i| i.issue_type == IssueType::UrlRedirectChain)
            .collect();
        assert_eq!(
            chain_issues.len(),
            1,
            "should emit 1 redirect chain issue from probes"
        );
        assert_eq!(chain_issues[0].evidence["new"]["hops"], 2);
    }

    // -------------------------------------------------------------------------
    // Schema validation: DiffResult with one of each new issue type
    // -------------------------------------------------------------------------

    #[test]
    fn test_diff_result_schema_validation() {
        use crate::contract::{
            AgentSummary, Artifacts, DeterminismSummary, DiffResult, Scores, Suppressed,
            ViewportResult,
        };
        use crate::report::json::make_default_det_for_test;
        use jsonschema::JSONSchema;
        use std::collections::BTreeMap;

        // Build a minimal DiffResult with each hygiene issue type
        let old_b = make_bundle("http://old.com/", "http://old.com/", vec![], 200);
        let new_b_404 = make_bundle("http://new.com/", "http://new.com/", vec![], 404);

        let outcome = hygiene_issues(&old_b, &new_b_404, "desktop", &profile());
        let issues = outcome.issues;

        let result = DiffResult {
            schema_version: "1.0".to_string(),
            tool_version: "0.1.0".to_string(),
            run_id: "2026-01-01T00-00-00Z".to_string(),
            old_url: "http://old.com/".to_string(),
            new_url: "http://new.com/".to_string(),
            parity_profile: "content-structure".to_string(),
            status: crate::contract::Status::Fail,
            agent_summary: AgentSummary {
                fixable_now: 0,
                by_type: BTreeMap::new(),
                cluster_count: 0,
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
            suppressed: Suppressed {
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
            panic!("DiffResult failed schema validation:\n{}", msgs.join("\n"));
        }
    }

    // -------------------------------------------------------------------------
    // Per-link protocol downgrade test
    // -------------------------------------------------------------------------

    #[test]
    fn test_per_link_protocol_downgrade_fires() {
        // New page has an http link; old page has the same path but https → issue fires
        let mut old_b = make_bundle("https://example.com/", "https://example.com/", vec![], 200);
        let mut new_b = make_bundle("https://example.com/", "https://example.com/", vec![], 200);

        let old_node = SemanticNode {
            id: "node_0".to_string(),
            kind: "link".to_string(),
            role: Some("link".to_string()),
            text: Some("About".to_string()),
            acc_name: Some("About".to_string()),
            href: Some("https://example.com/about".to_string()),
            image_alt: None,
            bbox: [0, 0, 100, 30],
            seq_index: 0,
            anchors: crate::contract::NodeAnchors {
                text: Some("About".to_string()),
                role: Some("link".to_string()),
                href: Some("https://example.com/about".to_string()),
                alt: None,
                aria_label: None,
                nearest_heading: None,
                landmark: None,
                ordinal_in_landmark: None,
            },
            css_selector: None,
            ..Default::default()
        };

        let new_node = SemanticNode {
            id: "node_0".to_string(),
            kind: "link".to_string(),
            role: Some("link".to_string()),
            text: Some("About".to_string()),
            acc_name: Some("About".to_string()),
            href: Some("http://example.com/about".to_string()), // http instead of https
            image_alt: None,
            bbox: [0, 0, 100, 30],
            seq_index: 0,
            anchors: crate::contract::NodeAnchors {
                text: Some("About".to_string()),
                role: Some("link".to_string()),
                href: Some("http://example.com/about".to_string()),
                alt: None,
                aria_label: None,
                nearest_heading: None,
                landmark: None,
                ordinal_in_landmark: None,
            },
            css_selector: None,
            ..Default::default()
        };

        old_b.page.nodes = vec![old_node];
        new_b.page.nodes = vec![new_node];

        let result = hygiene_issues(&old_b, &new_b, "desktop", &profile());
        let downgrade: Vec<_> = result
            .issues
            .iter()
            .filter(|i| i.issue_type == IssueType::UrlProtocolDowngrade)
            .collect();
        // Both page-level (https → https, same host, no downgrade) and per-link
        // Page level: new final is https, old final is https → no page downgrade
        // Per-link: new link is http, old link same path was https → fires
        assert!(
            downgrade.iter().any(|i| i.evidence["new"]["url"]
                .as_str()
                .unwrap_or("")
                .starts_with("http://example.com/about")),
            "per-link downgrade should fire"
        );
    }

    #[test]
    fn test_per_link_protocol_downgrade_no_false_positive_both_http() {
        // Old page also has http link → no downgrade issue
        let mut old_b = make_bundle("https://example.com/", "https://example.com/", vec![], 200);
        let mut new_b = make_bundle("https://example.com/", "https://example.com/", vec![], 200);

        let make_link = |href: &str| SemanticNode {
            id: "node_0".to_string(),
            kind: "link".to_string(),
            role: Some("link".to_string()),
            text: Some("About".to_string()),
            acc_name: Some("About".to_string()),
            href: Some(href.to_string()),
            image_alt: None,
            bbox: [0, 0, 100, 30],
            seq_index: 0,
            anchors: crate::contract::NodeAnchors {
                text: Some("About".to_string()),
                role: Some("link".to_string()),
                href: Some(href.to_string()),
                alt: None,
                aria_label: None,
                nearest_heading: None,
                landmark: None,
                ordinal_in_landmark: None,
            },
            css_selector: None,
            ..Default::default()
        };

        old_b.page.nodes = vec![make_link("http://example.com/about")];
        new_b.page.nodes = vec![make_link("http://example.com/about")];

        let result = hygiene_issues(&old_b, &new_b, "desktop", &profile());
        let downgrade: Vec<_> = result
            .issues
            .iter()
            .filter(|i| i.issue_type == IssueType::UrlProtocolDowngrade)
            .collect();
        assert!(
            downgrade.is_empty(),
            "no downgrade when old link is also http"
        );
    }
}
