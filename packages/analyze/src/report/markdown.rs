//! GitHub-flavoured Markdown report renderer (M8 §6).

use std::path::Path;

use anyhow::Context;

use crate::contract::DiffResult;

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
// Public API
// ---------------------------------------------------------------------------

/// Render a DiffResult into a GitHub-flavoured Markdown string.
/// Pure function, deterministic, no filesystem access.
pub fn render_markdown(result: &DiffResult) -> String {
    let mut out = String::with_capacity(16 * 1024);

    let status_str = match &result.status {
        crate::contract::Status::Pass => "pass",
        crate::contract::Status::Warn => "warn",
        crate::contract::Status::Fail => "fail",
        crate::contract::Status::Error => "error",
    };

    // ------------------------------------------------------------------
    // Header
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
    // Summary
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

    if !result.agent_summary.top_fixes.is_empty() {
        let top = result.agent_summary.top_fixes.join(", ");
        out.push_str(&format!("- **Top fixes:** {}\n", md_cell(&top)));
    }

    if !result.agent_summary.by_type.is_empty() {
        out.push_str("\n**By type:**\n\n");
        // BTreeMap iterates in sorted key order — deterministic.
        for (type_str, count) in &result.agent_summary.by_type {
            out.push_str(&format!("- {}: {count}\n", md_cell(type_str)));
        }
    }
    out.push('\n');

    // ------------------------------------------------------------------
    // Scores
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

    // ------------------------------------------------------------------
    // Issues (in result.issues order = fix-value order)
    // ------------------------------------------------------------------
    out.push_str("## Issues\n\n");
    if result.issues.is_empty() {
        out.push_str("No issues.\n\n");
    } else {
        out.push_str("| # | Type | Severity | Goal | Message |\n");
        out.push_str("|---|---|---|---|---|\n");
        for (i, issue) in result.issues.iter().enumerate() {
            let sev_str = match &issue.severity {
                crate::contract::IssueSeverity::Info => "info",
                crate::contract::IssueSeverity::Warning => "warning",
                crate::contract::IssueSeverity::Error => "error",
                crate::contract::IssueSeverity::Critical => "critical",
            };
            let goal = issue.goal.as_deref().map(md_cell).unwrap_or_default();
            let msg = md_cell(&issue.message);
            out.push_str(&format!(
                "| {} | {} | {sev_str} | {goal} | {msg} |\n",
                i + 1,
                md_cell(issue.issue_type.as_str()),
            ));
        }
        out.push('\n');

        // Remediation grep targets as a bullet list
        let has_rem = result.issues.iter().any(|i| {
            i.remediation
                .as_ref()
                .map(has_grep_targets)
                .unwrap_or(false)
        });
        if has_rem {
            out.push_str("**Remediation grep targets:**\n\n");
            for issue in &result.issues {
                if let Some(rem) = &issue.remediation {
                    if let Some(targets) = rem.get("grepTargets").and_then(|v| v.as_array()) {
                        if !targets.is_empty() {
                            let targets_str: Vec<String> = targets
                                .iter()
                                .filter_map(|t| t.as_str())
                                .map(|t| format!("`{}`", md_cell(t)))
                                .collect();
                            out.push_str(&format!(
                                "- {}: grep {}\n",
                                md_cell(&issue.id),
                                targets_str.join(", ")
                            ));
                        }
                    }
                }
            }
            out.push('\n');
        }
    }

    // ------------------------------------------------------------------
    // Clusters
    // ------------------------------------------------------------------
    if !result.clusters.is_empty() {
        out.push_str("## Clusters\n\n");
        for cluster in &result.clusters {
            let summary = cluster
                .summary
                .as_deref()
                .map(md_cell)
                .unwrap_or_else(|| cluster.id.clone());
            out.push_str(&format!(
                "- {} (members: {})\n",
                summary,
                cluster.issue_ids.len()
            ));
        }
        out.push('\n');
    }

    // ------------------------------------------------------------------
    // Suppressed
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

fn has_grep_targets(rem: &serde_json::Value) -> bool {
    rem.get("grepTargets")
        .and_then(|v| v.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false)
}

/// Write the Markdown report to `out_dir/report.md` (creates the directory if needed).
pub fn write_markdown(result: &DiffResult, out_dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create output dir: {}", out_dir.display()))?;
    let md = render_markdown(result);
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
        IssueCategory, IssueSeverity, IssueType, Locator, Scores, Status, Suppressed,
        ViewportResult,
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
                anchors: Anchors::null(),
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
            schema_version: "1.0".to_string(),
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
            suppressed: Suppressed {
                count: 2,
                ids: vec![
                    "issue_dead000000ff".to_string(),
                    "issue_dead000001ff".to_string(),
                ],
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

    #[test]
    fn test_section_headers_present() {
        let result = make_fixture();
        let md = render_markdown(&result);
        assert!(md.contains("# matchy report"), "Missing '# matchy report'");
        assert!(md.contains("## Summary"), "Missing '## Summary'");
        assert!(md.contains("## Scores"), "Missing '## Scores'");
        assert!(md.contains("## Issues"), "Missing '## Issues'");
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
        // Issue message has "|" and "\n" which should be escaped in table cells
        let result = make_fixture();
        let md = render_markdown(&result);
        // The raw pipe should not appear unescaped in table context
        // The message "Text changed: hello|world\nand more" should be cell-escaped
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
        write_markdown(&result, &tmp).expect("write_markdown should succeed");
        let path = tmp.join("report.md");
        assert!(path.exists(), "report.md should be created");
        // cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
