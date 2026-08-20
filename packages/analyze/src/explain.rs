//! `matchy explain` — hermetic computed-style / bbox triage probe.
//!
//! Locates a node by anchor string, node id, or CSS selector across two frozen
//! `CaptureBundle`s and prints a per-side computed-style + bbox table, highlighting
//! differences.  No browser, no network, no taxonomy or scoring — surfaces only data
//! already in the bundles.

use std::collections::BTreeMap;

use crate::contract::{CaptureBundle, SemanticNode};

// ---------------------------------------------------------------------------
// Public API types
// ---------------------------------------------------------------------------

/// The three ways to point at a node.
#[derive(Debug, Clone)]
pub enum Locator {
    /// `key=value` where key ∈ {text, role, href, nearestHeading}.
    /// Substring match on the corresponding `NodeAnchors` / `SemanticNode` field.
    Anchor { key: String, value: String },
    /// Exact match on `SemanticNode.id`.
    NodeId(String),
    /// Exact match on `SemanticNode.css_selector` first; falls back to substring.
    Selector(String),
}

impl Locator {
    /// Parse `--anchor "text=Get started"` into `Locator::Anchor`.
    /// Returns `Err` if the string has no `=`.
    pub fn parse_anchor(s: &str) -> Result<Self, String> {
        let (key, value) = s
            .split_once('=')
            .ok_or_else(|| format!("invalid --anchor syntax '{}': expected key=value", s))?;
        let key = key.to_string();
        if !matches!(key.as_str(), "text" | "role" | "href" | "nearestHeading") {
            return Err(format!(
                "unknown anchor key '{}': expected text, role, href, or nearestHeading",
                key
            ));
        }
        Ok(Locator::Anchor {
            key,
            value: value.to_string(),
        })
    }
}

/// Resolution status for one side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionStatus {
    Resolved,
    NotFound,
}

/// Per-side resolution result.
#[derive(Debug, Clone)]
pub struct SideResult {
    pub status: ResolutionStatus,
    pub node_id: Option<String>,
    /// Computed-style props for the resolved node (empty if not found).
    pub computed_styles: BTreeMap<String, String>,
    /// Bbox as four synthetic props: bbox.x, bbox.y, bbox.w, bbox.h
    pub bbox: Option<[i32; 4]>,
}

/// One property row in the explain output.
#[derive(Debug, Clone)]
pub struct PropRow {
    pub property: String,
    pub old_value: String,
    pub new_value: String,
    pub changed: bool,
}

/// The structured result returned by [`explain`].
#[derive(Debug, Clone)]
pub struct ExplainReport {
    pub old: SideResult,
    pub new: SideResult,
    /// Property rows in alphabetical order.
    pub rows: Vec<PropRow>,
    /// Human-readable asymmetry description when only one side resolved.
    pub asymmetry_message: Option<String>,
}

// ---------------------------------------------------------------------------
// Core pure function
// ---------------------------------------------------------------------------

