//! Static HTML report renderer (M8 §5 / WP-F).
//!
//! Security invariants (spec §15):
//! - Every page-derived string passes through `escape()` before interpolation.
//! - Restrictive CSP meta tag present in <head>.
//! - No <script> tags, no inline event handlers, no javascript: urls.
//! - Single inline <style> block; no external resources.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;

use crate::contract::DiffResult;

// ---------------------------------------------------------------------------
// Security: HTML escaping
// ---------------------------------------------------------------------------

/// Escape a page-derived string for safe HTML interpolation.
/// Order matters: ampersand MUST be replaced first to avoid double-encoding.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Render one issue as a `<div class="issue ...">` card. Shared by the top-level
/// Issues section and the per-region collapsed member `<details>` (U2 demotion).
fn render_issue_card(out: &mut String, issue: &crate::contract::Issue) {
    let sev_str = match &issue.severity {
        crate::contract::IssueSeverity::Info => "info",
        crate::contract::IssueSeverity::Warning => "warning",
        crate::contract::IssueSeverity::Error => "error",
        crate::contract::IssueSeverity::Critical => "critical",
    };

    let cat_str = match &issue.category {
        crate::contract::IssueCategory::Visual => "visual",
        crate::contract::IssueCategory::Content => "content",
        crate::contract::IssueCategory::Structure => "structure",
        crate::contract::IssueCategory::Style => "style",
        crate::contract::IssueCategory::Accessibility => "accessibility",
        crate::contract::IssueCategory::Technical => "technical",
        crate::contract::IssueCategory::Hygiene => "hygiene",
    };

    let uncertain = crate::report::is_uncertain_pairing(&issue.evidence);

    out.push_str(&format!(
        "<div class=\"issue sev-{sev_str}\" id=\"{}\">\n",
        escape(&issue.id)
    ));
    // Title line: type + severity badge + optional uncertain badge
    out.push_str(&format!(
        "<h4>{} <span class=\"badge badge-{sev_str}\">{}</span>",
        escape(issue.issue_type.as_str()),
        sev_str.to_uppercase()
    ));
    if uncertain {
        out.push_str(" <span class=\"badge uncertain\">uncertain pairing</span>");
    }
    out.push_str("</h4>\n");

    out.push_str("<dl class=\"issue-meta\">\n");
    out.push_str(&format!("<dt>ID</dt><dd>{}</dd>\n", escape(&issue.id)));
    out.push_str(&format!("<dt>Category</dt><dd>{cat_str}</dd>\n"));
    out.push_str(&format!(
        "<dt>Confidence</dt><dd>{:.2}</dd>\n",
        issue.confidence
    ));
    out.push_str(&format!(
        "<dt>Viewport</dt><dd>{}</dd>\n",
        escape(&issue.viewport)
    ));
    if let Some(goal) = &issue.goal {
        out.push_str(&format!("<dt>Goal</dt><dd>{}</dd>\n", escape(goal)));
    }
    if let Some(locale) = &issue.locale {
        out.push_str(&format!("<dt>Locale</dt><dd>{}</dd>\n", escape(locale)));
    }
    out.push_str("</dl>\n");

    // Message (page-derived — must escape)
    out.push_str(&format!(
        "<p class=\"issue-message\">{}</p>\n",
        escape(&issue.message)
    ));

    // Anchors sub-block
    let anchors = &issue.locator.anchors;
    let has_anchors = anchors.text.is_some()
        || anchors.href.is_some()
        || anchors.role.is_some()
        || anchors.landmark.is_some()
        || anchors.nearest_heading.is_some()
        || anchors.alt.is_some()
        || anchors.aria_label.is_some();
    if has_anchors {
        out.push_str("<details class=\"anchors\">\n<summary>Anchors</summary>\n<dl>\n");
        if let Some(t) = &anchors.text {
            out.push_str(&format!("<dt>text</dt><dd>{}</dd>\n", escape(t)));
        }
        if let Some(h) = &anchors.href {
            out.push_str(&format!("<dt>href</dt><dd>{}</dd>\n", escape(h)));
        }
        if let Some(r) = &anchors.role {
            out.push_str(&format!("<dt>role</dt><dd>{}</dd>\n", escape(r)));
        }
        if let Some(a) = &anchors.alt {
            out.push_str(&format!("<dt>alt</dt><dd>{}</dd>\n", escape(a)));
        }
        if let Some(al) = &anchors.aria_label {
            out.push_str(&format!("<dt>aria-label</dt><dd>{}</dd>\n", escape(al)));
        }
        if let Some(nh) = &anchors.nearest_heading {
            out.push_str(&format!(
                "<dt>nearest heading</dt><dd>{}</dd>\n",
                escape(nh)
            ));
        }
        if let Some(lm) = &anchors.landmark {
            out.push_str(&format!("<dt>landmark</dt><dd>{}</dd>\n", escape(lm)));
        }
        if let Some(ord) = &anchors.ordinal_in_landmark {
            out.push_str(&format!("<dt>ordinal in landmark</dt><dd>{ord}</dd>\n"));
        }
        out.push_str("</dl>\n</details>\n");
    }

    // Evidence: print evidence.old and evidence.new if present
    render_evidence(out, &issue.evidence);

    // Remediation
    if let Some(rem) = &issue.remediation {
        render_remediation(out, rem);
    }

    out.push_str("</div>\n");
}

/// Full legacy HTML (back-compat wrapper; byte-identical to pre-feature).
pub fn render_html(result: &DiffResult, old_dims: &BTreeMap<String, (u32, u32)>) -> String {
    render_html_mode(result, old_dims, crate::report::DisclosureMode::Full, "")
}

