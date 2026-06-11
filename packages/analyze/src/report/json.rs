//! JSON report assembly and writing (M1.md §3.2, M8.md §4).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Context;
use chrono::Utc;

use crate::contract::{
    AgentSummary, Artifacts, CaptureDeterminism, Cluster, DeterminismSummary, DiffResult, Issue,
    IssueCategory, IssueSeverity, IssueType, Scores, Status, Suppressed, ViewportResult,
};
use crate::scoring::{compute_status, count_fixable_now, fix_value, ParityProfile};

/// Per-viewport analysis inputs.
pub struct ViewportAnalysis {
    pub name: String,
    pub issues: Vec<Issue>,
    pub scores: Scores,
    pub artifacts: Artifacts,
    pub old_det: CaptureDeterminism,
    pub new_det: CaptureDeterminism,
}

/// Assemble a DiffResult from per-viewport analyses.
///
/// DETERMINISM:
/// - issues merged via flat_map (viewport order stable), then sorted by descending
///   fix_value, tie-break ascending id.
/// - Suppression: partition kept vs suppressed by baseline id set; suppressed.ids sorted asc.
/// - Clustering: BTreeMap-based grouping; member ids sorted; final array (count DESC, id ASC).
/// - topFixes: cluster id for clustered groups, issue id for unclustered; sorted by
///   (fv DESC, id ASC); take 5.
/// - byType uses BTreeMap.
/// - Multi-viewport: scores = min per category; determinism = worst per step.
/// - No HashMap or serde_json::Map iteration for ordering anywhere.
pub fn assemble_diff_result(
    run_id: &str,
    old_url: &str,
    new_url: &str,
    profile: &ParityProfile,
    viewports: Vec<ViewportAnalysis>,
    baseline: &crate::baseline::Baseline,
) -> DiffResult {
    // ------------------------------------------------------------------
    // 1. Merge issues from all viewports.
    // ------------------------------------------------------------------
    let all_issues: Vec<Issue> = viewports.iter().flat_map(|v| v.issues.clone()).collect();

    // ------------------------------------------------------------------
    // 2. Suppress: partition into kept and suppressed.
    // ------------------------------------------------------------------
    let mut kept: Vec<Issue> = Vec::new();
    let mut suppressed_issues: Vec<Issue> = Vec::new();
    for issue in all_issues {
        if baseline.contains(&issue.id) {
            suppressed_issues.push(issue);
        } else {
            kept.push(issue);
        }
    }
    let mut suppressed_ids: Vec<String> = suppressed_issues.iter().map(|i| i.id.clone()).collect();
    suppressed_ids.sort();
    let suppressed = Suppressed {
        count: suppressed_ids.len() as u32,
        ids: suppressed_ids,
    };

    // ------------------------------------------------------------------
    // 3. Sort kept: descending fix_value, ascending id (tiebreaker).
    // ------------------------------------------------------------------
    kept.sort_by(|a, b| {
        let fv_a = fix_value(&a.severity, a.confidence, &a.locator.anchors.strength());
        let fv_b = fix_value(&b.severity, b.confidence, &b.locator.anchors.strength());
        fv_b.partial_cmp(&fv_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });

    // ------------------------------------------------------------------
    // 4. Cluster kept issues.
    // ------------------------------------------------------------------
    let clusters: Vec<Cluster> =
        crate::clustering::cluster_issues(&kept, crate::config::CLUSTER_MIN);

    // Build a set of all issue ids that are members of any cluster.
    let clustered_ids: BTreeSet<&str> = clusters
        .iter()
        .flat_map(|c| c.issue_ids.iter().map(|s| s.as_str()))
        .collect();

    // (id_to_cluster is available for future use; cluster membership tested via clustered_ids)

    // ------------------------------------------------------------------
    // 5. byType over kept.
    // ------------------------------------------------------------------
    let mut by_type: BTreeMap<String, u32> = BTreeMap::new();
    for issue in &kept {
        *by_type
            .entry(issue.issue_type.as_str().to_string())
            .or_insert(0) += 1;
    }

    // ------------------------------------------------------------------
    // 6. fixable_now over kept.
    // ------------------------------------------------------------------
    let fixable_now = count_fixable_now(&kept);

    // ------------------------------------------------------------------
    // 7. topFixes: cluster-aware work queue.
    //
    // For clustered issues: one entry per cluster with fv = max over member
    // issues (iterated in sorted member order for determinism).
    // For unclustered: one entry per issue.
    // Sort: (fv DESC, id ASC). Take first 5.
    // ------------------------------------------------------------------

    // Build id → &Issue lookup for member fv computation (BTreeMap for determinism).
    let id_to_issue: BTreeMap<&str, &Issue> = kept.iter().map(|i| (i.id.as_str(), i)).collect();

    // Work queue entries: (id, fix_value)
    let mut work_queue: Vec<(String, f64)> = Vec::new();

    // One entry per cluster: max fv over members (members already sorted ascending in cluster).
    for cluster in &clusters {
        let max_fv = cluster
            .issue_ids
            .iter()
            .filter_map(|mid| id_to_issue.get(mid.as_str()))
            .map(|issue| {
                fix_value(
                    &issue.severity,
                    issue.confidence,
                    &issue.locator.anchors.strength(),
                )
            })
            .fold(f64::NEG_INFINITY, f64::max);
        work_queue.push((cluster.id.clone(), max_fv));
    }

    // One entry per unclustered kept issue.
    for issue in &kept {
        if !clustered_ids.contains(issue.id.as_str()) {
            let fv = fix_value(
                &issue.severity,
                issue.confidence,
                &issue.locator.anchors.strength(),
            );
            work_queue.push((issue.id.clone(), fv));
        }
    }

    // Sort: (fv DESC, id ASC) — total order.
    work_queue.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    let top_fixes: Vec<String> = work_queue.into_iter().take(5).map(|(id, _)| id).collect();

    // ------------------------------------------------------------------
    // 8. cluster_count.
    // ------------------------------------------------------------------
    let cluster_count = clusters.len() as u32;

    // ------------------------------------------------------------------
    // 9. Per-viewport results.
    //
    // Filter each viewport's issues to kept (not in baseline).
    // Issue id lists in per-viewport results: preserve original order, just filter.
    // Status computed from kept severities only.
    // ------------------------------------------------------------------
    let viewport_results: Vec<ViewportResult> = viewports
        .iter()
        .map(|v| {
            let kept_vp: Vec<&Issue> = v
                .issues
                .iter()
                .filter(|i| !baseline.contains(&i.id))
                .collect();
            let sev: Vec<IssueSeverity> = kept_vp.iter().map(|i| i.severity.clone()).collect();
            let vp_status = compute_status(&sev);
            ViewportResult {
                name: v.name.clone(),
                status: vp_status,
                issues: kept_vp.iter().map(|i| i.id.clone()).collect(),
                artifacts: v.artifacts.clone(),
            }
        })
        .collect();

    // ------------------------------------------------------------------
    // 10. Overall status = worst across viewport statuses.
    // ------------------------------------------------------------------
    let overall_status = viewport_results
        .iter()
        .map(|v| v.status.clone())
        .fold(Status::Pass, Status::worst);

    // ------------------------------------------------------------------
    // 11. Scores.
    //
    // Empty baseline: min_per_category of the passed-in per-viewport scores (unchanged).
    // Non-empty baseline: recompute count-based scores per viewport from kept_vp issues,
    // keep visual from original; then min_per_category across viewports.
    // ------------------------------------------------------------------
    let scores = if baseline.is_empty() {
        let all_scores: Vec<Scores> = viewports.iter().map(|v| v.scores.clone()).collect();
        Scores::min_per_category(&all_scores)
    } else {
        let recomputed: Vec<Scores> = viewports
            .iter()
            .map(|v| {
                let kept_vp: Vec<&Issue> = v
                    .issues
                    .iter()
                    .filter(|i| !baseline.contains(&i.id))
                    .collect();
                recompute_scores(&kept_vp, &v.scores)
            })
            .collect();
        Scores::min_per_category(&recomputed)
    };

    // ------------------------------------------------------------------
    // 12. Determinism: worst per step across viewports for old and new.
    // ------------------------------------------------------------------
    let old_det = viewports
        .iter()
        .map(|v| v.old_det.clone())
        .reduce(|a, b| CaptureDeterminism::merge_worst(&a, &b))
        .unwrap_or_else(make_default_determinism);

    let new_det = viewports
        .iter()
        .map(|v| v.new_det.clone())
        .reduce(|a, b| CaptureDeterminism::merge_worst(&a, &b))
        .unwrap_or_else(make_default_determinism);

    // ------------------------------------------------------------------
    // 13. Artifacts: first viewport's artifacts.
    // ------------------------------------------------------------------
    let artifacts = viewports
        .first()
        .map(|v| v.artifacts.clone())
        .unwrap_or(Artifacts {
            old: "".to_string(),
            new: "".to_string(),
            diff: "".to_string(),
        });

    // ------------------------------------------------------------------
    // 14. Assemble.
    // ------------------------------------------------------------------
    DiffResult {
        schema_version: "1.0".to_string(),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        run_id: run_id.to_string(),
        old_url: old_url.to_string(),
        new_url: new_url.to_string(),
        parity_profile: profile.as_str().to_string(),
        status: overall_status,
        agent_summary: AgentSummary {
            fixable_now,
            by_type,
            cluster_count,
            top_fixes,
        },
        scores,
        viewports: viewport_results,
        issues: kept,
        clusters,
        suppressed,
        determinism: DeterminismSummary {
            old: old_det,
            new: new_det,
        },
        artifacts,
    }
}

/// Recompute count-based category scores from kept issues; keep visual (ratio-based) from original.
///
/// Mirrors analyze_viewport formulas so that suppression doesn't inflate scores beyond what
/// the remaining kept issues warrant.
fn recompute_scores(kept_vp: &[&Issue], original: &Scores) -> Scores {
    let content_n = kept_vp
        .iter()
        .filter(|i| i.category == IssueCategory::Content)
        .count();
    let structure_n = kept_vp
        .iter()
        .filter(|i| i.category == IssueCategory::Structure)
        .count();
    let style_n = kept_vp
        .iter()
        .filter(|i| i.category == IssueCategory::Style)
        .count();
    let a11y_n = kept_vp
        .iter()
        .filter(|i| i.issue_type == IssueType::AccessibilityRegression)
        .count();
    let technical_n = kept_vp
        .iter()
        .filter(|i| i.category == IssueCategory::Technical)
        .count();
    let hygiene_n = kept_vp
        .iter()
        .filter(|i| i.category == IssueCategory::Hygiene)
        .count();

    Scores {
        visual: original.visual, // ratio-based, not derivable from issue list
        content: 1.0 / (1.0 + content_n as f64),
        structure: 1.0 / (1.0 + structure_n as f64),
        style: 1.0 / (1.0 + style_n as f64),
        accessibility: 1.0 / (1.0 + a11y_n as f64),
        technical: 1.0 / (1.0 + technical_n as f64),
        hygiene: 1.0 / (1.0 + hygiene_n as f64),
    }
}

/// Write DiffResult as pretty JSON (with trailing newline) to out_dir/diff-result.json.
pub fn write_diff_result(result: &DiffResult, out_dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create output dir: {}", out_dir.display()))?;
    let json = result.to_json()?;
    let path = out_dir.join("diff-result.json");
    std::fs::write(&path, &json)
        .with_context(|| format!("failed to write diff-result.json: {}", path.display()))?;
    Ok(())
}

/// Generate a run_id from current UTC time.
pub fn make_run_id() -> String {
    Utc::now().format("%Y-%m-%dT%H-%M-%SZ").to_string()
}

fn make_default_determinism() -> CaptureDeterminism {
    use crate::contract::StepStatus;
    CaptureDeterminism {
        animations_disabled: StepStatus::Skipped,
        reduced_motion: StepStatus::Skipped,
        time_frozen: StepStatus::Skipped,
        random_stubbed: StepStatus::Skipped,
        fonts_ready: StepStatus::Skipped,
        images_decoded: StepStatus::Skipped,
        lazy_load_pass: StepStatus::Skipped,
        settled: StepStatus::Skipped,
        clicked: vec![],
        hidden: vec![],
        masked: vec![],
        retried_without_time_freeze: false,
    }
}

/// Exposed for tests in other modules that need a default determinism value.
#[cfg(test)]
pub fn make_default_det_for_test() -> CaptureDeterminism {
    make_default_determinism()
}
