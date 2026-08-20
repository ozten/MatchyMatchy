//! A11y diff (M7.md §3).
//!
//! Emits `accessibility_regression` (rule in new_rules \ old_rules) and
//! `accessibility_improved` (rule in old_rules \ new_rules) by diffing axe-core violation
//! rule-id sets. Rule-level granularity bounds blast radius on stable pages.
//!
//! DETERMINISM: BTreeSet for rule sets; emit sorted by rule id; no HashMap.

use std::collections::{BTreeMap, BTreeSet};

use crate::config::base_confidence;
use crate::contract::{
    Anchors, CaptureBundle, Issue, IssueCategory, IssueType, Locator, SemanticNode,
};
use crate::issue::compute_issue_id;
use crate::scoring::{compute_confidence, SeverityResolver};

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

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

/// Build a null locator for page-level issues.
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

/// Extract the first `target[0]` CSS selector from a violation node Value, if present.
fn first_target(node: &serde_json::Value) -> Option<&str> {
    node.get("target")
        .and_then(|t| t.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
}

/// Cap a HTML snippet to 500 chars (UTF-8 char boundary safe).
fn cap_html(html: &str) -> &str {
    if html.len() <= 500 {
        return html;
    }
    // Walk backwards from 500 to find a char boundary.
    let mut end = 500;
    while !html.is_char_boundary(end) {
        end -= 1;
    }
    &html[..end]
}

/// Build the "nodes" evidence array (up to first 5 nodes) from a violation Value.
fn violation_nodes_evidence(violation: &serde_json::Value) -> serde_json::Value {
    let empty = vec![];
    let nodes = violation
        .get("nodes")
        .and_then(|n| n.as_array())
        .unwrap_or(&empty);
    let node_evs: Vec<serde_json::Value> = nodes
        .iter()
        .take(5)
        .map(|n| {
            let target = n.get("target").cloned().unwrap_or(serde_json::Value::Null);
            let html_raw = n.get("html").and_then(|v| v.as_str()).unwrap_or("");
            let html_snippet = cap_html(html_raw).to_string();
            serde_json::json!({
                "target": target,
                "htmlSnippet": html_snippet
            })
        })
        .collect();
    serde_json::Value::Array(node_evs)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Emit `accessibility_regression` and `accessibility_improved` issues via axe rule-set diff.
///
/// Emission order: all regressions (sorted by rule id) then all improvements (sorted by rule id).
pub fn a11y_issues(
    old_bundle: &CaptureBundle,
    new_bundle: &CaptureBundle,
    viewport: &str,
    profile: &SeverityResolver,
    env_mismatch: bool,
) -> Vec<Issue> {
    let old_det = &old_bundle.determinism;
    let new_det = &new_bundle.determinism;

    // Build rule sets and first-violation maps from each side.
    let mut old_rules: BTreeSet<String> = BTreeSet::new();
    let mut old_by_rule: BTreeMap<String, &serde_json::Value> = BTreeMap::new();
    for v in &old_bundle.page.a11y.violations {
        if let Some(id) = v.get("id").and_then(|x| x.as_str()) {
            old_rules.insert(id.to_string());
            old_by_rule.entry(id.to_string()).or_insert(v);
        }
    }

    let mut new_rules: BTreeSet<String> = BTreeSet::new();
    let mut new_by_rule: BTreeMap<String, &serde_json::Value> = BTreeMap::new();
    for v in &new_bundle.page.a11y.violations {
        if let Some(id) = v.get("id").and_then(|x| x.as_str()) {
            new_rules.insert(id.to_string());
            new_by_rule.entry(id.to_string()).or_insert(v);
        }
    }

    let mut issues: Vec<Issue> = Vec::new();

    // ------------------------------------------------------------------
    // accessibility_regression: ids in new_rules \ old_rules
    // ------------------------------------------------------------------
    let regressions: Vec<String> = new_rules.difference(&old_rules).cloned().collect();
    // new_rules and old_rules are BTreeSets, so difference() is already sorted.
    // (BTreeSet::difference yields items in ascending order of the left set.)
    for rule_id in &regressions {
        let violation = match new_by_rule.get(rule_id.as_str()) {
            Some(v) => *v,
            None => continue,
        };

        let impact = violation
            .get("impact")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let help_url = violation
            .get("helpUrl")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        // Node count
        let node_count = violation
            .get("nodes")
            .and_then(|n| n.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

        let nodes_ev = violation_nodes_evidence(violation);

        let evidence = serde_json::json!({
            "ruleId": rule_id,
            "impact": impact,
            "helpUrl": help_url,
            "old": { "present": false },
            "new": {
                "ruleId": rule_id,
                "impact": impact,
                "nodeCount": node_count,
                "nodes": nodes_ev
            }
        });

        // Anchor: try to find a new SemanticNode matching the first violating node target[0]
        let (anchors, locator) = {
            let target_sel: Option<&str> = violation
                .get("nodes")
                .and_then(|n| n.as_array())
                .and_then(|arr| arr.first())
                .and_then(|n| first_target(n));

            match target_sel.and_then(|sel| {
                new_bundle
                    .page
                    .nodes
                    .iter()
                    .find(|n| n.css_selector.as_deref() == Some(sel))
            }) {
                Some(node) => {
                    let a = node_to_anchors(node);
                    let loc = null_locator(a.clone());
                    (a, loc)
                }
                None => {
                    let a = Anchors::null();
                    let loc = null_locator(a.clone());
                    (a, loc)
                }
            }
        };

        // Remediation grep token: first target[0] or empty string
        let grep_token: String = violation
            .get("nodes")
            .and_then(|n| n.as_array())
            .and_then(|arr| arr.first())
            .and_then(|n| first_target(n))
            .unwrap_or("")
            .to_string();

        let remediation = serde_json::json!({
            "action": "fix_accessibility_violation",
            "findBy": { "grep": [grep_token] },
            "ruleId": rule_id,
            "helpUrl": help_url,
            "note": format!("axe rule {} now fails on the new page (clean on old). See helpUrl.", rule_id)
        });

        let severity = profile.severity_for(
            &IssueType::AccessibilityRegression,
            &IssueCategory::Accessibility,
        );
        let confidence = compute_confidence(base_confidence::A11Y, env_mismatch, old_det, new_det);
        let id = compute_issue_id(
            &IssueType::AccessibilityRegression,
            viewport,
            &anchors,
            None,
        );

        let impact_display = impact.as_deref().unwrap_or("unknown");
        issues.push(Issue {
            id,
            issue_type: IssueType::AccessibilityRegression,
            category: IssueCategory::Accessibility,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_bundle.page.lang.clone(),
            goal: Some("G8".to_string()),
            message: format!(
                "Accessibility regression: axe rule '{}' now fails ({})",
                rule_id, impact_display
            ),
            locator,
            evidence,
            remediation: Some(remediation),
        });
    }

    // ------------------------------------------------------------------
    // accessibility_improved: ids in old_rules \ new_rules
    // ------------------------------------------------------------------
    let improvements: Vec<String> = old_rules.difference(&new_rules).cloned().collect();
    for rule_id in &improvements {
        let violation = match old_by_rule.get(rule_id.as_str()) {
            Some(v) => *v,
            None => continue,
        };

        let impact = violation
            .get("impact")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let help_url = violation
            .get("helpUrl")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        let node_count = violation
            .get("nodes")
            .and_then(|n| n.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

        let nodes_ev = violation_nodes_evidence(violation);

        // evidence.old has the rule detail; evidence.new has { "present": false }
        let evidence = serde_json::json!({
            "ruleId": rule_id,
            "impact": impact,
            "helpUrl": help_url,
            "old": {
                "ruleId": rule_id,
                "impact": impact,
                "nodeCount": node_count,
                "nodes": nodes_ev
            },
            "new": { "present": false }
        });

        // Anchor: try to find an old SemanticNode matching the first violating node target[0]
        let (anchors, locator) = {
            let target_sel: Option<&str> = violation
                .get("nodes")
                .and_then(|n| n.as_array())
                .and_then(|arr| arr.first())
                .and_then(|n| first_target(n));

            match target_sel.and_then(|sel| {
                old_bundle
                    .page
                    .nodes
                    .iter()
                    .find(|n| n.css_selector.as_deref() == Some(sel))
            }) {
                Some(node) => {
                    let a = node_to_anchors(node);
                    let loc = null_locator(a.clone());
                    (a, loc)
                }
                None => {
                    let a = Anchors::null();
                    let loc = null_locator(a.clone());
                    (a, loc)
                }
            }
        };

        let severity = profile.severity_for(
            &IssueType::AccessibilityImproved,
            &IssueCategory::Accessibility,
        );
        let confidence = compute_confidence(base_confidence::A11Y, env_mismatch, old_det, new_det);
        let id = compute_issue_id(&IssueType::AccessibilityImproved, viewport, &anchors, None);

        issues.push(Issue {
            id,
            issue_type: IssueType::AccessibilityImproved,
            category: IssueCategory::Accessibility,
            severity,
            confidence,
            viewport: viewport.to_string(),
            locale: new_bundle.page.lang.clone(),
            goal: Some("G8".to_string()),
            message: format!(
                "Accessibility improved: axe rule '{}' no longer fails",
                rule_id
            ),
            locator,
            evidence,
            // improved → null remediation (spec §3)
            remediation: None,
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
        A11yInfo, CaptureDeterminism, Environment, NetworkInfo, PageModel, Screenshots, StepStatus,
        StyleCandidates, ViewportConfig,
    };
    use crate::scoring::ParityProfile;
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

    fn make_bundle(url: &str, violations: Vec<serde_json::Value>) -> CaptureBundle {
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
                nodes: vec![],
                landmarks: vec![],
                landmark_rects: None,
                network: NetworkInfo { requests: vec![] },
                console: vec![],
                a11y: A11yInfo { violations },
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

    fn html_has_lang_violation() -> serde_json::Value {
        serde_json::json!({
            "id": "html-has-lang",
            "impact": "serious",
            "help": "html element must have a lang attribute",
            "helpUrl": "https://dequeuniversity.com/rules/axe/4.9/html-has-lang",
            "tags": ["wcag2a"],
            "nodes": [
                {
                    "target": ["html"],
                    "html": "<html>"
                }
            ]
        })
    }

    /// old violations=[] , new violations=[html-has-lang] → one accessibility_regression,
    /// ruleId in evidence.new, severity warning, goal G8.
    #[test]
    fn test_a11y_regression_new_only() {
        let old_bundle = make_bundle("http://localhost:3000/", vec![]);
        let new_bundle = make_bundle("http://localhost:3001/", vec![html_has_lang_violation()]);

        let profile = SeverityResolver::from_profile(ParityProfile::ContentStructure);
        let issues = a11y_issues(&old_bundle, &new_bundle, "desktop", &profile, false);

        let regressions: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::AccessibilityRegression)
            .collect();

        assert_eq!(
            regressions.len(),
            1,
            "should emit one accessibility_regression"
        );
        let issue = &regressions[0];
        assert_eq!(issue.goal, Some("G8".to_string()));
        assert_eq!(issue.severity, crate::contract::IssueSeverity::Warning);

        // ruleId must be in evidence.new
        let rule_in_new = issue.evidence["new"]["ruleId"].as_str().unwrap_or("");
        assert_eq!(
            rule_in_new, "html-has-lang",
            "ruleId must be in evidence.new"
        );

        // evidence.old must be { "present": false }
        assert_eq!(
            issue.evidence["old"]["present"].as_bool(),
            Some(false),
            "evidence.old.present must be false"
        );
    }

    /// Reverse: old has html-has-lang, new=[] → one accessibility_improved, severity info,
    /// remediation null.
    #[test]
    fn test_a11y_improved_old_only() {
        let old_bundle = make_bundle("http://localhost:3000/", vec![html_has_lang_violation()]);
        let new_bundle = make_bundle("http://localhost:3001/", vec![]);

        let profile = SeverityResolver::from_profile(ParityProfile::ContentStructure);
        let issues = a11y_issues(&old_bundle, &new_bundle, "desktop", &profile, false);

        let improvements: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::AccessibilityImproved)
            .collect();

        assert_eq!(
            improvements.len(),
            1,
            "should emit one accessibility_improved"
        );
        let issue = &improvements[0];
        assert_eq!(issue.severity, crate::contract::IssueSeverity::Info);
        assert!(
            issue.remediation.is_none(),
            "improved → remediation must be null"
        );

        // ruleId must be in evidence.old
        let rule_in_old = issue.evidence["old"]["ruleId"].as_str().unwrap_or("");
        assert_eq!(
            rule_in_old, "html-has-lang",
            "ruleId must be in evidence.old"
        );

        // evidence.new must be { "present": false }
        assert_eq!(
            issue.evidence["new"]["present"].as_bool(),
            Some(false),
            "evidence.new.present must be false"
        );
    }

    /// Same rule on both old and new → nothing emitted.
    #[test]
    fn test_a11y_same_rule_both_sides_no_issue() {
        let v = html_has_lang_violation();
        let old_bundle = make_bundle("http://localhost:3000/", vec![v.clone()]);
        let new_bundle = make_bundle("http://localhost:3001/", vec![v]);

        let profile = SeverityResolver::from_profile(ParityProfile::ContentStructure);
        let issues = a11y_issues(&old_bundle, &new_bundle, "desktop", &profile, false);
        assert!(
            issues.is_empty(),
            "same rule on both sides must emit nothing"
        );
    }

    /// Determinism: same inputs twice → identical ids and order.
    #[test]
    fn test_a11y_determinism() {
        let old_bundle = make_bundle("http://localhost:3000/", vec![]);
        let new_bundle = make_bundle("http://localhost:3001/", vec![html_has_lang_violation()]);

        let profile = SeverityResolver::from_profile(ParityProfile::ContentStructure);
        let issues1 = a11y_issues(&old_bundle, &new_bundle, "desktop", &profile, false);
        let issues2 = a11y_issues(&old_bundle, &new_bundle, "desktop", &profile, false);

        assert_eq!(issues1.len(), issues2.len());
        for (a, b) in issues1.iter().zip(issues2.iter()) {
            assert_eq!(a.id, b.id, "ids must be identical on repeated calls");
            assert_eq!(a.issue_type, b.issue_type);
        }
    }
}