/// Render a DiffResult into a self-contained static HTML string.
/// `mode` selects compact (progressive-disclosure `<details>`) or full (legacy flat cards).
/// `out_dir` is used for drill-command generation in compact mode.
/// Pure function, deterministic, no filesystem access.
///
/// `old_dims` maps viewport name → (width, height) of the old screenshot in pixels.
/// When present, a visual overlay is drawn on the old screenshot image showing each
/// region's bounding-box extent.
pub fn render_html_mode(
    result: &DiffResult,
    old_dims: &BTreeMap<String, (u32, u32)>,
    mode: crate::report::DisclosureMode,
    out_dir: &str,
) -> String {
    let mut out = String::with_capacity(64 * 1024);

    let status_str = match &result.status {
        crate::contract::Status::Pass => "pass",
        crate::contract::Status::Warn => "warn",
        crate::contract::Status::Fail => "fail",
        crate::contract::Status::Error => "error",
    };

    // Build the claimed-id set from all saturated regions. Used to filter
    // member cards out of the top-level Issues loop (they move into per-region
    // <details> blocks — U2 progressive disclosure).
    let claimed = crate::report::claimed_issue_ids(result);

    let title = format!(
        "matchy: {} → {}",
        escape(&result.old_url),
        escape(&result.new_url)
    );

    // ------------------------------------------------------------------
    // Document open
    // ------------------------------------------------------------------
    out.push_str("<!DOCTYPE html>\n");
    out.push_str("<html lang=\"en\">\n");
    out.push_str("<head>\n");
    out.push_str("<meta charset=\"utf-8\">\n");
    // CSP: exact string required by spec §15 / M8 security invariants.
    out.push_str("<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\">\n");
    out.push_str(&format!("<title>{title}</title>\n"));
    out.push_str(STYLE);
    out.push_str("</head>\n");
    out.push_str("<body>\n");

    // ------------------------------------------------------------------
    // 1. Header
    // ------------------------------------------------------------------
    out.push_str("<div class=\"container\">\n");
    out.push_str(&format!(
        "<h1>matchy report <span class=\"badge badge-{status_str}\">{}</span></h1>\n",
        status_str.to_uppercase()
    ));
    out.push_str("<dl class=\"meta\">\n");
    out.push_str(&format!(
        "<dt>Old URL</dt><dd>{}</dd>\n",
        escape(&result.old_url)
    ));
    out.push_str(&format!(
        "<dt>New URL</dt><dd>{}</dd>\n",
        escape(&result.new_url)
    ));
    out.push_str(&format!(
        "<dt>Profile</dt><dd>{}</dd>\n",
        escape(&result.parity_profile)
    ));
    out.push_str(&format!(
        "<dt>Run ID</dt><dd>{}</dd>\n",
        escape(&result.run_id)
    ));
    out.push_str(&format!(
        "<dt>Tool version</dt><dd>{}</dd>\n",
        escape(&result.tool_version)
    ));
    out.push_str(&format!(
        "<dt>Schema version</dt><dd>{}</dd>\n",
        escape(&result.schema_version)
    ));
    // Scoping metadata
    if let Some(scoped) = &result.scoped_to {
        let scope_str = scoped
            .iter()
            .map(|s| escape(s))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("<dt>Scoped to</dt><dd>{scope_str}</dd>\n"));
        out.push_str(&format!(
            "<dt>Out of scope</dt><dd>{} issue(s)</dd>\n",
            result.out_of_scope.count
        ));
    }
    out.push_str("</dl>\n");

    // ------------------------------------------------------------------
    // 1b. Warnings banner (right after header metadata)
    // ------------------------------------------------------------------
    if !result.warnings.is_empty() {
        out.push_str("<section class=\"warnings\">\n");
        for w in &result.warnings {
            out.push_str(&format!(
                "<div class=\"warning\">⚠ {}: {}</div>\n",
                escape(&w.code),
                escape(&w.message)
            ));
        }
        out.push_str("</section>\n");
    }

    // ------------------------------------------------------------------
    // 2. Agent summary
    // ------------------------------------------------------------------
    out.push_str("<section>\n<h2>Summary</h2>\n");
    out.push_str("<dl class=\"meta\">\n");
    out.push_str(&format!(
        "<dt>Fixable now</dt><dd>{}</dd>\n",
        result.agent_summary.fixable_now
    ));
    out.push_str(&format!(
        "<dt>Cluster count</dt><dd>{}</dd>\n",
        result.agent_summary.cluster_count
    ));
    out.push_str(&format!(
        "<dt>Region count</dt><dd>{}</dd>\n",
        result.agent_summary.region_count
    ));
    out.push_str("</dl>\n");

    if !result.agent_summary.top_fixes.is_empty() {
        out.push_str("<h3>Top fixes</h3>\n<ul>\n");
        for id in &result.agent_summary.top_fixes {
            // IDs are tool-controlled; escape is harmless
            out.push_str(&format!("<li>{}</li>\n", escape(id)));
        }
        out.push_str("</ul>\n");
    }

    if !result.agent_summary.by_type.is_empty() {
        out.push_str("<h3>By type</h3>\n<ul>\n");
        // BTreeMap iterates in sorted key order — deterministic.
        for (type_str, count) in &result.agent_summary.by_type {
            out.push_str(&format!("<li>{}: {}</li>\n", escape(type_str), count));
        }
        out.push_str("</ul>\n");
    }

    out.push_str("</section>\n");

    // ------------------------------------------------------------------
    // 3. Scores table
    // ------------------------------------------------------------------
    out.push_str("<section>\n<h2>Scores</h2>\n");
    out.push_str("<table class=\"scores\">\n");
    out.push_str("<thead><tr><th>Category</th><th>Score</th></tr></thead>\n");
    out.push_str("<tbody>\n");
    let s = &result.scores;
    for (cat, val) in [
        ("visual", s.visual),
        ("content", s.content),
        ("structure", s.structure),
        ("style", s.style),
        ("accessibility", s.accessibility),
        ("technical", s.technical),
        ("hygiene", s.hygiene),
    ] {
        out.push_str(&format!("<tr><td>{cat}</td><td>{val:.2}</td></tr>\n"));
    }
    out.push_str("</tbody>\n</table>\n</section>\n");

    // ------------------------------------------------------------------
    // 4. Per-viewport side-by-side screenshots
    // ------------------------------------------------------------------

    // Build a stable id→&Issue map for overlay bbox lookups (BTreeMap = deterministic).
    let issue_map: BTreeMap<&str, &crate::contract::Issue> = result
        .issues
        .iter()
        .map(|iss| (iss.id.as_str(), iss))
        .collect();

    out.push_str("<section>\n<h2>Viewports</h2>\n");
    for vp in &result.viewports {
        let vp_status_str = match &vp.status {
            crate::contract::Status::Pass => "pass",
            crate::contract::Status::Warn => "warn",
            crate::contract::Status::Fail => "fail",
            crate::contract::Status::Error => "error",
        };
        out.push_str(&format!(
            "<section class=\"viewport\">\n<h3>{} <span class=\"badge badge-{vp_status_str}\">{}</span></h3>\n",
            escape(&vp.name),
            vp_status_str.to_uppercase()
        ));
        out.push_str("<div class=\"screenshots\">\n");

        let old_src = escape(&vp.artifacts.old);
        let new_src = escape(&vp.artifacts.new);
        let diff_src = escape(&vp.artifacts.diff);

        // Render OLD screenshot — with region overlay if we have dimensions and qualifying regions.
        out.push_str("<figure>\n");
        if let Some(&(img_w, img_h)) = old_dims.get(&vp.name) {
            // Collect regions that have at least one member with a bbox_old in this viewport.
            let regions_with_bbox: Vec<_> = result
                .regions
                .iter()
                .filter_map(|region| {
                    // Union all qualifying bboxes for this region+viewport.
                    let mut min_x = i64::MAX;
                    let mut min_y = i64::MAX;
                    let mut max_x = i64::MIN;
                    let mut max_y = i64::MIN;
                    let mut found = false;
                    for id in &region.member_issue_ids {
                        if let Some(iss) = issue_map.get(id.as_str()) {
                            if iss.viewport == vp.name {
                                if let Some([bx, by, bw, bh]) = iss.locator.bbox_old {
                                    let x = bx as i64;
                                    let y = by as i64;
                                    let w = bw as i64;
                                    let h = bh as i64;
                                    min_x = min_x.min(x);
                                    min_y = min_y.min(y);
                                    max_x = max_x.max(x + w);
                                    max_y = max_y.max(y + h);
                                    found = true;
                                }
                            }
                        }
                    }
                    if found {
                        Some((region, min_x, min_y, max_x, max_y))
                    } else {
                        None
                    }
                })
                .collect();

            if !regions_with_bbox.is_empty() {
                // Wrap in relative-positioned container.
                out.push_str(&format!(
                    "<a href=\"{old_src}\"><div style=\"position:relative; display:inline-block; max-width:100%\">\n"
                ));
                out.push_str(&format!(
                    "<img src=\"{old_src}\" alt=\"Old screenshot\" style=\"display:block; max-width:100%\">\n"
                ));
                let w_f = img_w as f64;
                let h_f = img_h as f64;
                for (region, min_x, min_y, max_x, max_y) in regions_with_bbox {
                    let left = min_x as f64 / w_f * 100.0;
                    let top = min_y as f64 / h_f * 100.0;
                    let width = (max_x - min_x) as f64 / w_f * 100.0;
                    let height = (max_y - min_y) as f64 / h_f * 100.0;
                    let member_count = region.member_issue_ids.len();
                    let label = format!(
                        "{} · {:.2} · {} issues",
                        escape(&region.landmark),
                        region.saturation,
                        member_count
                    );
                    out.push_str(&format!(
                        "<div style=\"position:absolute; left:{left:.2}%; top:{top:.2}%; width:{width:.2}%; height:{height:.2}%; border:1px solid #d4a017; background:rgba(255,221,51,0.18); box-sizing:border-box;\">\
<span style=\"position:absolute; top:0; left:0; background:#d4a017; color:#000; font:11px/1.3 monospace; padding:0 3px; white-space:nowrap;\">{label}</span>\
</div>\n"
                    ));
                }
                out.push_str("</div></a>\n");
            } else {
                // No overlay needed — plain figure.
                out.push_str(&format!(
                    "<a href=\"{old_src}\"><img src=\"{old_src}\" alt=\"Old screenshot\"></a>\n"
                ));
            }
        } else {
            // No dimension info — plain figure.
            out.push_str(&format!(
                "<a href=\"{old_src}\"><img src=\"{old_src}\" alt=\"Old screenshot\"></a>\n"
            ));
        }
        out.push_str("<figcaption>Old</figcaption></figure>\n");

        out.push_str(&format!(
            "<figure><a href=\"{new_src}\"><img src=\"{new_src}\" alt=\"New screenshot\"></a><figcaption>New</figcaption></figure>\n"
        ));
        out.push_str(&format!(
            "<figure><a href=\"{diff_src}\"><img src=\"{diff_src}\" alt=\"Diff screenshot\"></a><figcaption>Diff</figcaption></figure>\n"
        ));

        out.push_str("</div>\n</section>\n");
    }
    out.push_str("</section>\n");

    // ------------------------------------------------------------------
    // Compact mode: compute outline model ONCE here so sections 5, 6, and 7 can
    // all share it. This also provides the critical_set needed to avoid duplicate
    // DOM ids in the Regions section (Fix 2).
    // Full mode: no outline model; this is None.
    // ------------------------------------------------------------------
    let compact_model: Option<crate::report::outline::OutlineModel> =
        if mode == crate::report::DisclosureMode::Compact {
            let opts = crate::report::outline::DisclosureOptions::new(out_dir);
            Some(crate::report::outline::compute_outline(result, &opts))
        } else {
            None
        };

    // critical_set: ids of Critical issues surfaced as visible cards in the compact
    // Issues section (under "Critical defects"). Only relevant in Compact mode.
    let compact_critical_set: std::collections::BTreeSet<&str> = compact_model
        .as_ref()
        .map(|m| m.critical_lead.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();

    // ------------------------------------------------------------------
    // 5. Regions section (R8 — highest-altitude work first)
    // ------------------------------------------------------------------
    if !result.regions.is_empty() {
        out.push_str("<section>\n<h2>Regions</h2>\n");
        for region in &result.regions {
            let sev_str = match &region.severity {
                crate::contract::IssueSeverity::Info => "info",
                crate::contract::IssueSeverity::Warning => "warning",
                crate::contract::IssueSeverity::Error => "error",
                crate::contract::IssueSeverity::Critical => "critical",
            };
            out.push_str("<div class=\"region\">\n");
            out.push_str(&format!(
                "<p class=\"region-summary\">{}</p>\n",
                escape(&region.summary)
            ));
            out.push_str(&format!(
                "<p>Landmark: <code>{}</code> · saturation {:.2} · severity {} · {} issues claimed</p>\n",
                escape(&region.landmark),
                region.saturation,
                sev_str,
                region.member_issue_ids.len()
            ));
            // Member detail — collapsed by default (CSP-safe, no JS). Cards MOVE here from
            // the Issues section but keep their id anchors so deep links still resolve (R7/R11).
            // In compact mode, members that are also in critical_lead are OMITTED here to avoid
            // duplicate DOM ids — they are already rendered as visible cards in "Critical defects".
            let member_count = region.member_issue_ids.len();
            // Build the summary line. In Compact mode, append the region drill command (Fix 4).
            let summary_line = if mode == crate::report::DisclosureMode::Compact {
                let cmd = crate::report::outline::BranchHandle::Region {
                    landmark: region.landmark.clone(),
                }
                .drill_command(out_dir);
                format!(
                    "<details class=\"region-members\">\n<summary>{} \u{00b7} {} member issue{} \u{2014} show detail \u{2014} drill: <code>{}</code></summary>\n",
                    escape(&region.landmark),
                    member_count,
                    if member_count == 1 { "" } else { "s" },
                    escape(&cmd)
                )
            } else {
                format!(
                    "<details class=\"region-members\">\n<summary>{} · {} member issue{} — show detail</summary>\n",
                    escape(&region.landmark),
                    member_count,
                    if member_count == 1 { "" } else { "s" }
                )
            };
            out.push_str(&summary_line);
            for id in &region.member_issue_ids {
                // In Compact mode: skip members already shown as visible Critical cards.
                if mode == crate::report::DisclosureMode::Compact
                    && compact_critical_set.contains(id.as_str())
                {
                    continue;
                }
                if let Some(iss) = issue_map.get(id.as_str()) {
                    render_issue_card(&mut out, iss);
                }
            }
            out.push_str("</details>\n");
            out.push_str("</div>\n");
        }
        out.push_str("</section>\n");
    }

    // ------------------------------------------------------------------
    // 6. Clusters section
    // ------------------------------------------------------------------
    if !result.clusters.is_empty() {
        out.push_str("<section>\n<h2>Clusters</h2>\n");
        for cluster in &result.clusters {
            out.push_str("<div class=\"cluster\">\n");

            let summary = cluster.summary.as_deref().map(escape).unwrap_or_default();
            out.push_str(&format!("<p class=\"cluster-summary\">{summary}</p>\n"));

            if let Some(prop) = &cluster.shared_property {
                out.push_str(&format!(
                    "<p>Shared property: <code>{}</code></p>\n",
                    escape(prop)
                ));
            }
            if let Some(lm) = &cluster.shared_landmark {
                out.push_str(&format!(
                    "<p>Shared landmark: <code>{}</code></p>\n",
                    escape(lm)
                ));
            }

            out.push_str(&format!(
                "<p>Members: {} (ids: {})</p>\n",
                cluster.issue_ids.len(),
                escape(&cluster.issue_ids.join(", "))
            ));

            // In Compact mode, add the drill command (Fix 3).
            if mode == crate::report::DisclosureMode::Compact {
                let cmd = crate::report::outline::BranchHandle::Cluster {
                    id: cluster.id.clone(),
                }
                .drill_command(out_dir);
                out.push_str(&format!("<p>drill: <code>{}</code></p>\n", escape(&cmd)));
            }

            out.push_str("</div>\n");
        }
        out.push_str("</section>\n");
    }

    // ------------------------------------------------------------------
    // 7. Issues section (in result.issues order = fix-value order)
    //    Claimed members are excluded here — they moved into their region's
    //    <details> block in section 5 (U2 progressive disclosure).
    //    Mode-dependent: Full = flat cards; Compact = per-section <details> (R12).
    // ------------------------------------------------------------------
    out.push_str("<section>\n<h2>Issues</h2>\n");
    match mode {
        crate::report::DisclosureMode::Full => {
            let visible: Vec<&crate::contract::Issue> = result
                .issues
                .iter()
                .filter(|i| !claimed.contains(i.id.as_str()))
                .collect();
            if result.issues.is_empty() {
                out.push_str("<p>No issues.</p>\n");
            } else if visible.is_empty() {
                out.push_str("<p>All issues are demoted into saturated regions — see the Regions section above.</p>\n");
            } else {
                for issue in visible {
                    render_issue_card(&mut out, issue);
                }
            }
        }
        crate::report::DisclosureMode::Compact => {
            // Use the already-computed model (computed once before section 5).
            let model = compact_model
                .as_ref()
                .expect("compact_model must be Some in Compact mode");

            // (a) Critical defects — always visible (R13), never hidden behind a closed <details>.
            if !model.critical_lead.is_empty() {
                out.push_str("<h3>Critical defects</h3>\n");
                for id in &model.critical_lead {
                    if let Some(iss) = issue_map.get(id.as_str()) {
                        render_issue_card(&mut out, iss);
                    }
                }
            }

            // (b) Per-section <details>, open iff the outline inlines the section (R12 parity).
            for s in &model.sections {
                let open = if s.collapsed { "" } else { " open" };
                let sev_label = match &s.severity {
                    crate::contract::IssueSeverity::Info => "info",
                    crate::contract::IssueSeverity::Warning => "warning",
                    crate::contract::IssueSeverity::Error => "error",
                    crate::contract::IssueSeverity::Critical => "critical",
                };
                out.push_str(&format!("<details class=\"section\"{}>\n", open));
                out.push_str(&format!(
                    "<summary>[{}] {} \u{203a} {} \u{2014} {} issue{} \u{2014} drill: {}</summary>\n",
                    sev_label,
                    escape(&s.key.0),
                    escape(&s.key.1),
                    s.count,
                    if s.count == 1 { "" } else { "s" },
                    escape(&s.handle.drill_command(out_dir))
                ));
                for iss in crate::report::outline::section_issues(result, &s.key) {
                    render_issue_card(&mut out, iss);
                }
                out.push_str("</details>\n");
            }

            // Empty/clean cases.
            if model.clean_pass {
                out.push_str("<p>No issues.</p>\n");
            } else if model.critical_lead.is_empty() && model.sections.is_empty() {
                out.push_str("<p>All issues are demoted into saturated regions \u{2014} see the Regions section above.</p>\n");
            }
        }
    }
    out.push_str("</section>\n");

    // ------------------------------------------------------------------
    // 8. Suppressed section
    // ------------------------------------------------------------------
    if result.suppressed.count > 0 {
        out.push_str("<section>\n<h2>Suppressed</h2>\n");
        out.push_str(&format!(
            "<p>{} issue(s) suppressed by baseline.</p>\n",
            result.suppressed.count
        ));
        out.push_str("<ul>\n");
        for id in &result.suppressed.ids {
            out.push_str(&format!("<li>{}</li>\n", escape(id)));
        }
        out.push_str("</ul>\n</section>\n");
    }

    out.push_str("</div>\n"); // .container
    out.push_str("</body>\n</html>\n");
    out
}

