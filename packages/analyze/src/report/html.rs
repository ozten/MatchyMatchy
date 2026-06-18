//! Static HTML report renderer (M8 §5 / WP-F).
//!
//! Security invariants (spec §15):
//! - Every page-derived string passes through `escape()` before interpolation.
//! - Restrictive CSP meta tag present in <head>.
//! - No <script> tags, no inline event handlers, no javascript: urls.
//! - Single inline <style> block; no external resources.

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
// Helper
// ---------------------------------------------------------------------------

fn is_uncertain_pairing(evidence: &serde_json::Value) -> bool {
    evidence
        .get("match")
        .and_then(|m| m.get("uncertainPairing"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Render a DiffResult into a self-contained static HTML string.
/// Pure function, deterministic, no filesystem access.
pub fn render_html(result: &DiffResult) -> String {
    let mut out = String::with_capacity(64 * 1024);

    let status_str = match &result.status {
        crate::contract::Status::Pass => "pass",
        crate::contract::Status::Warn => "warn",
        crate::contract::Status::Fail => "fail",
        crate::contract::Status::Error => "error",
    };

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

        out.push_str(&format!(
            "<figure><a href=\"{old_src}\"><img src=\"{old_src}\" alt=\"Old screenshot\"></a><figcaption>Old</figcaption></figure>\n"
        ));
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
    // 5. Clusters section
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

            out.push_str("</div>\n");
        }
        out.push_str("</section>\n");
    }

    // ------------------------------------------------------------------
    // 6. Issues section (in result.issues order = fix-value order)
    // ------------------------------------------------------------------
    out.push_str("<section>\n<h2>Issues</h2>\n");
    if result.issues.is_empty() {
        out.push_str("<p>No issues.</p>\n");
    } else {
        for issue in &result.issues {
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

            let uncertain = is_uncertain_pairing(&issue.evidence);

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
            render_evidence(&mut out, &issue.evidence);

            // Remediation
            if let Some(rem) = &issue.remediation {
                render_remediation(&mut out, rem);
            }

            out.push_str("</div>\n");
        }
    }
    out.push_str("</section>\n");

    // ------------------------------------------------------------------
    // 7. Suppressed section
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
pub fn write_html(result: &DiffResult, out_dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create output dir: {}", out_dir.display()))?;
    let mut html = render_html(result);
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
        IssueCategory, IssueSeverity, IssueType, Locator, OutOfScope, RunWarning, Scores, Status,
        Suppressed, ViewportResult,
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
            schema_version: "1.1".to_string(),
            tool_version: "0.0.0".to_string(),
            run_id: "2026-01-01T00-00-00Z".to_string(),
            old_url: "https://example.com/old".to_string(),
            new_url: "https://example.com/new".to_string(),
            parity_profile: "content-structure".to_string(),
            status: Status::Warn,
            agent_summary: AgentSummary {
                fixable_now: 1,
                by_type,
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
        let html = render_html(&result);
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
        let html = render_html(&result);
        let lower = html.to_lowercase();
        assert!(
            !lower.contains("<script"),
            "Output must not contain <script tags"
        );
    }

    #[test]
    fn test_no_event_handlers() {
        let result = make_fixture();
        let html = render_html(&result);
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
        let html = render_html(&result);
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
        let html = render_html(&result);
        assert!(
            html.starts_with("<!DOCTYPE html>"),
            "Output must start with <!DOCTYPE html>"
        );
    }

    #[test]
    fn test_suppressed_section() {
        let result = make_fixture();
        let html = render_html(&result);
        assert!(html.contains("Suppressed"));
        assert!(html.contains("issue_dead000000ff"));
    }

    #[test]
    fn test_clusters_section() {
        let result = make_fixture();
        let html = render_html(&result);
        assert!(html.contains("Clusters"));
        assert!(html.contains("font-family"));
    }

    #[test]
    fn test_scores_table() {
        let result = make_fixture();
        let html = render_html(&result);
        assert!(html.contains("visual"));
        assert!(html.contains("1.00"));
        assert!(html.contains("0.50"));
    }

    #[test]
    fn test_message_escaped() {
        // issue.message = "Text changed: <old> vs &new"
        let result = make_fixture();
        let html = render_html(&result);
        assert!(html.contains("Text changed: &lt;old&gt; vs &amp;new"));
        assert!(!html.contains("Text changed: <old>"));
    }

    #[test]
    fn test_write_html_creates_file() {
        let tmp = std::env::temp_dir().join("matchy_html_test");
        let result = make_fixture();
        write_html(&result, &tmp).expect("write_html should succeed");
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
        let html = render_html(&result);
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
        let html = render_html(&result);
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
        let html = render_html(&result);
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
        let html = render_html(&result);
        assert!(
            html.contains("<span class=\"badge uncertain\">uncertain pairing</span>"),
            "Uncertain badge must appear on issue card"
        );
    }

    #[test]
    fn test_uncertain_badge_absent_for_normal_issue() {
        let result = make_fixture(); // issue has no uncertainPairing
        let html = render_html(&result);
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
        let html = render_html(&result);
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
        let html = render_html(&result);
        assert!(
            !html.contains("<dt>Scoped to</dt>"),
            "Scoped to must not appear when None"
        );
    }
}