/// Locate a node by `locator` in each bundle independently, gather their
/// computed-style + bbox values, and produce a property comparison table.
///
/// `props`: when `Some`, show exactly those properties (even if absent/unchanged).
/// When `None`, show only properties that differ between the two sides (diff-only default).
///
/// Determinism: all maps are `BTreeMap`; property rows sorted alphabetically; node
/// resolution tie-breaks on (seq_index, id) to mirror `region_link`'s total order.
pub fn explain(
    old_bundle: &CaptureBundle,
    new_bundle: &CaptureBundle,
    locator: &Locator,
    props: Option<&[String]>,
) -> ExplainReport {
    let old_side = resolve_side(old_bundle, locator);
    let new_side = resolve_side(new_bundle, locator);

    // Build the property rows.
    let rows = build_rows(&old_side, &new_side, props);

    // Build asymmetry message.
    let asymmetry_message = match (&old_side.status, &new_side.status) {
        (ResolutionStatus::Resolved, ResolutionStatus::NotFound) => Some(format!(
            "node present in OLD only (id: {}) — element may have been removed in new",
            old_side.node_id.as_deref().unwrap_or("?")
        )),
        (ResolutionStatus::NotFound, ResolutionStatus::Resolved) => Some(format!(
            "node present in NEW only (id: {}) — element may have been added in new",
            new_side.node_id.as_deref().unwrap_or("?")
        )),
        _ => None,
    };

    ExplainReport {
        old: old_side,
        new: new_side,
        rows,
        asymmetry_message,
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Resolve the locator against one bundle independently.
fn resolve_side(bundle: &CaptureBundle, locator: &Locator) -> SideResult {
    let node = find_node(&bundle.page.nodes, locator);

    match node {
        None => SideResult {
            status: ResolutionStatus::NotFound,
            node_id: None,
            computed_styles: BTreeMap::new(),
            bbox: None,
        },
        Some(n) => {
            let computed_styles = bundle
                .computed_styles
                .get(&n.id)
                .cloned()
                .unwrap_or_default();
            let bbox = Some(n.bbox);
            SideResult {
                status: ResolutionStatus::Resolved,
                node_id: Some(n.id.clone()),
                computed_styles,
                bbox,
            }
        }
    }
}

/// Find a node in `nodes` matching the locator.
/// When multiple nodes match, pick the one with the lowest (seq_index, id) — total order.
fn find_node<'a>(nodes: &'a [SemanticNode], locator: &Locator) -> Option<&'a SemanticNode> {
    let mut candidates: Vec<&SemanticNode> =
        nodes.iter().filter(|n| node_matches(n, locator)).collect();

    if candidates.is_empty() {
        return None;
    }

    // Total-order tie-break: (seq_index ASC, id ASC)
    candidates.sort_by(|a, b| a.seq_index.cmp(&b.seq_index).then_with(|| a.id.cmp(&b.id)));
    candidates.into_iter().next()
}

/// Return true if `node` matches the locator.
fn node_matches(node: &SemanticNode, locator: &Locator) -> bool {
    match locator {
        Locator::NodeId(id) => &node.id == id,

        Locator::Selector(sel) => {
            // Exact match first; fall back to substring.
            if let Some(css) = &node.css_selector {
                if css == sel {
                    return true;
                }
                // Substring fallback
                css.contains(sel.as_str())
            } else {
                false
            }
        }

        Locator::Anchor { key, value } => match key.as_str() {
            "text" => {
                // Check anchors.text first, then node.text
                let anchors_text = node.anchors.text.as_deref().unwrap_or("");
                let node_text = node.text.as_deref().unwrap_or("");
                anchors_text.contains(value.as_str()) || node_text.contains(value.as_str())
            }
            "role" => {
                let anchors_role = node.anchors.role.as_deref().unwrap_or("");
                let node_role = node.role.as_deref().unwrap_or("");
                anchors_role.contains(value.as_str()) || node_role.contains(value.as_str())
            }
            "href" => {
                let anchors_href = node.anchors.href.as_deref().unwrap_or("");
                let node_href = node.href.as_deref().unwrap_or("");
                anchors_href.contains(value.as_str()) || node_href.contains(value.as_str())
            }
            "nearestHeading" => {
                let heading = node.anchors.nearest_heading.as_deref().unwrap_or("");
                heading.contains(value.as_str())
            }
            _ => false,
        },
    }
}