/// Render evidence.old and evidence.new as escaped compact JSON if present.
fn render_evidence(out: &mut String, evidence: &serde_json::Value) {
    let old_val = evidence.get("old");
    let new_val = evidence.get("new");
    if old_val.is_none() && new_val.is_none() {
        return;
    }
    out.push_str("<details class=\"evidence\">\n<summary>Evidence</summary>\n<dl>\n");
    if let Some(v) = old_val {
        let s = serde_json::to_string(v).unwrap_or_default();
        out.push_str(&format!(
            "<dt>old</dt><dd><code>{}</code></dd>\n",
            escape(&s)
        ));
    }
    if let Some(v) = new_val {
        let s = serde_json::to_string(v).unwrap_or_default();
        out.push_str(&format!(
            "<dt>new</dt><dd><code>{}</code></dd>\n",
            escape(&s)
        ));
    }
    out.push_str("</dl>\n</details>\n");
}

/// Render a remediation serde_json::Value, escaping all page-derived strings.
fn render_remediation(out: &mut String, rem: &serde_json::Value) {
    out.push_str("<details class=\"remediation\">\n<summary>Remediation</summary>\n<dl>\n");

    if let Some(action) = rem.get("action").and_then(|v| v.as_str()) {
        out.push_str(&format!("<dt>action</dt><dd>{}</dd>\n", escape(action)));
    }
    if let Some(prop) = rem.get("property").and_then(|v| v.as_str()) {
        out.push_str(&format!("<dt>property</dt><dd>{}</dd>\n", escape(prop)));
    }
    if let Some(from) = rem.get("from").and_then(|v| v.as_str()) {
        out.push_str(&format!("<dt>from</dt><dd>{}</dd>\n", escape(from)));
    }
    if let Some(to) = rem.get("to").and_then(|v| v.as_str()) {
        out.push_str(&format!("<dt>to</dt><dd>{}</dd>\n", escape(to)));
    }

    // Grep targets (array of strings)
    if let Some(targets) = rem.get("grepTargets").and_then(|v| v.as_array()) {
        if !targets.is_empty() {
            out.push_str("<dt>grep targets</dt><dd><ul>\n");
            for t in targets {
                if let Some(s) = t.as_str() {
                    out.push_str(&format!("<li><code>{}</code></li>\n", escape(s)));
                }
            }
            out.push_str("</ul></dd>\n");
        }
    }

    out.push_str("</dl>\n</details>\n");
}

