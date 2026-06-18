//! GitHub-flavoured Markdown report renderer (M8 §6 / WP-F).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;

use crate::contract::{DiffResult, IssueSeverity};

// ---------------------------------------------------------------------------
// Table-cell safety
// ---------------------------------------------------------------------------

/// Escape a page-derived string for safe use inside a Markdown table cell.
/// Replaces `|` → `\|`, `\r`/`\n` → space, then trims.
pub fn md_cell(s: &str) -> String {
    s.replace('|', "\\|")
        .replace(['\r', '\n'], " ")
        .trim()
        .to_string()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sev_str(sev: &IssueSeverity) -> &'static str {
    match sev {
        IssueSeverity::Info => "info",
        IssueSeverity::Warning => "warning",
        IssueSeverity::Error => "error",
        IssueSeverity::Critical => "critical",
    }
}

fn worst_severity<'a>(a: &'a IssueSeverity, b: &'a IssueSeverity) -> &'a IssueSeverity {
    if b.rank() > a.rank() {
        b
    } else {
        a
    }
}

fn has_grep_targets(rem: &serde_json::Value) -> bool {
    rem.get("grepTargets")
        .and_then(|v| v.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false)
}

/// Render remediation grep-target bullets for a single issue.
fn render_grep_bullets(out: &mut String, issue_id: &str, rem: &serde_json::Value) {
    if let Some(targets) = rem.get("grepTargets").and_then(|v| v.as_array()) {
        if !targets.is_empty() {
            let targets_str: Vec<String> = targets
                .iter()
                .filter_map(|t| t.as_str())
                .map(|t| format!("`{}`", md_cell(t)))
                .collect();
            out.push_str(&format!(
                "- {}: grep {}\n",
                md_cell(issue_id),
                targets_str.join(", ")
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Section key type for grouping
// ---------------------------------------------------------------------------

/// Grouping key: (landmark-display, heading-display).
/// `(page)` when landmark is None, `—` when heading is None.
type SectionKey = (String, String);

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Full legacy markdown (back-compat wrapper; byte-identical to pre-feature).
pub fn render_markdown(result: &DiffResult) -> String {
    render_markdown_mode(result, crate::report::DisclosureMode::Full, "")
}

/// Render a DiffResult into a GitHub-flavoured Markdown string.
/// `mode` selects compact (progressive-disclosure ToC) or full (legacy dump).
/// `out_dir` is used for drill-command generation in compact mode.
/// Pure function, deterministic, no filesystem access.
// The determinism-safe fold pattern uses a HashMap for O(1) lookup and a
// parallel Vec<FoldKey> for first-appearance order. The Entry API cannot
// express the side-effecting `fold_order.push` on the vacant branch without
// restructuring, so we suppress the map_entry lint here intentionally.
#[allow(clippy::map_entry)]
pub fn render_markdown_mode(
    result: &DiffResult,
    mode: crate::report::DisclosureMode,
    out_dir: &str,
) -> String {
    let mut out = String::with_capacity(32 * 1024);

    let status_str = match &result.status {
        crate::contract::Status::Pass => "pass",
        crate::contract::Status::Warn => "warn",
        crate::contract::Status::Fail => "fail",
        crate::contract::Status::Error => "error",
    };

    let claimed = crate::report::claimed_issue_ids(result);

    // ------------------------------------------------------------------
    // 1. Header
    // ------------------------------------------------------------------
    out.push_str("# matchy report\n\n");
    out.push_str(&format!("- **Status:** {status_str}\n"));
    out.push_str(&format!("- **Old URL:** {}\n", md_cell(&result.old_url)));
    out.push_str(&format!("- **New URL:** {}\n", md_cell(&result.new_url)));
    out.push_str(&format!(
        "- **Profile:** {}\n",
        md_cell(&result.parity_profile)
    ));
    out.push_str(&format!("- **Run ID:** {}\n", md_cell(&result.run_id)));
    out.push('\n');

    // ------------------------------------------------------------------
    // 2. Warnings (only when non-empty)
    // ------------------------------------------------------------------
    if !result.warnings.is_empty() {
        out.push_str("## Warnings\n\n");
        for w in &result.warnings {
            out.push_str(&format!(
                "> ⚠ **{}**: {}\n",
                md_cell(&w.code),
                md_cell(&w.message)
            ));
        }
        out.push('\n');
    }

    // ------------------------------------------------------------------
    // 3. Summary
    // ------------------------------------------------------------------
    out.push_str("## Summary\n\n");
    out.push_str(&format!(
        "- **Fixable now:** {}\n",
        result.agent_summary.fixable_now
    ));
    out.push_str(&format!(
        "- **Cluster count:** {}\n",
        result.agent_summary.cluster_count
    ));
    out.push_str(&format!(
        "- **Region count:** {}\n",
        result.agent_summary.region_count
    ));

    if !result.agent_summary.top_fixes.is_empty() {
        let top = result.agent_summary.top_fixes.join(", ");
        out.push_str(&format!("- **Top fixes:** {}\n", md_cell(&top)));
    }

    // Scoping line
    if let Some(scoped) = &result.scoped_to {
        let scope_names = scoped.join(", ");
        out.push_str(&format!(
            "- **Scoped to:** {} ({} out-of-scope issues recorded)\n",
            md_cell(&scope_names),
            result.out_of_scope.count
        ));
    }

    if !result.agent_summary.by_type.is_empty() {
        out.push_str("\n**By type:**\n\n");
        // BTreeMap iterates in sorted key order — deterministic.
        for (type_str, count) in &result.agent_summary.by_type {
            out.push_str(&format!("- {}: {count}\n", md_cell(type_str)));
        }
    }

    // Per-section count table (from non-uncertain issues)
    {
        // Build section counts using a BTreeMap for determinism (counts will be sorted later).
        let mut counts: BTreeMap<SectionKey, u32> = BTreeMap::new();
        for issue in &result.issues {
            if crate::report::is_uncertain_pairing(&issue.evidence) || claimed.contains(issue.id.as_str()) {
                continue;
            }
            *counts.entry(crate::report::section_key_of(issue)).or_insert(0) += 1;
        }

        if !counts.is_empty() {
            out.push_str("\n**By section:**\n\n");
            out.push_str("| Landmark | Section (nearest heading) | Issues |\n");
            out.push_str("|---|---|---|\n");

            // Sort: count desc, tie-break landmark asc then heading asc.
            let mut rows: Vec<(SectionKey, u32)> = counts.into_iter().collect();
            rows.sort_by(|(ka, ca), (kb, cb)| {
                cb.cmp(ca)
                    .then_with(|| ka.0.cmp(&kb.0))
                    .then_with(|| ka.1.cmp(&kb.1))
            });

            for ((lm, hd), count) in &rows {
                out.push_str(&format!(
                    "| {} | {} | {} |\n",
                    md_cell(lm),
                    md_cell(hd),
                    count
                ));
            }
            out.push('\n');
        } else {
            out.push('\n');
        }
    }

    // ------------------------------------------------------------------
    // 4. Scores
    // ------------------------------------------------------------------
    out.push_str("## Scores\n\n");
    out.push_str("| Category | Score |\n");
    out.push_str("|---|---|\n");
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
        out.push_str(&format!("| {cat} | {val:.2} |\n"));
    }
    out.push('\n');

    // By-landmark scores table
    if !result.scores.by_landmark.is_empty() {
        out.push_str("**By landmark:**\n\n");
        out.push_str("| Landmark | Content | Structure | Style | A11y | Technical | Hygiene |\n");
        out.push_str("|---|---|---|---|---|---|---|\n");
        // BTreeMap iterates in key order — deterministic.
        for (lm, ls) in &result.scores.by_landmark {
            out.push_str(&format!(
                "| {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} |\n",
                md_cell(lm),
                ls.content,
                ls.structure,
                ls.style,
                ls.accessibility,
                ls.technical,
                ls.hygiene,
            ));
        }
        out.push('\n');
    }

    // ------------------------------------------------------------------
    // 5+6. Regions + Issues — mode-dependent
    // Full mode: legacy ## Regions block + ## Issues by section walk (unchanged).
    // Compact mode: progressive-disclosure outline from outline.rs.
    // ------------------------------------------------------------------
    match mode {
        crate::report::DisclosureMode::Full => {
            // ------------------------------------------------------------------
            // 5. Regions (saturated ARIA-landmark rollups) — ahead of the issue tail (R8)
            // ------------------------------------------------------------------
            if !result.regions.is_empty() {
                out.push_str("## Regions\n\n");
                for region in &result.regions {
                    out.push_str(&format!(
                        "- {} — saturation {:.2}, severity {}, members: {}\n",
                        md_cell(&region.summary),
                        region.saturation,
                        sev_str(&region.severity),
                        region.member_issue_ids.len(),
                    ));
                }
                out.push('\n');
            }

            // ------------------------------------------------------------------
            // 6. Issues by section (non-uncertain issues grouped)
            // ------------------------------------------------------------------
            out.push_str("## Issues by section\n\n");

            // Breadcrumb: region-claimed members are collapsed into ## Regions above and
            // are intentionally not repeated here (R7/R10 — no information lost, just demoted).
            if !result.regions.is_empty() {
                let collapsed: Vec<String> = result
                    .regions
                    .iter()
                    .map(|r| {
                        format!(
                            "{} ({} issue{})",
                            md_cell(&r.landmark),
                            r.member_issue_ids.len(),
                            if r.member_issue_ids.len() == 1 {
                                ""
                            } else {
                                "s"
                            }
                        )
                    })
                    .collect();
                out.push_str(&format!(
                    "> Saturated regions are collapsed into **## Regions** above; their member issues are not repeated below: {}.\n\n",
                    collapsed.join(", ")
                ));
            }

            // Separate uncertain vs normal issues. Claimed issues (region members) are excluded
            // from the per-issue listing — the region rollup is their single representation.
            let normal_issues: Vec<&crate::contract::Issue> = result
                .issues
                .iter()
                .filter(|i| !crate::report::is_uncertain_pairing(&i.evidence) && !claimed.contains(i.id.as_str()))
                .collect();

            let uncertain_issues: Vec<&crate::contract::Issue> = result
                .issues
                .iter()
                .filter(|i| crate::report::is_uncertain_pairing(&i.evidence))
                .collect();

            if normal_issues.is_empty() && uncertain_issues.is_empty() {
                out.push_str("No issues.\n\n");
            } else {
                // Group normal issues by section key, preserving first-appearance order for rows.
                // Use BTreeMap to collect groups deterministically by key, but the row order
                // within each section preserves first-appearance from the issues[] array.
                let mut section_groups: BTreeMap<SectionKey, Vec<&crate::contract::Issue>> =
                    BTreeMap::new();
                for issue in &normal_issues {
                    section_groups
                        .entry(crate::report::section_key_of(issue))
                        .or_default()
                        .push(issue);
                }

                // Sort sections: count desc, tie-break by (landmark, heading) asc.
                let mut sections: Vec<(SectionKey, Vec<&crate::contract::Issue>)> =
                    section_groups.into_iter().collect();
                sections.sort_by(|(ka, va), (kb, vb)| {
                    vb.len()
                        .cmp(&va.len())
                        .then_with(|| ka.0.cmp(&kb.0))
                        .then_with(|| ka.1.cmp(&kb.1))
                });

                for ((lm, hd), issues) in &sections {
                    let n = issues.len();
                    out.push_str(&format!(
                        "### {} \u{203a} {} ({n} issue{})\n\n",
                        md_cell(lm),
                        md_cell(hd),
                        if n == 1 { "" } else { "s" }
                    ));

                    // Fold rows: fold key = (issue_type_str, message).
                    // Track first-appearance index so we can sort by it.
                    // FoldKey → (severity_worst, viewports_set, count, first_issue_ref)
                    type FoldKey = (String, String);
                    struct FoldEntry<'a> {
                        worst_sev: &'a IssueSeverity,
                        viewports: BTreeMap<String, ()>,
                        count: u32,
                        first_issue: &'a crate::contract::Issue,
                    }

                    // Use a Vec to preserve first-appearance ordering.
                    let mut fold_order: Vec<FoldKey> = Vec::new();
                    let mut fold_map: std::collections::HashMap<FoldKey, FoldEntry<'_>> =
                        std::collections::HashMap::new();

                    for issue in issues.iter() {
                        let fk: FoldKey = (issue.issue_type.as_str().to_string(), issue.message.clone());
                        if !fold_map.contains_key(&fk) {
                            fold_order.push(fk.clone());
                            let mut vp_map = BTreeMap::new();
                            vp_map.insert(issue.viewport.clone(), ());
                            fold_map.insert(
                                fk,
                                FoldEntry {
                                    worst_sev: &issue.severity,
                                    viewports: vp_map,
                                    count: 1,
                                    first_issue: issue,
                                },
                            );
                        } else {
                            let entry = fold_map
                                .get_mut(&(issue.issue_type.as_str().to_string(), issue.message.clone()))
                                .unwrap();
                            entry.worst_sev = worst_severity(entry.worst_sev, &issue.severity);
                            entry.viewports.insert(issue.viewport.clone(), ());
                            entry.count += 1;
                        }
                    }

                    // Emit table header.
                    out.push_str("| Type | Severity | Viewports | Count | Message |\n");
                    out.push_str("|---|---|---|---|---|\n");

                    // Rows in first-appearance order.
                    let mut has_rem = false;
                    for fk in &fold_order {
                        let entry = &fold_map[fk];
                        let vp_sorted: Vec<&String> = entry.viewports.keys().collect();
                        // BTreeMap keys are already sorted.
                        let vp_str = vp_sorted
                            .iter()
                            .map(|v| v.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        out.push_str(&format!(
                            "| {} | {} | {} | {} | {} |\n",
                            md_cell(&fk.0),
                            sev_str(entry.worst_sev),
                            md_cell(&vp_str),
                            entry.count,
                            md_cell(&fk.1),
                        ));
                        if entry
                            .first_issue
                            .remediation
                            .as_ref()
                            .map(has_grep_targets)
                            .unwrap_or(false)
                        {
                            has_rem = true;
                        }
                    }
                    out.push('\n');

                    // Remediation grep targets (one per fold key, first instance only).
                    if has_rem {
                        out.push_str("**Remediation grep targets:**\n\n");
                        for fk in &fold_order {
                            let entry = &fold_map[fk];
                            if let Some(rem) = &entry.first_issue.remediation {
                                render_grep_bullets(&mut out, &entry.first_issue.id, rem);
                            }
                        }
                        out.push('\n');
                    }
                }

                // ------------------------------------------------------------------
                // Uncertain pairings subsection
                // ------------------------------------------------------------------
                if !uncertain_issues.is_empty() {
                    let n = uncertain_issues.len();
                    out.push_str("### Uncertain pairings (excluded from scores)\n\n");
                    out.push_str("These style differences come from element pairings the matcher could not confidently establish; they are reported for completeness and do not affect scores.\n\n");

                    out.push_str("| Type | Severity | Viewports | Count | Message |\n");
                    out.push_str("|---|---|---|---|---|\n");

                    // Fold uncertain issues (flat — no per-heading grouping).
                    type FoldKey2 = (String, String);
                    struct FoldEntry2<'a> {
                        worst_sev: &'a IssueSeverity,
                        viewports: BTreeMap<String, ()>,
                        count: u32,
                    }
                    let mut fold_order2: Vec<FoldKey2> = Vec::new();
                    let mut fold_map2: std::collections::HashMap<FoldKey2, FoldEntry2<'_>> =
                        std::collections::HashMap::new();

                    for issue in &uncertain_issues {
                        let fk: FoldKey2 = (issue.issue_type.as_str().to_string(), issue.message.clone());
                        if !fold_map2.contains_key(&fk) {
                            fold_order2.push(fk.clone());
                            let mut vp_map = BTreeMap::new();
                            vp_map.insert(issue.viewport.clone(), ());
                            fold_map2.insert(
                                fk,
                                FoldEntry2 {
                                    worst_sev: &issue.severity,
                                    viewports: vp_map,
                                    count: 1,
                                },
                            );
                        } else {
                            let entry = fold_map2
                                .get_mut(&(issue.issue_type.as_str().to_string(), issue.message.clone()))
                                .unwrap();
                            entry.worst_sev = worst_severity(entry.worst_sev, &issue.severity);
                            entry.viewports.insert(issue.viewport.clone(), ());
                            entry.count += 1;
                        }
                    }

                    for fk in &fold_order2 {
                        let entry = &fold_map2[fk];
                        let vp_str = entry
                            .viewports
                            .keys()
                            .map(|v| v.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        out.push_str(&format!(
                            "| {} | {} | {} | {} | {} |\n",
                            md_cell(&fk.0),
                            sev_str(entry.worst_sev),
                            md_cell(&vp_str),
                            entry.count,
                            md_cell(&fk.1),
                        ));
                    }
                    let _ = n;
                    out.push('\n');
                }
            }
        }
        crate::report::DisclosureMode::Compact => {
            let opts = crate::report::outline::DisclosureOptions::new(out_dir);
            out.push_str(&crate::report::outline::render_outline(result, &opts));
        }
    }

    // ------------------------------------------------------------------
    // 7. Out of scope
    // ------------------------------------------------------------------
    if result.out_of_scope.count > 0 {
        out.push_str("## Out of scope\n\n");
        let scope_str = result
            .scoped_to
            .as_ref()
            .map(|v| v.join(", "))
            .unwrap_or_default();
        out.push_str(&format!(
            "{} issue(s) outside the configured scope ({}) were excluded from issues, scores and status.\n\n",
            result.out_of_scope.count,
            md_cell(&scope_str)
        ));

        // IDs as inline code, capped at 30 with "… and K more".
        let ids = &result.out_of_scope.ids;
        let cap = 30;
        let shown: Vec<String> = ids
            .iter()
            .take(cap)
            .map(|id| format!("`{}`", md_cell(id)))
            .collect();
        out.push_str(&shown.join(", "));
        if ids.len() > cap {
            out.push_str(&format!(" … and {} more", ids.len() - cap));
        }
        out.push_str("\n\n");
    }

    // ------------------------------------------------------------------
    // 8. Clusters
    // ------------------------------------------------------------------
    if !result.clusters.is_empty() {
        out.push_str("## Clusters\n\n");
        for cluster in &result.clusters {
            let summary = cluster
                .summary
                .as_deref()
                .map(md_cell)
                .unwrap_or_else(|| cluster.id.clone());
            // In Compact mode, append the drill command (Fix 3 / R12 parity).
            if mode == crate::report::DisclosureMode::Compact {
                let cmd = crate::report::outline::BranchHandle::Cluster {
                    id: cluster.id.clone(),
                }.drill_command(out_dir);
                out.push_str(&format!(
                    "- {} (members: {}) \u{2014} drill: {}\n",
                    summary,
                    cluster.issue_ids.len(),
                    cmd
                ));
            } else {
                out.push_str(&format!(
                    "- {} (members: {})\n",
                    summary,
                    cluster.issue_ids.len()
                ));
            }
        }
        out.push('\n');
    }

    // ------------------------------------------------------------------
    // 9. Suppressed
    // ------------------------------------------------------------------
    if result.suppressed.count > 0 {
        out.push_str("## Suppressed\n\n");
        out.push_str(&format!(
            "{} issue(s) suppressed by baseline: {}\n\n",
            result.suppressed.count,
            result.suppressed.ids.join(", ")
        ));
    }

    out
}

/// Write the Markdown report to `out_dir/report.md` (creates the directory if needed).
pub fn write_markdown(
    result: &DiffResult,
    out_dir: &Path,
    mode: crate::report::DisclosureMode,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create output dir: {}", out_dir.display()))?;
    let md = render_markdown_mode(result, mode, &out_dir.display().to_string());
    let path = out_dir.join("report.md");
    std::fs::write(&path, &md)
        .with_context(|| format!("failed to write report.md: {}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        AgentSummary, Anchors, Artifacts, Cluster, DeterminismSummary, DiffResult, Issue,
        IssueCategory, IssueSeverity, IssueType, LandmarkScores, Locator, OutOfScope, Region,
        RunWarning, Scores, Status, Suppressed, ViewportResult,
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

    fn make_anchors(landmark: Option<&str>, heading: Option<&str>) -> Anchors {
        Anchors {
            landmark: landmark.map(str::to_string),
            nearest_heading: heading.map(str::to_string),
            ..Anchors::null()
        }
    }

    fn make_issue(
        id: &str,
        issue_type: IssueType,
        severity: IssueSeverity,
        viewport: &str,
        message: &str,
        landmark: Option<&str>,
        heading: Option<&str>,
        uncertain: bool,
    ) -> Issue {
        Issue {
            id: id.to_string(),
            issue_type,
            category: IssueCategory::Content,
            severity,
            confidence: 0.9,
            viewport: viewport.to_string(),
            locale: None,
            goal: None,
            message: message.to_string(),
            locator: Locator {
                anchors: make_anchors(landmark, heading),
                css_selector_old: None,
                css_selector_new: None,
                bbox_old: None,
                bbox_new: None,
                seq_index_old: None,
                seq_index_new: None,
            },
            evidence: if uncertain {
                serde_json::json!({ "match": { "uncertainPairing": true } })
            } else {
                serde_json::json!({})
            },
            remediation: None,
        }
    }

    fn make_fixture() -> DiffResult {
        let issue = Issue {
            id: "issue_aabbccddeeff".to_string(),
            issue_type: IssueType::ChangedText,
            category: IssueCategory::Content,
            severity: IssueSeverity::Warning,
            confidence: 0.95,
            viewport: "desktop".to_string(),
            locale: None,
            goal: Some("G3".to_string()),
            message: "Text changed: hello|world\nand more".to_string(),
            locator: Locator {
                anchors: make_anchors(Some("main"), Some("FAQs")),
                css_selector_old: None,
                css_selector_new: None,
                bbox_old: None,
                bbox_new: None,
                seq_index_old: None,
                seq_index_new: None,
            },
            evidence: serde_json::json!({ "old": "hello", "new": "world" }),
            remediation: Some(serde_json::json!({
                "action": "restore",
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
                cluster_count: 1,
                region_count: 0,
                top_fixes: vec!["cluster_112233445566".to_string()],
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
                count: 2,
                ids: vec![
                    "issue_dead000000ff".to_string(),
                    "issue_dead000001ff".to_string(),
                ],
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
    // Baseline / regression guards
    // -----------------------------------------------------------------------

    #[test]
    fn test_md_cell_escaping() {
        assert_eq!(md_cell("a|b\nc"), "a\\|b c");
    }

    #[test]
    fn test_md_cell_trim() {
        assert_eq!(md_cell("  hello  "), "hello");
    }

    #[test]
    fn test_md_cell_pipe_and_newlines() {
        assert_eq!(md_cell("a|b\r\nc"), "a\\|b  c");
    }

    /// check-m8.py line 125 guard: all four required substrings must be present.
    #[test]
    fn test_required_section_substrings() {
        let result = make_fixture();
        let md = render_markdown(&result);
        assert!(md.contains("# matchy report"), "Missing '# matchy report'");
        assert!(md.contains("## Summary"), "Missing '## Summary'");
        assert!(md.contains("## Scores"), "Missing '## Scores'");
        // "## Issues by section" contains "## Issues" as substring — satisfies check-m8.py.
        assert!(md.contains("## Issues"), "Missing '## Issues'");
        assert!(
            md.contains("## Issues by section"),
            "Missing '## Issues by section'"
        );
    }

    #[test]
    fn test_section_headers_present() {
        let result = make_fixture();
        let md = render_markdown(&result);
        assert!(md.contains("# matchy report"), "Missing '# matchy report'");
        assert!(md.contains("## Summary"), "Missing '## Summary'");
        assert!(md.contains("## Scores"), "Missing '## Scores'");
        assert!(
            md.contains("## Issues by section"),
            "Missing '## Issues by section'"
        );
        assert!(md.contains("## Clusters"), "Missing '## Clusters'");
        assert!(md.contains("## Suppressed"), "Missing '## Suppressed'");
    }

    #[test]
    fn test_scores_table_present() {
        let result = make_fixture();
        let md = render_markdown(&result);
        assert!(md.contains("| Category | Score |"));
        assert!(md.contains("| visual |"));
        assert!(md.contains("| content |"));
    }

    #[test]
    fn test_issue_message_cell_escaped() {
        let result = make_fixture();
        let md = render_markdown(&result);
        // The raw pipe should not appear unescaped in table context.
        // The message "Text changed: hello|world\nand more" should be cell-escaped.
        assert!(md.contains("hello\\|world"));
    }

    #[test]
    fn test_suppressed_section() {
        let result = make_fixture();
        let md = render_markdown(&result);
        assert!(md.contains("## Suppressed"));
        assert!(md.contains("issue_dead000000ff"));
        assert!(md.contains("2 issue(s) suppressed"));
    }

    #[test]
    fn test_clusters_section() {
        let result = make_fixture();
        let md = render_markdown(&result);
        assert!(md.contains("## Clusters"));
        assert!(md.contains("font-family"));
    }

    #[test]
    fn test_write_markdown_creates_file() {
        let tmp = std::env::temp_dir().join("matchy_md_test");
        let result = make_fixture();
        write_markdown(&result, &tmp, crate::report::DisclosureMode::Full)
            .expect("write_markdown should succeed");
        let path = tmp.join("report.md");
        assert!(path.exists(), "report.md should be created");
        // cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // -----------------------------------------------------------------------
    // NEW: warnings section
    // -----------------------------------------------------------------------

    #[test]
    fn test_warnings_section_present_when_non_empty() {
        let mut result = make_fixture();
        result.warnings.push(RunWarning {
            code: "STALE_BASELINE".to_string(),
            message: "Baseline may be outdated".to_string(),
            context: None,
        });
        let md = render_markdown(&result);
        assert!(md.contains("## Warnings"), "## Warnings must appear");
        assert!(
            md.contains("> ⚠ **STALE_BASELINE**: Baseline may be outdated"),
            "Blockquote warning line must appear"
        );
        // Warnings section must appear BEFORE Summary.
        let pos_warn = md.find("## Warnings").unwrap();
        let pos_summary = md.find("## Summary").unwrap();
        assert!(pos_warn < pos_summary, "Warnings must come before Summary");
    }

    #[test]
    fn test_warnings_section_absent_when_empty() {
        let result = make_fixture(); // warnings: vec![]
        let md = render_markdown(&result);
        assert!(
            !md.contains("## Warnings"),
            "## Warnings must not appear when empty"
        );
    }

    // -----------------------------------------------------------------------
    // NEW: grouped section ordering
    // -----------------------------------------------------------------------

    #[test]
    fn test_grouped_section_ordering_bigger_first() {
        // Two sections: main›FAQs (3 issues) and nav›About (1 issue).
        // Bigger section must appear first.
        let mut result = make_fixture();
        result.issues.clear();
        // 3 issues in main > FAQs
        for i in 0..3u8 {
            result.issues.push(make_issue(
                &format!("issue_{:016x}", i),
                IssueType::ChangedText,
                IssueSeverity::Warning,
                "desktop",
                &format!("msg {i}"),
                Some("main"),
                Some("FAQs"),
                false,
            ));
        }
        // 1 issue in nav > About
        result.issues.push(make_issue(
            "issue_nav_0000000001",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "desktop",
            "nav msg",
            Some("nav"),
            Some("About"),
            false,
        ));
        let md = render_markdown(&result);
        let pos_main = md.find("### main").unwrap();
        let pos_nav = md.find("### nav").unwrap();
        assert!(
            pos_main < pos_nav,
            "main (3 issues) must appear before nav (1 issue)"
        );
    }

    #[test]
    fn test_grouped_section_heading_asc_tiebreak() {
        // Two sections with same count — ordered by landmark asc then heading asc.
        let mut result = make_fixture();
        result.issues.clear();
        result.issues.push(make_issue(
            "issue_z_section_0001",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "desktop",
            "z",
            Some("z_landmark"),
            Some("Z"),
            false,
        ));
        result.issues.push(make_issue(
            "issue_a_section_0001",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "desktop",
            "a",
            Some("a_landmark"),
            Some("A"),
            false,
        ));
        let md = render_markdown(&result);
        let pos_a = md.find("### a_landmark").unwrap();
        let pos_z = md.find("### z_landmark").unwrap();
        assert!(
            pos_a < pos_z,
            "a_landmark must come before z_landmark on tie"
        );
    }

    // -----------------------------------------------------------------------
    // NEW: viewport folding
    // -----------------------------------------------------------------------

    #[test]
    fn test_viewport_folding_same_type_message() {
        // Same type+message on desktop+mobile → folded to one row, Count=2, both viewports.
        let mut result = make_fixture();
        result.issues.clear();
        // Clear clusters so `style_changed` doesn't leak into the issues section slice.
        result.clusters.clear();
        result.issues.push(make_issue(
            "issue_fold_desktop_01",
            IssueType::StyleChanged,
            IssueSeverity::Warning,
            "desktop",
            "color changed",
            Some("main"),
            Some("Hero"),
            false,
        ));
        result.issues.push(make_issue(
            "issue_fold_mobile_001",
            IssueType::StyleChanged,
            IssueSeverity::Warning,
            "mobile",
            "color changed",
            Some("main"),
            Some("Hero"),
            false,
        ));
        let md = render_markdown(&result);
        // Should see "desktop mobile" (sorted) and Count=2.
        assert!(
            md.contains("desktop mobile"),
            "Folded row must show both viewports"
        );
        assert!(md.contains("| 2 |"), "Folded row must show count 2");
        // Only one table row for style_changed (not two).
        // Count occurrences within the Issues section only.
        let issues_start = md.find("## Issues by section").unwrap();
        let issues_section = &md[issues_start..];
        let row_occurrences = issues_section.matches("style_changed").count();
        assert_eq!(row_occurrences, 1, "Only one folded row for style_changed");
    }

    // -----------------------------------------------------------------------
    // NEW: uncertain subsection excluded from main grouping
    // -----------------------------------------------------------------------

    #[test]
    fn test_uncertain_excluded_from_main_grouping() {
        let mut result = make_fixture();
        result.issues.clear();
        result.issues.push(make_issue(
            "issue_normal_0000001",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "desktop",
            "normal issue",
            Some("main"),
            Some("Hero"),
            false,
        ));
        result.issues.push(make_issue(
            "issue_uncertain_0001",
            IssueType::StyleChanged,
            IssueSeverity::Info,
            "desktop",
            "uncertain style",
            Some("main"),
            Some("Hero"),
            true,
        ));
        let md = render_markdown(&result);
        // Uncertain subsection should appear.
        assert!(
            md.contains("### Uncertain pairings"),
            "Uncertain subsection must appear"
        );
        // The main section should show count 1 (only the normal issue).
        assert!(
            md.contains("(1 issue)"),
            "Main section must show 1 issue (uncertain excluded)"
        );
        // By-section summary table must also count 1 for main>Hero.
        // "1 |" should appear in the by-section table.
        assert!(
            md.contains("| main | Hero | 1 |"),
            "By-section summary must count 1"
        );
    }

    #[test]
    fn test_uncertain_subsection_absent_when_none() {
        let result = make_fixture(); // issue has no uncertainPairing
        let md = render_markdown(&result);
        assert!(
            !md.contains("Uncertain pairings"),
            "Uncertain subsection must not appear when none"
        );
    }

    // -----------------------------------------------------------------------
    // NEW: out-of-scope section
    // -----------------------------------------------------------------------

    #[test]
    fn test_out_of_scope_section() {
        let mut result = make_fixture();
        result.scoped_to = Some(vec!["main".to_string()]);
        result.out_of_scope = OutOfScope {
            count: 3,
            ids: vec![
                "issue_oos_000000001".to_string(),
                "issue_oos_000000002".to_string(),
                "issue_oos_000000003".to_string(),
            ],
        };
        let md = render_markdown(&result);
        assert!(
            md.contains("## Out of scope"),
            "Out of scope section must appear"
        );
        assert!(
            md.contains("3 issue(s) outside the configured scope"),
            "Out of scope count must appear"
        );
        assert!(
            md.contains("`issue_oos_000000001`"),
            "IDs must appear as inline code"
        );
    }

    #[test]
    fn test_out_of_scope_absent_when_zero() {
        let result = make_fixture(); // out_of_scope.count = 0
        let md = render_markdown(&result);
        assert!(
            !md.contains("## Out of scope"),
            "Out of scope section must not appear when count=0"
        );
    }

    #[test]
    fn test_out_of_scope_id_cap() {
        let mut result = make_fixture();
        result.out_of_scope = OutOfScope {
            count: 35,
            ids: (0..35).map(|i| format!("issue_oos_{i:015}")).collect(),
        };
        let md = render_markdown(&result);
        assert!(
            md.contains("… and 5 more"),
            "Overflow must show '… and K more'"
        );
    }

    // -----------------------------------------------------------------------
    // NEW: scoped-to line in summary
    // -----------------------------------------------------------------------

    #[test]
    fn test_scoped_to_line_in_summary() {
        let mut result = make_fixture();
        result.scoped_to = Some(vec!["main".to_string(), "footer".to_string()]);
        result.out_of_scope = OutOfScope {
            count: 7,
            ids: vec![],
        };
        let md = render_markdown(&result);
        assert!(
            md.contains("**Scoped to:** main, footer (7 out-of-scope issues recorded)"),
            "Scoped-to line must appear in Summary"
        );
    }

    // -----------------------------------------------------------------------
    // NEW: by-section summary table
    // -----------------------------------------------------------------------

    #[test]
    fn test_by_section_summary_table() {
        let result = make_fixture(); // 1 issue in main > FAQs
        let md = render_markdown(&result);
        assert!(
            md.contains("| Landmark | Section (nearest heading) | Issues |"),
            "By-section table header must appear"
        );
        assert!(
            md.contains("| main | FAQs | 1 |"),
            "By-section table row must appear"
        );
    }

    // -----------------------------------------------------------------------
    // NEW: by-landmark scores table
    // -----------------------------------------------------------------------

    #[test]
    fn test_by_landmark_scores_table() {
        let mut result = make_fixture();
        let mut by_lm = BTreeMap::new();
        by_lm.insert(
            "main".to_string(),
            LandmarkScores {
                content: 0.8,
                structure: 0.9,
                style: 0.7,
                accessibility: 1.0,
                technical: 0.6,
                hygiene: 0.5,
            },
        );
        result.scores.by_landmark = by_lm;
        let md = render_markdown(&result);
        assert!(
            md.contains("**By landmark:**"),
            "By landmark header must appear"
        );
        assert!(
            md.contains("| Landmark | Content | Structure | Style | A11y | Technical | Hygiene |"),
            "By landmark table header must appear"
        );
        assert!(
            md.contains("| main |"),
            "By landmark row for main must appear"
        );
        assert!(md.contains("0.80"), "Score value 0.80 must appear");
    }

    #[test]
    fn test_by_landmark_scores_absent_when_empty() {
        let result = make_fixture(); // by_landmark is empty
        let md = render_markdown(&result);
        assert!(
            !md.contains("**By landmark:**"),
            "By landmark section must not appear when empty"
        );
    }

    // -----------------------------------------------------------------------
    // NEW: no issues case
    // -----------------------------------------------------------------------

    #[test]
    fn test_no_issues() {
        let mut result = make_fixture();
        result.issues.clear();
        let md = render_markdown(&result);
        assert!(
            md.contains("No issues."),
            "Must say No issues when list empty"
        );
    }

    // -----------------------------------------------------------------------
    // NEW: regions section
    // -----------------------------------------------------------------------

    #[test]
    fn test_regions_section_happy_path() {
        // A DiffResult with one Region renders a ## Regions section with the
        // landmark, saturation, severity, and member count visible; and the
        // ## Summary shows "Region count: 1".
        let mut result = make_fixture();
        let mut member_ids: Vec<String> = (0..88).map(|i| format!("issue_{i:016x}")).collect();
        member_ids.sort();
        let region = Region {
            id: "region_aabbccddeeff".to_string(),
            landmark: "contentinfo".to_string(),
            saturation: 0.86,
            structural_count: 44,
            old_node_count: 51,
            member_issue_ids: member_ids,
            severity: IssueSeverity::Error,
            summary: "contentinfo region: 44/51 structural nodes affected".to_string(),
        };
        result.regions = vec![region];
        result.agent_summary.region_count = 1;

        let md = render_markdown(&result);

        // ## Regions section must appear
        assert!(md.contains("## Regions"), "## Regions section must appear");

        // Summary must show the correct values
        assert!(
            md.contains("contentinfo"),
            "Region landmark must appear in Regions section"
        );
        assert!(
            md.contains("saturation 0.86"),
            "Saturation must appear formatted to 2 decimal places"
        );
        assert!(
            md.contains("members: 88"),
            "Member count must appear in Regions section"
        );

        // ## Summary must show Region count: 1
        assert!(
            md.contains("**Region count:** 1"),
            "Summary must show Region count: 1"
        );

        // ## Regions must appear BEFORE ## Issues by section (R8)
        let pos_regions = md.find("## Regions").unwrap();
        let pos_issues = md.find("## Issues by section").unwrap();
        assert!(
            pos_regions < pos_issues,
            "## Regions must appear before ## Issues by section"
        );
    }

    #[test]
    fn test_regions_section_absent_when_empty() {
        // regions: vec![] and region_count: 0 → NO ## Regions header appears,
        // and the rest of the report renders normally.
        let result = make_fixture(); // regions: vec![], region_count: 0

        let md = render_markdown(&result);

        assert!(
            !md.contains("## Regions"),
            "## Regions must not appear when regions is empty"
        );
        // The rest of the report must still be intact
        assert!(md.contains("# matchy report"), "Header must still appear");
        assert!(md.contains("## Summary"), "Summary must still appear");
        assert!(
            md.contains("## Issues by section"),
            "Issues section must still appear"
        );
        // Region count line is still present (showing 0)
        assert!(
            md.contains("**Region count:** 0"),
            "Region count line must show 0 in Summary"
        );
    }

    // -----------------------------------------------------------------------
    // NEW (U1): region demotion — claimed members excluded from per-issue listing
    // -----------------------------------------------------------------------

    /// AE1 core scenario: contentinfo is saturated. Its member issues must NOT
    /// appear in "Issues by section" or the by-section count table. The single
    /// standalone defect in `main` MUST still appear. The breadcrumb note and
    /// ## Regions section must be present.
    #[test]
    fn test_region_claimed_members_demoted_from_issues_and_counts() {
        let mut result = make_fixture();
        result.issues.clear();
        result.clusters.clear();

        // Five contentinfo issues across two headings.
        let footer_ids: &[&str] = &[
            "issue_footer_00000001",
            "issue_footer_00000002",
            "issue_footer_00000003",
            "issue_footer_00000004",
            "issue_footer_00000005",
        ];
        for (i, id) in footer_ids.iter().enumerate() {
            let heading = if i < 3 { "PRODUCTS" } else { "RESOURCES" };
            result.issues.push(make_issue(
                id,
                IssueType::ChangedText,
                IssueSeverity::Warning,
                "desktop",
                &format!("footer msg {i}"),
                Some("contentinfo"),
                Some(heading),
                false,
            ));
        }

        // One standalone issue in main.
        result.issues.push(make_issue(
            "issue_broken_link_01",
            IssueType::BrokenLink,
            IssueSeverity::Error,
            "desktop",
            "Link target missing",
            Some("main"),
            Some("Body"),
            false,
        ));

        // Build region claiming exactly the five footer issues.
        let mut member_ids: Vec<String> = footer_ids.iter().map(|s| s.to_string()).collect();
        member_ids.sort();
        let region = Region {
            id: "region_contentinfo_01".to_string(),
            landmark: "contentinfo".to_string(),
            saturation: 0.86,
            structural_count: 5,
            old_node_count: 6,
            member_issue_ids: member_ids,
            severity: IssueSeverity::Error,
            summary: "contentinfo region: 5/6 structural nodes affected".to_string(),
        };
        result.regions = vec![region];
        result.agent_summary.region_count = 1;

        let md = render_markdown(&result);

        // contentinfo issues must NOT appear in "Issues by section"
        let issues_start = md.find("## Issues by section").unwrap();
        let issues_section = &md[issues_start..];
        assert!(
            !issues_section.contains("### contentinfo"),
            "contentinfo section header must not appear in Issues by section"
        );

        // The by-section count table must not have a contentinfo row.
        // The by-section table is in the Summary section.
        let summary_start = md.find("## Summary").unwrap();
        let issues_header_pos = md.find("## Issues by section").unwrap();
        let summary_section = &md[summary_start..issues_header_pos];
        assert!(
            !summary_section.contains("| contentinfo |"),
            "by-section count table must not have a contentinfo row"
        );

        // The main broken_link issue IS still rendered.
        assert!(
            issues_section.contains("### main"),
            "main section header must appear in Issues by section"
        );
        assert!(
            summary_section.contains("| main |"),
            "by-section count table must have a main row"
        );

        // ## Regions must be present and mention contentinfo.
        assert!(md.contains("## Regions"), "## Regions must be present");
        assert!(
            md.contains("contentinfo"),
            "contentinfo must appear in ## Regions"
        );

        // The breadcrumb note must be present and mention contentinfo.
        assert!(
            md.contains("Saturated regions are collapsed into **## Regions** above"),
            "breadcrumb note must be present"
        );
        assert!(
            md.contains("contentinfo (5 issues)"),
            "breadcrumb note must mention contentinfo with count"
        );
    }

    /// Guard: with regions empty the breadcrumb note must not appear, and the
    /// normal issue still renders exactly as before (qualitative byte-stability).
    #[test]
    fn test_regions_empty_markdown_unchanged_by_filter() {
        let result = make_fixture(); // regions: vec![]
        let md = render_markdown(&result);

        // No breadcrumb note.
        assert!(
            !md.contains("Saturated regions are collapsed"),
            "breadcrumb note must not appear when regions is empty"
        );

        // Normal issue still renders.
        assert!(
            md.contains("## Issues by section"),
            "Issues section must be present"
        );
        assert!(
            md.contains("### main"),
            "main section must still appear when regions is empty"
        );
    }

    /// An uncertain issue in contentinfo whose id is NOT in the region member set
    /// must still appear in "### Uncertain pairings", even when the region claims
    /// other contentinfo issue ids.
    #[test]
    fn test_uncertain_still_rendered_when_region_claims() {
        let mut result = make_fixture();
        result.issues.clear();
        result.clusters.clear();

        // One normal contentinfo issue that WILL be claimed.
        result.issues.push(make_issue(
            "issue_footer_claimed_01",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "desktop",
            "claimed footer msg",
            Some("contentinfo"),
            Some("PRODUCTS"),
            false,
        ));

        // One uncertain contentinfo issue whose id is NOT claimed.
        result.issues.push(make_issue(
            "issue_uncertain_footer",
            IssueType::StyleChanged,
            IssueSeverity::Info,
            "desktop",
            "uncertain footer style",
            Some("contentinfo"),
            Some("PRODUCTS"),
            true, // uncertain
        ));

        // Region claims only the non-uncertain issue.
        let region = Region {
            id: "region_contentinfo_02".to_string(),
            landmark: "contentinfo".to_string(),
            saturation: 0.80,
            structural_count: 1,
            old_node_count: 1,
            member_issue_ids: vec!["issue_footer_claimed_01".to_string()],
            severity: IssueSeverity::Warning,
            summary: "contentinfo region: 1/1 structural nodes affected".to_string(),
        };
        result.regions = vec![region];
        result.agent_summary.region_count = 1;

        let md = render_markdown(&result);

        // Uncertain subsection must still appear.
        assert!(
            md.contains("### Uncertain pairings"),
            "Uncertain pairings subsection must appear"
        );
        // Uncertain issue's message must be present.
        assert!(
            md.contains("uncertain footer style"),
            "Uncertain issue message must appear"
        );
    }

    /// Byte-determinism guard: rendering the same region-bearing result twice
    /// produces identical output.
    #[test]
    fn test_markdown_render_deterministic_with_regions() {
        let mut result = make_fixture();
        result.issues.clear();
        result.clusters.clear();

        for i in 0..3u8 {
            result.issues.push(make_issue(
                &format!("issue_det_{i:016x}"),
                IssueType::ChangedText,
                IssueSeverity::Warning,
                "desktop",
                &format!("det msg {i}"),
                Some("contentinfo"),
                Some("Section"),
                false,
            ));
        }
        result.issues.push(make_issue(
            "issue_det_standalone_01",
            IssueType::BrokenLink,
            IssueSeverity::Error,
            "desktop",
            "standalone det msg",
            Some("main"),
            Some("Body"),
            false,
        ));

        let mut member_ids: Vec<String> = (0..3u8).map(|i| format!("issue_det_{i:016x}")).collect();
        member_ids.sort();
        let region = Region {
            id: "region_det_test_0001".to_string(),
            landmark: "contentinfo".to_string(),
            saturation: 0.75,
            structural_count: 3,
            old_node_count: 4,
            member_issue_ids: member_ids,
            severity: IssueSeverity::Warning,
            summary: "contentinfo region: 3/4 structural nodes affected".to_string(),
        };
        result.regions = vec![region];
        result.agent_summary.region_count = 1;

        let md1 = render_markdown(&result);
        let md2 = render_markdown(&result);

        assert_eq!(md1, md2, "render_markdown must be byte-deterministic");
    }

    // -----------------------------------------------------------------------
    // NEW (U4): DisclosureMode tests
    // -----------------------------------------------------------------------

    /// Wrapper identity: render_markdown_mode with Full must equal render_markdown.
    #[test]
    fn test_full_mode_equals_legacy_render() {
        let mut result = make_fixture();
        result.issues.clear();
        result.clusters.clear();

        // Add a region so the fixture exercises the region path.
        result.issues.push(make_issue(
            "issue_r_0000000001",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "desktop",
            "region msg",
            Some("contentinfo"),
            Some("Products"),
            false,
        ));
        result.issues.push(make_issue(
            "issue_main_001_u4m",
            IssueType::BrokenLink,
            IssueSeverity::Error,
            "desktop",
            "standalone link",
            Some("main"),
            Some("Body"),
            false,
        ));
        let mut member_ids = vec!["issue_r_0000000001".to_string()];
        member_ids.sort();
        result.regions = vec![Region {
            id: "region_u4_test_0001".to_string(),
            landmark: "contentinfo".to_string(),
            saturation: 0.80,
            structural_count: 1,
            old_node_count: 1,
            member_issue_ids: member_ids,
            severity: IssueSeverity::Warning,
            summary: "contentinfo region: 1/1".to_string(),
        }];
        result.agent_summary.region_count = 1;

        let via_wrapper = render_markdown(&result);
        let via_mode = render_markdown_mode(&result, crate::report::DisclosureMode::Full, "");
        assert_eq!(via_wrapper, via_mode, "render_markdown_mode Full must equal render_markdown wrapper");
    }

    /// Compact mode must contain the required check-m8 substrings.
    #[test]
    fn test_compact_check_m8_substrings() {
        let result = make_fixture();
        let md = render_markdown_mode(&result, crate::report::DisclosureMode::Compact, "");
        assert!(md.contains("# matchy report"), "Missing '# matchy report'");
        assert!(md.contains("## Summary"), "Missing '## Summary'");
        assert!(md.contains("## Scores"), "Missing '## Scores'");
        // render_outline emits "## Issues (table of contents)" which contains "## Issues".
        assert!(md.contains("## Issues"), "Missing '## Issues'");
    }

    /// Compact mode for a fixture with a region must contain the outline ToC and a drill command.
    #[test]
    fn test_compact_mode_has_issues_toc() {
        let mut result = make_fixture();
        result.issues.clear();
        result.clusters.clear();

        // Saturated region.
        let footer_ids: Vec<String> = (0u8..5).map(|i| format!("issue_cmp_{i:016x}")).collect();
        for (i, id) in footer_ids.iter().enumerate() {
            result.issues.push(make_issue(
                id,
                IssueType::ChangedText,
                IssueSeverity::Warning,
                "desktop",
                &format!("footer msg {i}"),
                Some("contentinfo"),
                Some("PRODUCTS"),
                false,
            ));
        }
        let mut member_ids = footer_ids.clone();
        member_ids.sort();
        result.regions = vec![Region {
            id: "region_compact_0001".to_string(),
            landmark: "contentinfo".to_string(),
            saturation: 0.86,
            structural_count: 5,
            old_node_count: 6,
            member_issue_ids: member_ids,
            severity: IssueSeverity::Error,
            summary: "contentinfo region: 5/6".to_string(),
        }];
        result.agent_summary.region_count = 1;

        // A standalone issue in main that must surface in the compact ToC.
        result.issues.push(make_issue(
            "issue_main_compact_01",
            IssueType::BrokenLink,
            IssueSeverity::Error,
            "desktop",
            "standalone link broken",
            Some("main"),
            Some("Body"),
            false,
        ));

        let md = render_markdown_mode(&result, crate::report::DisclosureMode::Compact, "/tmp/out");
        assert!(
            md.contains("## Issues (table of contents)"),
            "Compact mode must contain '## Issues (table of contents)', got: {md}"
        );
        assert!(
            md.contains("matchy show"),
            "Compact mode must contain a matchy show drill command"
        );
    }

    /// Fix 3 / R12: compact mode cluster bullet must include a drill command.
    /// Full mode must NOT include the drill text (wrapper-identity / byte-stability).
    #[test]
    fn test_compact_clusters_have_drill() {
        // Use make_fixture() which already has a cluster.
        let result = make_fixture();

        // Compact mode — cluster bullet must contain "matchy show --cluster".
        let md_compact = render_markdown_mode(&result, crate::report::DisclosureMode::Compact, "/tmp/out");
        assert!(
            md_compact.contains("matchy show --cluster"),
            "Compact mode cluster bullet must contain 'matchy show --cluster', got clusters section: {}",
            md_compact.find("## Clusters").map(|p| &md_compact[p..p.min(md_compact.len()).saturating_add(300)]).unwrap_or("(no ## Clusters found)")
        );

        // Full mode — cluster bullet must NOT contain the drill text.
        let md_full = render_markdown_mode(&result, crate::report::DisclosureMode::Full, "/tmp/out");
        assert!(
            !md_full.contains("matchy show --cluster"),
            "Full mode cluster bullet must NOT contain 'matchy show --cluster' (byte-identity with legacy render)"
        );

        // Verify wrapper identity: render_markdown (Full) must not gain drill text either.
        let md_legacy = render_markdown(&result);
        assert!(
            !md_legacy.contains("matchy show --cluster"),
            "render_markdown (legacy wrapper) must not contain cluster drill text"
        );
    }
}