/// Build the sorted property rows from the two resolved sides.
fn build_rows(
    old_side: &SideResult,
    new_side: &SideResult,
    props: Option<&[String]>,
) -> Vec<PropRow> {
    const ABSENT: &str = "<absent>";

    // Helper: get a value from a SideResult's computed_styles, or bbox pseudo-props.
    let get_val = |side: &SideResult, prop: &str| -> String {
        // Handle bbox pseudo-props first.
        if let Some(bbox) = side.bbox {
            match prop {
                "bbox.x" => return bbox[0].to_string(),
                "bbox.y" => return bbox[1].to_string(),
                "bbox.w" => return bbox[2].to_string(),
                "bbox.h" => return bbox[3].to_string(),
                _ => {}
            }
        }
        side.computed_styles
            .get(prop)
            .cloned()
            .unwrap_or_else(|| ABSENT.to_string())
    };

    // Build the key set to show.
    let keys: Vec<String> = if let Some(explicit) = props {
        // Explicit --props: always show these in order (sorted for determinism).
        let mut sorted = explicit.to_vec();
        sorted.sort();
        sorted
    } else {
        // Diff-only default: union of both sides' computed-style keys + bbox pseudo-props,
        // filtered to those that differ.
        let mut all_keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

        // Add bbox pseudo-props.
        for k in &["bbox.x", "bbox.y", "bbox.w", "bbox.h"] {
            all_keys.insert(k.to_string());
        }

        // Add computed-style keys from both sides.
        for k in old_side.computed_styles.keys() {
            all_keys.insert(k.clone());
        }
        for k in new_side.computed_styles.keys() {
            all_keys.insert(k.clone());
        }

        // Keep only differing keys.
        all_keys
            .into_iter()
            .filter(|k| get_val(old_side, k) != get_val(new_side, k))
            .collect()
    };

    keys.into_iter()
        .map(|prop| {
            let old_value = get_val(old_side, &prop);
            let new_value = get_val(new_side, &prop);
            let changed = old_value != new_value;
            PropRow {
                property: prop,
                old_value,
                new_value,
                changed,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Formatting helpers (used by the CLI handler in matchy.rs)
// ---------------------------------------------------------------------------

/// Format the `ExplainReport` as a human-readable table string.
///
/// Output is byte-deterministic (properties sorted alphabetically, BTreeMap iteration).
pub fn format_report(report: &ExplainReport, locator_str: &str) -> String {
    let mut out = String::new();

    // Header line
    out.push_str(&format!("matchy explain — locator: {}\n", locator_str));
    out.push('\n');

    // Per-side resolution summary
    match (&report.old.status, &report.new.status) {
        (ResolutionStatus::Resolved, ResolutionStatus::Resolved) => {
            out.push_str(&format!(
                "old: resolved  node={}\n",
                report.old.node_id.as_deref().unwrap_or("?")
            ));
            out.push_str(&format!(
                "new: resolved  node={}\n",
                report.new.node_id.as_deref().unwrap_or("?")
            ));
        }
        (ResolutionStatus::Resolved, ResolutionStatus::NotFound) => {
            out.push_str(&format!(
                "old: resolved  node={}\n",
                report.old.node_id.as_deref().unwrap_or("?")
            ));
            out.push_str("new: NOT FOUND\n");
        }
        (ResolutionStatus::NotFound, ResolutionStatus::Resolved) => {
            out.push_str("old: NOT FOUND\n");
            out.push_str(&format!(
                "new: resolved  node={}\n",
                report.new.node_id.as_deref().unwrap_or("?")
            ));
        }
        (ResolutionStatus::NotFound, ResolutionStatus::NotFound) => {
            out.push_str("old: NOT FOUND\n");
            out.push_str("new: NOT FOUND\n");
        }
    }

    if let Some(msg) = &report.asymmetry_message {
        out.push('\n');
        out.push_str(&format!("NOTE: {}\n", msg));
    }

    out.push('\n');

    if report.rows.is_empty() {
        out.push_str("(no properties to display)\n");
        return out;
    }

    // Compute column widths for alignment.
    let prop_w = report
        .rows
        .iter()
        .map(|r| r.property.len())
        .max()
        .unwrap_or(8)
        .max(8); // "property"
    let old_w = report
        .rows
        .iter()
        .map(|r| r.old_value.len())
        .max()
        .unwrap_or(3)
        .max(3); // "old"
    let new_w = report
        .rows
        .iter()
        .map(|r| r.new_value.len())
        .max()
        .unwrap_or(3)
        .max(3); // "new"

    // Header row
    out.push_str(&format!(
        "{:<prop_w$}  {:<old_w$}  {:<new_w$}  {}\n",
        "property",
        "old",
        "new",
        "changed?",
        prop_w = prop_w,
        old_w = old_w,
        new_w = new_w
    ));
    // Separator
    out.push_str(&format!(
        "{:-<prop_w$}  {:-<old_w$}  {:-<new_w$}  --------\n",
        "",
        "",
        "",
        prop_w = prop_w,
        old_w = old_w,
        new_w = new_w
    ));

    for row in &report.rows {
        let changed_str = if row.changed { "CHANGED" } else { "" };
        out.push_str(&format!(
            "{:<prop_w$}  {:<old_w$}  {:<new_w$}  {}\n",
            row.property,
            row.old_value,
            row.new_value,
            changed_str,
            prop_w = prop_w,
            old_w = old_w,
            new_w = new_w
        ));
    }

    out
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        A11yInfo, CaptureDeterminism, Environment, NetworkInfo, NodeAnchors, PageModel,
        Screenshots, SemanticNode, StepStatus, StyleCandidates, ViewportConfig,
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

    fn make_node(
        id: &str,
        text: Option<&str>,
        role: Option<&str>,
        href: Option<&str>,
        css_selector: Option<&str>,
        nearest_heading: Option<&str>,
        bbox: [i32; 4],
        seq_index: u32,
    ) -> SemanticNode {
        SemanticNode {
            id: id.to_string(),
            kind: "button".to_string(),
            role: role.map(str::to_string),
            text: text.map(str::to_string),
            acc_name: None,
            href: href.map(str::to_string),
            image_alt: None,
            bbox,
            seq_index,
            anchors: NodeAnchors {
                text: text.map(str::to_string),
                role: role.map(str::to_string),
                href: href.map(str::to_string),
                alt: None,
                aria_label: None,
                nearest_heading: nearest_heading.map(str::to_string),
                landmark: None,
                ordinal_in_landmark: None,
            },
            css_selector: css_selector.map(str::to_string),
            raw_href: None,
            src: None,
            natural_width: None,
            natural_height: None,
            loaded: None,
            heading_level: None,
        }
    }

    fn make_bundle(
        url: &str,
        nodes: Vec<SemanticNode>,
        computed_styles: BTreeMap<String, BTreeMap<String, String>>,
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
                network: NetworkInfo { requests: vec![] },
                console: vec![],
                a11y: A11yInfo { violations: vec![] },
                link_probes: vec![],
            },
            computed_styles,
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

    // -------------------------------------------------------------------------
    // Helper: Build two bundles with a CTA node that has a style change
    // -------------------------------------------------------------------------
    fn make_cta_bundles() -> (CaptureBundle, CaptureBundle) {
        let cta_old = make_node(
            "node_cta_old",
            Some("Get started"),
            Some("button"),
            None,
            Some(".hero .cta"),
            Some("Hero"),
            [100, 200, 150, 50],
            3,
        );
        let cta_new = make_node(
            "node_cta_new",
            Some("Get started"),
            Some("button"),
            None,
            Some(".hero .cta"),
            Some("Hero"),
            [100, 200, 150, 50],
            3,
        );
        let other_old = make_node(
            "node_other",
            Some("Sign in"),
            None,
            None,
            None,
            None,
            [0, 0, 100, 40],
            0,
        );

        let mut old_styles: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut cta_old_style: BTreeMap<String, String> = BTreeMap::new();
        cta_old_style.insert(
            "background-image".to_string(),
            "linear-gradient(90deg, #0070f3, #00c6ff)".to_string(),
        );
        cta_old_style.insert("color".to_string(), "#ffffff".to_string());
        cta_old_style.insert("font-family".to_string(), "Inter, sans-serif".to_string());
        old_styles.insert("node_cta_old".to_string(), cta_old_style);

        let mut new_styles: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut cta_new_style: BTreeMap<String, String> = BTreeMap::new();
        cta_new_style.insert("background-image".to_string(), "none".to_string());
        cta_new_style.insert("color".to_string(), "#ffffff".to_string()); // unchanged
        cta_new_style.insert("font-family".to_string(), "Inter, sans-serif".to_string()); // unchanged
        new_styles.insert("node_cta_new".to_string(), cta_new_style);

        let old_bundle = make_bundle(
            "http://old.example.com/",
            vec![other_old, cta_old],
            old_styles,
        );
        let new_bundle = make_bundle("http://new.example.com/", vec![cta_new], new_styles);

        (old_bundle, new_bundle)
    }

    // -------------------------------------------------------------------------
    // Test A1: anchor text=Get started resolves CTA; diff-only includes changed prop,
    //          excludes unchanged props.
    // -------------------------------------------------------------------------
    #[test]
    fn test_anchor_text_diff_only() {
        let (old_bundle, new_bundle) = make_cta_bundles();
        let locator = Locator::parse_anchor("text=Get started").unwrap();
        let report = explain(&old_bundle, &new_bundle, &locator, None);

        // Both sides resolved
        assert_eq!(report.old.status, ResolutionStatus::Resolved);
        assert_eq!(report.new.status, ResolutionStatus::Resolved);
        assert_eq!(report.old.node_id.as_deref(), Some("node_cta_old"));
        assert_eq!(report.new.node_id.as_deref(), Some("node_cta_new"));

        // background-image changed: must appear
        let bg_row = report
            .rows
            .iter()
            .find(|r| r.property == "background-image")
            .expect("background-image must be in rows");
        assert!(bg_row.changed, "background-image must be marked changed");
        assert!(
            bg_row.old_value.contains("gradient"),
            "old background-image must contain gradient"
        );
        assert_eq!(bg_row.new_value, "none");

        // color and font-family are unchanged — must NOT appear in diff-only output
        let unchanged_props: Vec<&str> = report
            .rows
            .iter()
            .filter(|r| !r.changed)
            .map(|r| r.property.as_str())
            .collect();
        assert!(
            !unchanged_props.contains(&"color"),
            "color must not appear in diff-only output"
        );
        assert!(
            !unchanged_props.contains(&"font-family"),
            "font-family must not appear in diff-only output"
        );

        // All rows in diff-only mode must be changed
        for row in &report.rows {
            // bbox props may differ too (different nodes) - that's expected
            assert!(
                row.changed,
                "diff-only must only include changed rows, got unchanged: {}",
                row.property
            );
        }
    }

    // -------------------------------------------------------------------------
    // Test A2: --props color,font-family restricts output to exactly those props,
    //          even when both are unchanged.
    // -------------------------------------------------------------------------
    #[test]
    fn test_explicit_props_shows_unchanged() {
        let (old_bundle, new_bundle) = make_cta_bundles();
        let locator = Locator::parse_anchor("text=Get started").unwrap();
        let props = vec!["color".to_string(), "font-family".to_string()];
        let report = explain(&old_bundle, &new_bundle, &locator, Some(&props));

        // Both sides resolved
        assert_eq!(report.old.status, ResolutionStatus::Resolved);
        assert_eq!(report.new.status, ResolutionStatus::Resolved);

        // Exactly 2 rows: color and font-family
        assert_eq!(
            report.rows.len(),
            2,
            "must have exactly 2 rows for explicit props"
        );
        let prop_names: Vec<&str> = report.rows.iter().map(|r| r.property.as_str()).collect();
        assert!(prop_names.contains(&"color"), "color must be present");
        assert!(
            prop_names.contains(&"font-family"),
            "font-family must be present"
        );

        // background-image must NOT appear (not in --props)
        assert!(
            !prop_names.contains(&"background-image"),
            "background-image must not appear when not in --props"
        );

        // Both must be marked NOT changed
        for row in &report.rows {
            assert!(
                !row.changed,
                "color/font-family must not be changed; got changed=true for {}",
                row.property
            );
        }
    }

    // -------------------------------------------------------------------------
    // Test A3: --node <id> resolves the same CTA node as the anchor.
    // -------------------------------------------------------------------------
    #[test]
    fn test_node_id_resolves_cta() {
        let (old_bundle, new_bundle) = make_cta_bundles();
        let locator = Locator::NodeId("node_cta_old".to_string());
        let report = explain(&old_bundle, &new_bundle, &locator, None);

        // Old side resolves to the CTA node
        assert_eq!(report.old.status, ResolutionStatus::Resolved);
        assert_eq!(report.old.node_id.as_deref(), Some("node_cta_old"));

        // New side: node_cta_new has a different id — not found by this locator
        // That is expected: --node uses exact id match
        assert_eq!(report.new.status, ResolutionStatus::NotFound);
    }

    // -------------------------------------------------------------------------
    // Test A4: --selector ".hero .cta" resolves the CTA node on both sides (both have that css_selector).
    // -------------------------------------------------------------------------
    #[test]
    fn test_selector_resolves_cta() {
        let (old_bundle, new_bundle) = make_cta_bundles();
        let locator = Locator::Selector(".hero .cta".to_string());
        let report = explain(&old_bundle, &new_bundle, &locator, None);

        // Both sides have css_selector = ".hero .cta"
        assert_eq!(report.old.status, ResolutionStatus::Resolved);
        assert_eq!(report.new.status, ResolutionStatus::Resolved);
        assert_eq!(report.old.node_id.as_deref(), Some("node_cta_old"));
        assert_eq!(report.new.node_id.as_deref(), Some("node_cta_new"));
    }

    // -------------------------------------------------------------------------
    // Test A5: node present in new only → reports asymmetry, no panic.
    // -------------------------------------------------------------------------
    #[test]
    fn test_one_side_only_no_panic() {
        let (old_bundle, new_bundle) = make_cta_bundles();
        // Use a text that only exists in the new bundle (new has "Get started",
        // old also has it — so use a node_id that only exists in new).
        let locator = Locator::NodeId("node_cta_new".to_string());
        let report = explain(&old_bundle, &new_bundle, &locator, None);

        // Old side should NOT resolve (no node with id=node_cta_new in old)
        assert_eq!(report.old.status, ResolutionStatus::NotFound);
        // New side resolves
        assert_eq!(report.new.status, ResolutionStatus::Resolved);
        assert_eq!(report.new.node_id.as_deref(), Some("node_cta_new"));

        // Asymmetry message must be present
        assert!(report.asymmetry_message.is_some());
        let msg = report.asymmetry_message.as_deref().unwrap();
        assert!(
            msg.contains("NEW only"),
            "asymmetry message must mention NEW only"
        );
    }

    // -------------------------------------------------------------------------
    // Test A6: locator matching no nodes → both sides NotFound, no panic.
    // -------------------------------------------------------------------------
    #[test]
    fn test_not_found_on_both_sides() {
        let (old_bundle, new_bundle) = make_cta_bundles();
        let locator = Locator::parse_anchor("text=DOES_NOT_EXIST_ANYWHERE").unwrap();
        let report = explain(&old_bundle, &new_bundle, &locator, None);

        assert_eq!(report.old.status, ResolutionStatus::NotFound);
        assert_eq!(report.new.status, ResolutionStatus::NotFound);
        assert!(report.asymmetry_message.is_none());
        assert!(report.rows.is_empty());
    }

    // -------------------------------------------------------------------------
    // Test A7: anchor key validation.
    // -------------------------------------------------------------------------
    #[test]
    fn test_anchor_key_validation() {
        assert!(Locator::parse_anchor("text=foo").is_ok());
        assert!(Locator::parse_anchor("role=button").is_ok());
        assert!(Locator::parse_anchor("href=/about").is_ok());
        assert!(Locator::parse_anchor("nearestHeading=Hero").is_ok());
        assert!(Locator::parse_anchor("badkey=foo").is_err());
        assert!(Locator::parse_anchor("no-equals").is_err());
    }

    // -------------------------------------------------------------------------
    // Test A8: tie-break determinism — multiple matches sorted by (seq_index, id).
    // -------------------------------------------------------------------------
    #[test]
    fn test_tie_break_determinism() {
        // Two nodes both with text="Get started"; lower seq_index wins.
        let node_a = make_node(
            "node_b", // id sorts after node_a but seq_index is higher
            Some("Get started"),
            None,
            None,
            None,
            None,
            [0, 300, 100, 40],
            5,
        );
        let node_b = make_node(
            "node_a", // id sorts first
            Some("Get started"),
            None,
            None,
            None,
            None,
            [0, 100, 100, 40],
            2, // lower seq_index → wins
        );
        let bundle = make_bundle("http://example.com/", vec![node_a, node_b], BTreeMap::new());

        let locator = Locator::parse_anchor("text=Get started").unwrap();
        let side = resolve_side(&bundle, &locator);

        assert_eq!(side.status, ResolutionStatus::Resolved);
        // node_b has seq_index=2 < node_a's seq_index=5, so node_b (id=node_a) wins
        assert_eq!(side.node_id.as_deref(), Some("node_a"));
    }

    // -------------------------------------------------------------------------
    // Test A9: format_report output is byte-deterministic and contains expected content.
    // -------------------------------------------------------------------------
    #[test]
    fn test_format_report_deterministic() {
        let (old_bundle, new_bundle) = make_cta_bundles();
        let locator = Locator::parse_anchor("text=Get started").unwrap();
        let report = explain(&old_bundle, &new_bundle, &locator, None);

        let out1 = format_report(&report, "text=Get started");
        let out2 = format_report(&report, "text=Get started");
        assert_eq!(out1, out2, "format_report must be byte-deterministic");

        // Must contain the column headers
        assert!(out1.contains("property"), "must contain 'property' header");
        assert!(out1.contains("old"), "must contain 'old' header");
        assert!(out1.contains("new"), "must contain 'new' header");
        assert!(out1.contains("changed?"), "must contain 'changed?' header");

        // Must mark the background-image row as CHANGED
        assert!(out1.contains("CHANGED"), "must mark changed rows");
        assert!(
            out1.contains("background-image"),
            "must show background-image"
        );
    }
}