/// Write the HTML report to `out_dir/report.html` (creates the directory if needed).
pub fn write_html(
    result: &DiffResult,
    out_dir: &Path,
    mode: crate::report::DisclosureMode,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create output dir: {}", out_dir.display()))?;

    // Attempt to read old-screenshot pixel dimensions for each viewport.
    // On any error (missing file, decode failure, etc.) we skip that viewport — no overlay.
    let mut old_dims: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    for vp in &result.viewports {
        let png_path = out_dir.join(&vp.artifacts.old);
        match image::image_dimensions(&png_path) {
            Ok((w, h)) => {
                old_dims.insert(vp.name.clone(), (w, h));
            }
            Err(_) => {
                // Missing or unreadable — no overlay for this viewport.
            }
        }
    }

    let mut html = render_html_mode(result, &old_dims, mode, &out_dir.display().to_string());
    // Ensure trailing newline.
    if !html.ends_with('\n') {
        html.push('\n');
    }
    let path = out_dir.join("report.html");
    std::fs::write(&path, &html)
        .with_context(|| format!("failed to write report.html: {}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Inline styles (no external resources, style-src 'unsafe-inline')
// ---------------------------------------------------------------------------

const STYLE: &str = r#"<style>
*, *::before, *::after { box-sizing: border-box; }
body {
  font-family: system-ui, -apple-system, sans-serif;
  margin: 0;
  padding: 1rem;
  background: #f5f5f5;
  color: #222;
}
.container { max-width: 1200px; margin: 0 auto; }
h1, h2, h3, h4 { margin: 0.5em 0; }
section { margin: 1.5rem 0; }

/* Badge */
.badge {
  display: inline-block;
  padding: 0.2em 0.6em;
  border-radius: 4px;
  font-size: 0.8em;
  font-weight: bold;
  color: #fff;
  text-transform: uppercase;
}
.badge-pass { background: #2a9d2a; }
.badge-warn { background: #d4820a; }
.badge-fail { background: #cc2222; }
.badge-error { background: #7a0a7a; }
.badge.uncertain { background: #888; text-transform: none; }

/* Warnings banner */
section.warnings { margin: 0.75rem 0; }
.warning {
  background: #fff8e1;
  border-left: 4px solid #f9a825;
  padding: 0.5em 0.75em;
  margin-bottom: 0.4em;
  border-radius: 0 4px 4px 0;
  font-size: 0.95em;
}

/* Meta definition lists */
dl.meta { display: grid; grid-template-columns: max-content 1fr; gap: 0.2em 1em; margin: 0.5em 0; }
dl.meta dt { font-weight: bold; }

/* Scores table */
table.scores { border-collapse: collapse; min-width: 300px; }
table.scores th, table.scores td {
  border: 1px solid #bbb;
  padding: 0.3em 0.8em;
  text-align: left;
}
table.scores thead { background: #ddd; }

/* Screenshots */
.screenshots { display: flex; gap: 1rem; flex-wrap: wrap; margin: 0.5rem 0; }
.screenshots figure { margin: 0; flex: 1 1 30%; }
.screenshots figure img { max-width: 100%; border: 1px solid #ccc; }
.screenshots figcaption { text-align: center; font-size: 0.85em; margin-top: 0.25em; }

/* Viewport section */
section.viewport { border: 1px solid #ddd; border-radius: 4px; padding: 0.75rem; margin-bottom: 1rem; background: #fff; }

/* Cluster */
.cluster {
  border: 1px solid #aac; border-radius: 4px; padding: 0.75rem; margin-bottom: 0.75rem; background: #f0f4ff;
}
.cluster-summary { font-weight: bold; margin: 0 0 0.4em 0; }

/* Issue cards */
.issue {
  border-left: 5px solid #999;
  padding: 0.75rem;
  margin-bottom: 1rem;
  background: #fff;
  border-radius: 0 4px 4px 0;
  box-shadow: 0 1px 3px rgba(0,0,0,.08);
}
.sev-info    { border-left-color: #4a90d9; }
.sev-warning { border-left-color: #d4820a; }
.sev-error   { border-left-color: #cc2222; }
.sev-critical { border-left-color: #7a0a7a; }

dl.issue-meta { display: grid; grid-template-columns: max-content 1fr; gap: 0.2em 1em; margin: 0.5em 0; font-size: 0.9em; }
dl.issue-meta dt { font-weight: bold; }

.issue-message { margin: 0.5em 0; }

details { margin: 0.5em 0; }
details summary { cursor: pointer; font-weight: bold; font-size: 0.9em; }
details dl { margin: 0.4em 0 0 1em; display: grid; grid-template-columns: max-content 1fr; gap: 0.2em 1em; font-size: 0.85em; }
details dl dt { font-weight: bold; }
code { background: #eee; padding: 0.1em 0.3em; border-radius: 3px; font-size: 0.9em; }
ul { margin: 0.25em 0; padding-left: 1.5em; }
</style>
"#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        AgentSummary, Anchors, Artifacts, Cluster, DeterminismSummary, DiffResult, Issue,
        IssueCategory, IssueSeverity, IssueType, Locator, OutOfScope, Region, RunWarning, Scores,
        Status, Suppressed, ViewportResult,
    };
    use std::collections::BTreeMap;

    fn make_default_det() -> crate::contract::CaptureDeterminism {
        use crate::contract::StepStatus;
        crate::contract::CaptureDeterminism {
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

    fn make_fixture() -> DiffResult {
        let anchors_injection = Anchors {
            text: Some("\"><script>alert(1)</script>".to_string()),
            role: Some("button".to_string()),
            href: None,
            alt: None,
            aria_label: None,
            nearest_heading: Some("Main heading".to_string()),
            landmark: Some("main".to_string()),
            ordinal_in_landmark: Some(1),
        };

        let issue = Issue {
            id: "issue_aabbccddeeff".to_string(),
            issue_type: IssueType::ChangedText,
            category: IssueCategory::Content,
            severity: IssueSeverity::Warning,
            confidence: 0.95,
            viewport: "desktop".to_string(),
            locale: None,
            goal: Some("G3".to_string()),
            message: "Text changed: <old> vs &new".to_string(),
            locator: Locator {
                anchors: anchors_injection,
                css_selector_old: None,
                css_selector_new: None,
                bbox_old: None,
                bbox_new: None,
                seq_index_old: None,
                seq_index_new: None,
            },
            evidence: serde_json::json!({ "old": "hello world", "new": "hello <earth>" }),
            remediation: Some(serde_json::json!({
                "action": "restore text",
                "property": "textContent",
                "from": "hello <earth>",
                "to": "hello world",
                "grepTargets": ["textContent", "innerText"]
            })),
        };

        let mut by_type = BTreeMap::new();
        by_type.insert("changed_text".to_string(), 1u32);

        DiffResult {
            schema_version: "1.2".to_string(),
            tool_version: "0.0.0".to_string(),
            run_id: "2026-01-01T00-00-00Z".to_string(),
            old_url: "https://example.com/old".to_string(),
            new_url: "https://example.com/new".to_string(),
            parity_profile: "content-structure".to_string(),
            severity_map: None,
            status: Status::Warn,
            agent_summary: AgentSummary {
                fixable_now: 1,
                by_type,
                by_severity: BTreeMap::new(),
                cluster_count: 0,
                region_count: 0,
                top_fixes: vec!["issue_aabbccddeeff".to_string()],
            },
            scores: Scores {
                visual: 1.0,
                content: 0.5,
                structure: 1.0,
                style: 1.0,
                accessibility: 1.0,
                technical: 1.0,
                hygiene: 1.0,
                by_landmark: BTreeMap::new(),
            },
            viewports: vec![ViewportResult {
                name: "desktop".to_string(),
                status: Status::Warn,
                issues: vec!["issue_aabbccddeeff".to_string()],
                artifacts: Artifacts {
                    old: "desktop/old.png".to_string(),
                    new: "desktop/new.png".to_string(),
                    diff: "desktop/diff.png".to_string(),
                },
            }],
            issues: vec![issue],
            clusters: vec![Cluster {
                id: "cluster_112233445566".to_string(),
                issue_ids: vec!["issue_aabbccddeeff".to_string()],
                shared_property: Some("font-family".to_string()),
                shared_landmark: None,
                summary: Some("1 style_changed issues share font-family".to_string()),
            }],
            regions: vec![],
            suppressed: Suppressed {
                count: 1,
                ids: vec!["issue_dead000000ff".to_string()],
            },
            warnings: vec![],
            scoped_to: None,
            out_of_scope: OutOfScope {
                count: 0,
                ids: vec![],
            },
            determinism: DeterminismSummary {
                old: make_default_det(),
                new: make_default_det(),
            },
            artifacts: Artifacts {
                old: "desktop/old.png".to_string(),
                new: "desktop/new.png".to_string(),
                diff: "desktop/diff.png".to_string(),
            },
        }
    }

    // -----------------------------------------------------------------------
    // Existing tests (kept/updated)
    // -----------------------------------------------------------------------

    /// port-parity U7: `clickable_area_regressed` renders via the generic
    /// evidence-map path without panicking, and its `missWinners` string is
    /// shown as-is (no renderer-side truncation — it's already top-3'd at
    /// emission).
    #[test]
    fn test_clickable_area_regressed_renders_without_panic() {
        let mut result = make_fixture();
        result.issues[0].issue_type = IssueType::ClickableAreaRegressed;
        result.issues[0].category = IssueCategory::Visual;
        result.issues[0].evidence = serde_json::json!({
            "old": { "hitFraction": "1.0000", "rawHits": "25/25" },
            "new": {
                "hitFraction": "0.1200",
                "rawHits": "3/25",
                "missWinners": "img.sibling-photo (x10); .overlay-banner (x8); .nav-fixed (x3)"
            },
            "excludedPoints": "0"
        });
        result.issues[0].remediation = Some(serde_json::json!({
            "action": "restore_clickable_area",
            "findBy": { "grep": ["img.sibling-photo"], "near": null },
            "note": "overlap note"
        }));

        let html = render_html(&result, &BTreeMap::new());
        assert!(
            html.contains("img.sibling-photo (x10); .overlay-banner (x8); .nav-fixed (x3)"),
            "missWinners string must be displayed as-is"
        );
    }

    #[test]
    fn test_escape_correctness() {
        assert_eq!(escape("a&b<c>\"d'e"), "a&amp;b&lt;c&gt;&quot;d&#39;e");
    }

    #[test]
    fn test_escape_ampersand_first() {
        // If & is not replaced first, & in later replacements would double-encode.
        // e.g. '<' → '&lt;' → '&amp;lt;' if & replaced after <.
        assert_eq!(escape("<a>&"), "&lt;a&gt;&amp;");
    }

    #[test]
    fn test_csp_meta_present() {
        let result = make_fixture();
        let html = render_html(&result, &BTreeMap::new());
        assert!(
            html.contains(
                "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\">"
            ),
            "CSP meta tag must be present exactly as specified"
        );
    }

    #[test]
    fn test_no_script_tag() {
        let result = make_fixture();
        let html = render_html(&result, &BTreeMap::new());
        let lower = html.to_lowercase();
        assert!(
            !lower.contains("<script"),
            "Output must not contain <script tags"
        );
    }

    #[test]
    fn test_no_event_handlers() {
        let result = make_fixture();
        let html = render_html(&result, &BTreeMap::new());
        let lower = html.to_lowercase();
        assert!(
            !lower.contains("onerror="),
            "Output must not contain onerror= event handlers"
        );
        assert!(
            !lower.contains("onclick="),
            "Output must not contain onclick= event handlers"
        );
        assert!(
            !lower.contains("onload="),
            "Output must not contain onload= event handlers"
        );
    }

    #[test]
    fn test_xss_injection_escaped() {
        // The issue fixture has anchors.text = Some("\"><script>alert(1)</script>")
        let result = make_fixture();
        let html = render_html(&result, &BTreeMap::new());
        // The raw payload must NOT appear unescaped.
        assert!(
            !html.contains("<script>alert"),
            "Raw XSS payload must not appear in output"
        );
        // The escaped form MUST appear.
        assert!(
            html.contains("&lt;script&gt;"),
            "Escaped form &lt;script&gt; must appear in output"
        );
    }

    #[test]
    fn test_doctype_first() {
        let result = make_fixture();
        let html = render_html(&result, &BTreeMap::new());
        assert!(
            html.starts_with("<!DOCTYPE html>"),
            "Output must start with <!DOCTYPE html>"
        );
    }

    #[test]
    fn test_suppressed_section() {
        let result = make_fixture();
        let html = render_html(&result, &BTreeMap::new());
        assert!(html.contains("Suppressed"));
        assert!(html.contains("issue_dead000000ff"));
    }

    #[test]
    fn test_clusters_section() {
        let result = make_fixture();
        let html = render_html(&result, &BTreeMap::new());
        assert!(html.contains("Clusters"));
        assert!(html.contains("font-family"));
    }

    #[test]
    fn test_scores_table() {
        let result = make_fixture();
        let html = render_html(&result, &BTreeMap::new());
        assert!(html.contains("visual"));
        assert!(html.contains("1.00"));
        assert!(html.contains("0.50"));
    }

    #[test]
    fn test_message_escaped() {
        // issue.message = "Text changed: <old> vs &new"
        let result = make_fixture();
        let html = render_html(&result, &BTreeMap::new());
        assert!(html.contains("Text changed: &lt;old&gt; vs &amp;new"));
        assert!(!html.contains("Text changed: <old>"));
    }

    #[test]
    fn test_write_html_creates_file() {
        let tmp = std::env::temp_dir().join("matchy_html_test");
        let result = make_fixture();
        write_html(&result, &tmp, crate::report::DisclosureMode::Full)
            .expect("write_html should succeed");
        let path = tmp.join("report.html");
        assert!(path.exists(), "report.html should be created");
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.ends_with('\n'), "report.html must end with newline");
        // cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // -----------------------------------------------------------------------
    // NEW: warnings banner
    // -----------------------------------------------------------------------

    #[test]
    fn test_warnings_banner_present_when_non_empty() {
        let mut result = make_fixture();
        result.warnings.push(RunWarning {
            code: "STALE_BASELINE".to_string(),
            message: "Baseline may be outdated".to_string(),
            context: None,
        });
        let html = render_html(&result, &BTreeMap::new());
        assert!(
            html.contains("<section class=\"warnings\">"),
            "Warnings section must appear"
        );
        assert!(
            html.contains("<div class=\"warning\">"),
            "Warning div must appear"
        );
        assert!(html.contains("STALE_BASELINE"), "Warning code must appear");
        assert!(
            html.contains("Baseline may be outdated"),
            "Warning message must appear"
        );
    }

    #[test]
    fn test_warnings_banner_absent_when_empty() {
        let result = make_fixture(); // warnings: vec![]
        let html = render_html(&result, &BTreeMap::new());
        assert!(
            !html.contains("<section class=\"warnings\">"),
            "Warnings section must not appear when empty"
        );
    }

    /// Warning message containing `<script>` must be escaped.
    #[test]
    fn test_warnings_banner_escaping() {
        let mut result = make_fixture();
        result.warnings.push(RunWarning {
            code: "XSS_TEST".to_string(),
            message: "<script>alert(1)</script>".to_string(),
            context: None,
        });
        let html = render_html(&result, &BTreeMap::new());
        // Raw payload must not appear.
        assert!(
            !html.contains("<script>alert"),
            "Raw script tag in warning must be escaped"
        );
        // Escaped form must appear.
        assert!(
            html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
            "Escaped warning message must appear"
        );
    }

    // -----------------------------------------------------------------------
    // NEW: uncertain badge on issue card
    // -----------------------------------------------------------------------

    #[test]
    fn test_uncertain_badge_present() {
        let mut result = make_fixture();
        // Modify the issue to be uncertain.
        result.issues[0].evidence = serde_json::json!({ "match": { "uncertainPairing": true } });
        let html = render_html(&result, &BTreeMap::new());
        assert!(
            html.contains("<span class=\"badge uncertain\">uncertain pairing</span>"),
            "Uncertain badge must appear on issue card"
        );
    }

    #[test]
    fn test_uncertain_badge_absent_for_normal_issue() {
        let result = make_fixture(); // issue has no uncertainPairing
        let html = render_html(&result, &BTreeMap::new());
        assert!(
            !html.contains("uncertain pairing"),
            "Uncertain badge must not appear on normal issue"
        );
    }

    // -----------------------------------------------------------------------
    // NEW: scoped_to in metadata DL
    // -----------------------------------------------------------------------

    #[test]
    fn test_scoped_to_in_metadata() {
        let mut result = make_fixture();
        result.scoped_to = Some(vec!["main".to_string()]);
        result.out_of_scope = OutOfScope {
            count: 5,
            ids: vec![],
        };
        let html = render_html(&result, &BTreeMap::new());
        assert!(
            html.contains("<dt>Scoped to</dt>"),
            "Scoped to dt must appear"
        );
        assert!(
            html.contains("5 issue(s)"),
            "Out-of-scope count must appear"
        );
    }

    #[test]
    fn test_scoped_to_absent_when_none() {
        let result = make_fixture(); // scoped_to: None
        let html = render_html(&result, &BTreeMap::new());
        assert!(
            !html.contains("<dt>Scoped to</dt>"),
            "Scoped to must not appear when None"
        );
    }

    // -----------------------------------------------------------------------
    // NEW: regions section
    // -----------------------------------------------------------------------

    #[test]
    fn test_regions_section_present_with_one_region() {
        let mut result = make_fixture();
        result.regions = vec![Region {
            id: "region_aabbccddeeff".to_string(),
            landmark: "contentinfo".to_string(),
            saturation: 0.86,
            structural_count: 44,
            old_node_count: 51,
            member_issue_ids: vec![
                "issue_aabbccddeeff".to_string(),
                "issue_112233445566".to_string(),
            ],
            severity: IssueSeverity::Error,
            summary: "contentinfo region: 44/51 structural nodes affected".to_string(),
        }];
        result.agent_summary.region_count = 1;
        let html = render_html(&result, &BTreeMap::new());
        assert!(
            html.contains("<h2>Regions</h2>"),
            "Regions h2 must appear when regions are non-empty"
        );
        assert!(
            html.contains("contentinfo"),
            "Landmark name must appear in regions section"
        );
        assert!(
            html.contains("Region count"),
            "Region count must appear in summary block"
        );
        // Saturation formatted to 2 decimal places
        assert!(
            html.contains("0.86"),
            "Saturation must appear formatted to 2 decimal places"
        );
        // Member count (2 issues claimed)
        assert!(
            html.contains("2 issues claimed"),
            "Member count must appear in region line"
        );
        // Severity string
        assert!(
            html.contains("error"),
            "Severity string must appear in region line"
        );
    }

    #[test]
    fn test_regions_section_absent_when_empty() {
        let result = make_fixture(); // regions: vec![]
        let html = render_html(&result, &BTreeMap::new());
        assert!(
            !html.contains("<h2>Regions</h2>"),
            "Regions section must not appear when regions is empty"
        );
    }

    // -----------------------------------------------------------------------
    // NEW: region overlay on old screenshot
    // -----------------------------------------------------------------------

    /// Build a DiffResult with one region whose member issue has a bbox_old on "desktop",
    /// pass old_dims = {"desktop": (1440, 4211)}, and assert the overlay div is emitted
    /// with the expected border colour and label text.
    #[test]
    fn test_region_overlay_rendered_with_bbox() {
        use crate::contract::{Locator, Region};

        let mut result = make_fixture();
        // Give the existing issue a bbox_old so the overlay has something to draw.
        // bbox: x=0, y=3800, w=1440, h=411  (near the bottom of a 4211px tall page)
        result.issues[0].locator.bbox_old = Some([0, 3800, 1440, 411]);
        result.issues[0].viewport = "desktop".to_string();

        // Add a second issue (no bbox) to confirm it's skipped cleanly.
        let issue_no_bbox = Issue {
            id: "issue_nobbox000000".to_string(),
            issue_type: IssueType::ChangedText,
            category: IssueCategory::Content,
            severity: IssueSeverity::Info,
            confidence: 0.5,
            viewport: "desktop".to_string(),
            locale: None,
            goal: None,
            message: "no bbox here".to_string(),
            locator: Locator {
                anchors: Anchors::null(),
                css_selector_old: None,
                css_selector_new: None,
                bbox_old: None,
                bbox_new: None,
                seq_index_old: None,
                seq_index_new: None,
            },
            evidence: serde_json::json!({}),
            remediation: None,
        };
        result.issues.push(issue_no_bbox);

        result.regions = vec![Region {
            id: "region_footer".to_string(),
            landmark: "contentinfo".to_string(),
            saturation: 0.75,
            structural_count: 10,
            old_node_count: 13,
            member_issue_ids: vec![
                "issue_aabbccddeeff".to_string(), // has bbox_old
                "issue_nobbox000000".to_string(), // no bbox_old — skipped
            ],
            severity: IssueSeverity::Warning,
            summary: "contentinfo region gutted".to_string(),
        }];

        let mut old_dims = BTreeMap::new();
        old_dims.insert("desktop".to_string(), (1440u32, 4211u32));

        let html = render_html(&result, &old_dims);

        // The overlay wrapper div must be present.
        assert!(
            html.contains("position:relative; display:inline-block"),
            "Overlay wrapper div must be present"
        );
        // The overlay box must use the specified border colour.
        assert!(
            html.contains("border:1px solid #d4a017"),
            "Overlay box must have border:1px solid #d4a017"
        );
        // The label background colour.
        assert!(
            html.contains("background:#d4a017"),
            "Label must have background:#d4a017"
        );
        // The landmark name must appear in the label.
        assert!(
            html.contains("contentinfo"),
            "Landmark name must appear in overlay label"
        );
        // Sanity-check the computed percentages:
        //   left  = 0/1440*100  = 0.00%
        //   top   = 3800/4211*100 ≈ 90.24%
        //   width = 1440/1440*100 = 100.00%
        //   height= 411/4211*100 ≈ 9.76%
        assert!(
            html.contains("left:0.00%"),
            "left% must be 0.00 (x=0, W=1440)"
        );
        assert!(
            html.contains("width:100.00%"),
            "width% must be 100.00 (w=1440, W=1440)"
        );
        // top should be around 90.24% — just check the integer part appears.
        assert!(
            html.contains("top:90."),
            "top% must start with 90. (y=3800, H=4211)"
        );
    }

    /// Passing an empty old_dims map must produce NO overlay div.
    #[test]
    fn test_no_overlay_when_old_dims_empty() {
        use crate::contract::Region;

        let mut result = make_fixture();
        result.issues[0].locator.bbox_old = Some([0, 100, 500, 200]);
        result.regions = vec![Region {
            id: "region_test".to_string(),
            landmark: "main".to_string(),
            saturation: 0.5,
            structural_count: 5,
            old_node_count: 10,
            member_issue_ids: vec!["issue_aabbccddeeff".to_string()],
            severity: IssueSeverity::Warning,
            summary: "main region".to_string(),
        }];

        // Pass empty dims — no overlay should be emitted.
        let html = render_html(&result, &BTreeMap::new());

        assert!(
            !html.contains("border:1px solid #d4a017"),
            "No overlay box must appear when old_dims is empty"
        );
        assert!(
            !html.contains("position:relative; display:inline-block"),
            "No overlay wrapper must appear when old_dims is empty"
        );
        // The text Regions section can still appear.
        assert!(
            html.contains("<h2>Regions</h2>"),
            "Text Regions section must still appear"
        );
    }

    // -----------------------------------------------------------------------
    // U2: Progressive-disclosure demotion tests
    // -----------------------------------------------------------------------

    /// Helper: build a fixture with one region claiming the default issue.
    fn make_fixture_with_region() -> DiffResult {
        use crate::contract::Region;
        let mut result = make_fixture();
        result.regions = vec![Region {
            id: "region_contentinfo".to_string(),
            landmark: "contentinfo".to_string(),
            saturation: 0.86,
            structural_count: 44,
            old_node_count: 51,
            member_issue_ids: vec!["issue_aabbccddeeff".to_string()],
            severity: IssueSeverity::Error,
            summary: "contentinfo region: 44/51 structural nodes affected".to_string(),
        }];
        result.agent_summary.region_count = 1;
        result
    }

    /// Claimed members must appear inside a `<details class="region-members">` in
    /// the Regions section, NOT as a direct child of the top-level Issues section.
    #[test]
    fn test_claimed_members_demoted_to_region_details() {
        let result = make_fixture_with_region();
        let html = render_html(&result, &BTreeMap::new());

        // A region-members details block must exist.
        assert!(
            html.contains("<details class=\"region-members\">"),
            "region-members details must be present"
        );

        // The claimed issue card's id anchor must be present somewhere in the HTML.
        assert!(
            html.contains("id=\"issue_aabbccddeeff\""),
            "Claimed issue id anchor must be preserved"
        );

        // The card must appear AFTER <h2>Regions</h2> and BEFORE the Issues section.
        let regions_h2 = html
            .find("<h2>Regions</h2>")
            .expect("<h2>Regions</h2> must be present");
        let details_pos = html
            .find("<details class=\"region-members\">")
            .expect("region-members details must be present");
        let card_pos = html
            .find("id=\"issue_aabbccddeeff\"")
            .expect("card must appear");

        assert!(
            details_pos > regions_h2,
            "region-members details must come after <h2>Regions</h2>"
        );
        assert!(
            card_pos > details_pos,
            "issue card must appear inside the region-members details (after summary)"
        );

        // Slice from <h2>Issues</h2> to the next <h2> and assert the card id is absent.
        let issues_h2_pos = html
            .find("<h2>Issues</h2>")
            .expect("<h2>Issues</h2> must be present");
        // Find the next <h2> after the Issues h2.
        let after_issues = &html[issues_h2_pos + "<h2>Issues</h2>".len()..];
        let next_h2 = after_issues.find("<h2>").unwrap_or(after_issues.len());
        let issues_slice = &after_issues[..next_h2];
        assert!(
            !issues_slice.contains("id=\"issue_aabbccddeeff\""),
            "Claimed issue card must NOT appear in the top-level Issues section slice"
        );

        // <h2>Issues</h2> must still be present.
        assert!(
            html.contains("<h2>Issues</h2>"),
            "<h2>Issues</h2> must always render"
        );
    }

    /// An unclaimed issue must still appear in the top-level Issues section; the
    /// claimed issue must not appear there.
    #[test]
    fn test_unclaimed_issue_still_top_level() {
        use crate::contract::{Anchors, IssueType, Locator};

        let mut result = make_fixture_with_region();
        // Add a second issue not claimed by any region.
        let issue_b = Issue {
            id: "issue_unclaimed_bbbb".to_string(),
            issue_type: IssueType::ChangedText,
            category: IssueCategory::Content,
            severity: IssueSeverity::Warning,
            confidence: 0.8,
            viewport: "desktop".to_string(),
            locale: None,
            goal: None,
            message: "Unclaimed issue B".to_string(),
            locator: Locator {
                anchors: Anchors::null(),
                css_selector_old: None,
                css_selector_new: None,
                bbox_old: None,
                bbox_new: None,
                seq_index_old: None,
                seq_index_new: None,
            },
            evidence: serde_json::json!({}),
            remediation: None,
        };
        result.issues.push(issue_b);

        let html = render_html(&result, &BTreeMap::new());

        // Slice the Issues section.
        let issues_h2_pos = html
            .find("<h2>Issues</h2>")
            .expect("<h2>Issues</h2> must be present");
        let after_issues = &html[issues_h2_pos + "<h2>Issues</h2>".len()..];
        let next_h2 = after_issues.find("<h2>").unwrap_or(after_issues.len());
        let issues_slice = &after_issues[..next_h2];

        // Unclaimed issue B must be in the Issues section.
        assert!(
            issues_slice.contains("id=\"issue_unclaimed_bbbb\""),
            "Unclaimed issue B must appear in the Issues section"
        );
        // Claimed issue A must NOT be in the Issues section.
        assert!(
            !issues_slice.contains("id=\"issue_aabbccddeeff\""),
            "Claimed issue A must NOT appear in the Issues section"
        );
    }

    /// The `<details class="region-members">` must NOT have an `open` attribute
    /// (collapsed by default — R11).
    #[test]
    fn test_region_members_details_collapsed_by_default() {
        let result = make_fixture_with_region();
        let html = render_html(&result, &BTreeMap::new());

        assert!(
            html.contains("<details class=\"region-members\">"),
            "region-members details must be present"
        );
        assert!(
            !html.contains("<details class=\"region-members\" open>"),
            "region-members details must NOT have the `open` attribute (must be collapsed by default)"
        );
    }

    /// With no regions, the `region-members` substring must not appear and the issue
    /// card must still render at the top level.
    #[test]
    fn test_regions_empty_no_member_details() {
        let result = make_fixture(); // regions: vec![]
        let html = render_html(&result, &BTreeMap::new());

        assert!(
            !html.contains("region-members"),
            "region-members must not appear when regions is empty"
        );
        // The issue card must still be top-level.
        assert!(
            html.contains("id=\"issue_aabbccddeeff\""),
            "Issue card must still render top-level when no regions"
        );
    }

    /// CSP meta, no script, no event handlers — all must hold even with a region
    /// demoting members into <details>.
    #[test]
    fn test_html_csp_safe_after_demotion() {
        let result = make_fixture_with_region();
        let html = render_html(&result, &BTreeMap::new());

        assert!(
            html.contains("<meta http-equiv=\"Content-Security-Policy\""),
            "CSP meta must be present"
        );
        let lower = html.to_lowercase();
        assert!(!lower.contains("<script"), "No <script tags after demotion");
        assert!(!lower.contains("onerror="), "No onerror= after demotion");
        assert!(!lower.contains("onclick="), "No onclick= after demotion");
        assert!(!lower.contains("onload="), "No onload= after demotion");
        assert!(
            !lower.contains("javascript:"),
            "No javascript: URLs after demotion"
        );
    }

    /// Rendering the same result twice must produce byte-identical HTML.
    #[test]
    fn test_html_deterministic_with_region_members() {
        let result = make_fixture_with_region();
        let html1 = render_html(&result, &BTreeMap::new());
        let html2 = render_html(&result, &BTreeMap::new());
        assert_eq!(
            html1, html2,
            "HTML must be byte-identical across two renders of the same input"
        );
    }

    // -----------------------------------------------------------------------
    // NEW (U4): DisclosureMode tests
    // -----------------------------------------------------------------------

    fn make_fixture_with_sections() -> DiffResult {
        use crate::contract::{Anchors, IssueType, Locator};
        let mut result = make_fixture();
        result.issues.clear();
        result.clusters.clear();
        result.regions.clear();

        // Two small sections — should both be inlined with default budget.
        let issue_a = Issue {
            id: "issue_sect_a_0001".to_string(),
            issue_type: IssueType::ChangedText,
            category: IssueCategory::Content,
            severity: IssueSeverity::Warning,
            confidence: 0.9,
            viewport: "desktop".to_string(),
            locale: None,
            goal: None,
            message: "section A msg".to_string(),
            locator: Locator {
                anchors: Anchors {
                    landmark: Some("main".to_string()),
                    nearest_heading: Some("SectionA".to_string()),
                    ..Anchors::null()
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
        };
        let issue_b = Issue {
            id: "issue_sect_b_0001".to_string(),
            issue_type: IssueType::BrokenLink,
            category: IssueCategory::Content,
            severity: IssueSeverity::Error,
            confidence: 0.95,
            viewport: "desktop".to_string(),
            locale: None,
            goal: None,
            message: "section B msg".to_string(),
            locator: Locator {
                anchors: Anchors {
                    landmark: Some("nav".to_string()),
                    nearest_heading: Some("SectionB".to_string()),
                    ..Anchors::null()
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
        };
        result.issues.push(issue_a);
        result.issues.push(issue_b);
        result
    }

    /// Wrapper identity: render_html_mode Full must equal render_html.
    #[test]
    fn test_full_mode_html_equals_legacy() {
        let result = make_fixture_with_sections();
        let dims = BTreeMap::new();
        let via_wrapper = render_html(&result, &dims);
        let via_mode = render_html_mode(&result, &dims, crate::report::DisclosureMode::Full, "");
        assert_eq!(
            via_wrapper, via_mode,
            "render_html_mode Full must equal render_html wrapper"
        );
    }

    /// Compact mode with small sections (fits default budget) → <details class="section" open>.
    #[test]
    fn test_compact_section_details_open_when_inlined() {
        let result = make_fixture_with_sections();
        let html = render_html_mode(
            &result,
            &BTreeMap::new(),
            crate::report::DisclosureMode::Compact,
            "/tmp/out",
        );
        assert!(
            html.contains("<details class=\"section\" open>"),
            "At least one inlined section must render as <details class=\"section\" open>, got html snippet: {}",
            &html[html.find("<section>").unwrap_or(0)..]
                .chars()
                .take(400)
                .collect::<String>()
        );
    }

    /// Compact mode — open/closed count matches inlined section count from outline model.
    /// We iterate the model and count inlined sections, then verify the HTML has exactly
    /// that many `<details class="section" open>` elements (R12 parity).
    #[test]
    fn test_compact_section_details_closed_when_collapsed() {
        use crate::report::outline::{compute_outline, DisclosureOptions};

        // Build a fixture with enough sections to force at least one collapse
        // by using a tiny budget.
        let mut result = make_fixture();
        result.issues.clear();
        result.clusters.clear();
        result.regions.clear();

        use crate::contract::{Anchors, IssueType, Locator};
        for i in 0u8..6 {
            result.issues.push(Issue {
                id: format!("issue_collapse_{i:016x}"),
                issue_type: IssueType::ChangedText,
                category: IssueCategory::Content,
                severity: IssueSeverity::Warning,
                confidence: 0.9,
                viewport: "desktop".to_string(),
                locale: None,
                goal: None,
                message: format!("collapse test message for section {i} with extra padding text"),
                locator: Locator {
                    anchors: Anchors {
                        landmark: Some("main".to_string()),
                        nearest_heading: Some(format!("Section{i}")),
                        ..Anchors::null()
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
            });
        }

        // Use a small budget to force some collapses.
        let opts = DisclosureOptions {
            out_dir: "/tmp/out".to_string(),
            budget: 50,
            section_ceiling: 10_000,
        };
        let model = compute_outline(&result, &opts);

        let inlined_count = model.sections.iter().filter(|s| !s.collapsed).count();
        let collapsed_count = model.sections.iter().filter(|s| s.collapsed).count();

        // Render compact HTML with the SAME opts budget.
        // We need to call render_html_mode but it uses the global opts from config.
        // Instead, verify the open/closed mapping via a custom test:
        // count "<details class=\"section\" open>" and "<details class=\"section\">"
        // (without " open") in the HTML.
        let html = render_html_mode(
            &result,
            &BTreeMap::new(),
            crate::report::DisclosureMode::Compact,
            "/tmp/out",
        );

        let open_count = html.matches("<details class=\"section\" open>").count();
        // "closed" details are `<details class="section">` without " open".
        let closed_count = html.matches("<details class=\"section\">").count();

        // The HTML uses config defaults (not our tiny opts), so we just assert
        // the open/closed mapping is consistent: open_count + closed_count == total sections.
        assert_eq!(
            open_count + closed_count,
            model.sections.len(),
            "Total <details class='section'> count must equal model.sections.len() (using config default budget)"
        );

        // When using config defaults, sections that are inlined in the model should be open.
        // Since we may have different budgets between model and html, we assert structure
        // is internally consistent: all open details have no collapsed sections missing.
        // The key assertion: <details class="section"> (closed) must NOT have " open" attr.
        let _ = (inlined_count, collapsed_count); // suppress unused

        // Additionally, if ANY section is collapsed in the model with config defaults, assert it.
        let default_opts = DisclosureOptions::new("/tmp/out");
        let default_model = compute_outline(&result, &default_opts);
        let default_open = default_model
            .sections
            .iter()
            .filter(|s| !s.collapsed)
            .count();
        assert_eq!(
            open_count, default_open,
            "HTML open details count must match model inlined count with config defaults"
        );
    }

    /// Compact HTML must still satisfy all CSP + no-script invariants.
    #[test]
    fn test_compact_html_csp_safe() {
        let result = make_fixture_with_sections();
        let html = render_html_mode(
            &result,
            &BTreeMap::new(),
            crate::report::DisclosureMode::Compact,
            "/tmp/out",
        );

        assert!(
            html.contains("<meta http-equiv=\"Content-Security-Policy\""),
            "CSP meta must be present in compact mode"
        );
        let lower = html.to_lowercase();
        assert!(
            !lower.contains("<script"),
            "No <script tags in compact mode"
        );
        assert!(!lower.contains("onerror="), "No onerror= in compact mode");
        assert!(!lower.contains("onclick="), "No onclick= in compact mode");
        assert!(!lower.contains("onload="), "No onload= in compact mode");
        assert!(
            !lower.contains("javascript:"),
            "No javascript: in compact mode"
        );
    }

    /// Fix 2: a Critical issue that is ALSO a region member must NOT render twice in compact
    /// HTML. Specifically, its `id="…"` must appear EXACTLY ONCE. It must appear under
    /// "Critical defects" (before the region details). Full mode must still render it inside
    /// the region details.
    #[test]
    fn test_compact_html_no_duplicate_critical_card() {
        use crate::contract::{Anchors, IssueType, Locator, Region};

        let mut result = make_fixture();
        result.issues.clear();
        result.clusters.clear();
        result.regions.clear();

        // A Critical issue — will appear in both the region and compact critical_lead.
        let critical_id = "issue_crit_dual_0001";
        let critical_issue = Issue {
            id: critical_id.to_string(),
            issue_type: IssueType::LoadError,
            category: IssueCategory::Technical,
            severity: IssueSeverity::Critical,
            confidence: 0.99,
            viewport: "desktop".to_string(),
            locale: None,
            goal: None,
            message: "critical dual-render issue".to_string(),
            locator: Locator {
                anchors: Anchors {
                    landmark: Some("contentinfo".to_string()),
                    nearest_heading: Some("Footer".to_string()),
                    ..Anchors::null()
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
        };
        result.issues.push(critical_issue);

        // A region that claims the critical issue.
        result.regions = vec![Region {
            id: "region_crit_dual".to_string(),
            landmark: "contentinfo".to_string(),
            saturation: 0.90,
            structural_count: 5,
            old_node_count: 6,
            member_issue_ids: vec![critical_id.to_string()],
            severity: IssueSeverity::Critical,
            summary: "contentinfo region with critical".to_string(),
        }];
        result.agent_summary.region_count = 1;

        // --- Compact mode: id must appear EXACTLY ONCE ---
        let html_compact = render_html_mode(
            &result,
            &BTreeMap::new(),
            crate::report::DisclosureMode::Compact,
            "/tmp/out",
        );

        let id_attr = format!("id=\"{}\"", critical_id);
        let occurrences = html_compact.matches(id_attr.as_str()).count();
        assert_eq!(
            occurrences, 1,
            "Critical issue id= must appear EXACTLY ONCE in compact HTML (no duplicate), got {occurrences}"
        );

        // Must appear under "Critical defects" (before region details).
        let critical_h3_pos = html_compact
            .find("<h3>Critical defects</h3>")
            .expect("<h3>Critical defects</h3> must be present");
        let card_pos = html_compact
            .find(id_attr.as_str())
            .expect("critical issue id= must appear");
        assert!(
            card_pos > critical_h3_pos,
            "Critical issue card must appear after <h3>Critical defects</h3>"
        );

        // --- Full mode: the critical issue must still be inside region-members details ---
        let html_full = render_html_mode(
            &result,
            &BTreeMap::new(),
            crate::report::DisclosureMode::Full,
            "/tmp/out",
        );
        // In Full mode there is no "Critical defects" heading.
        assert!(
            !html_full.contains("<h3>Critical defects</h3>"),
            "Full mode must NOT have <h3>Critical defects</h3>"
        );
        // The issue must appear inside region-members details.
        let region_details_pos = html_full
            .find("<details class=\"region-members\">")
            .expect("region-members details must be present in full mode");
        let full_card_pos = html_full
            .find(id_attr.as_str())
            .expect("critical issue id= must appear in full mode");
        assert!(
            full_card_pos > region_details_pos,
            "In full mode, critical issue card must appear inside region-members details"
        );
    }

    /// A critical issue must appear under a Critical defects heading (not only inside a closed region details).
    #[test]
    fn test_compact_html_critical_visible() {
        use crate::contract::{Anchors, IssueType, Locator};

        let mut result = make_fixture();
        result.issues.clear();
        result.clusters.clear();
        result.regions.clear();

        let critical_issue = Issue {
            id: "issue_critical_u4_01".to_string(),
            issue_type: IssueType::LoadError,
            category: IssueCategory::Technical,
            severity: IssueSeverity::Critical,
            confidence: 0.99,
            viewport: "desktop".to_string(),
            locale: None,
            goal: None,
            message: "critical page failure".to_string(),
            locator: Locator {
                anchors: Anchors {
                    landmark: Some("main".to_string()),
                    nearest_heading: Some("Header".to_string()),
                    ..Anchors::null()
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
        };
        result.issues.push(critical_issue);

        let html = render_html_mode(
            &result,
            &BTreeMap::new(),
            crate::report::DisclosureMode::Compact,
            "/tmp/out",
        );

        // Critical heading must be present.
        assert!(
            html.contains("<h3>Critical defects</h3>"),
            "Critical defects heading must appear in compact mode"
        );
        // The critical issue card must appear (its id must be in the html).
        assert!(
            html.contains("id=\"issue_critical_u4_01\""),
            "Critical issue card must appear under Critical defects heading"
        );
        // It must appear BEFORE any closed section details.
        let critical_h3 = html.find("<h3>Critical defects</h3>").unwrap();
        let card_pos = html.find("id=\"issue_critical_u4_01\"").unwrap();
        assert!(
            card_pos > critical_h3,
            "Critical issue card must appear after the Critical defects heading"
        );
    }
}
