//! JSON report assembly and writing (M1.md §3.2, M8.md §4).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Context;
use chrono::Utc;

use crate::contract::{
    AgentSummary, Artifacts, CaptureDeterminism, Cluster, DeterminismSummary, DiffResult, Issue,
    IssueCategory, IntegrityInventory, IssueSeverity, IssueType, LandmarkScores, OutOfScope,
    RunWarning, Scores, Status, StepStatus, Suppressed, ViewportResult,
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

/// Scope options passed to assemble_diff_result.
pub struct ScopeOptions {
    /// Landmark roles to include. Empty = no scoping (include everything).
    pub scope: Vec<String>,
}

impl Default for ScopeOptions {
    fn default() -> Self {
        ScopeOptions { scope: vec![] }
    }
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
    scope_opts: &ScopeOptions,
    extra_warnings: Vec<RunWarning>,
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
    // 2b. Scope: partition kept into in-scope and out-of-scope.
    //
    // An issue with locator.anchors.landmark = Some(l) where l is NOT in scope
    // → out of scope. Issues with landmark = None or in scope → stay.
    // Page-level issues (no landmark) always stay in scope.
    // ------------------------------------------------------------------
    let (kept, out_of_scope) = if scope_opts.scope.is_empty() {
        // No scoping — everything stays, out_of_scope is zero/empty.
        (
            kept,
            OutOfScope {
                count: 0,
                ids: vec![],
            },
        )
    } else {
        let scope_set: BTreeSet<&str> = scope_opts.scope.iter().map(|s| s.as_str()).collect();
        let mut in_scope: Vec<Issue> = Vec::new();
        let mut oos_ids: Vec<String> = Vec::new();
        for issue in kept {
            match &issue.locator.anchors.landmark {
                Some(lm) if !scope_set.contains(lm.as_str()) => {
                    oos_ids.push(issue.id.clone());
                }
                _ => {
                    in_scope.push(issue);
                }
            }
        }
        oos_ids.sort();
        let count = oos_ids.len() as u32;
        (
            in_scope,
            OutOfScope {
                count,
                ids: oos_ids,
            },
        )
    };

    // scoped_to: Some(sorted scope vec) when scope non-empty, else None.
    let scoped_to = if scope_opts.scope.is_empty() {
        None
    } else {
        let mut s = scope_opts.scope.clone();
        s.sort();
        Some(s)
    };

    // ------------------------------------------------------------------
    // 3. Sort kept: descending fix_value, ascending id (tiebreaker).
    // ------------------------------------------------------------------
    let mut kept = kept;
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
    // Filter each viewport's issues to kept (not in baseline, and in scope).
    // Issue id lists in per-viewport results: preserve original order, just filter.
    // Status computed from kept+in-scope severities only.
    // ------------------------------------------------------------------
    // Build a BTreeSet of kept issue ids for fast membership test.
    let kept_ids: BTreeSet<&str> = kept.iter().map(|i| i.id.as_str()).collect();

    let viewport_results: Vec<ViewportResult> = viewports
        .iter()
        .map(|v| {
            let kept_vp: Vec<&Issue> = v
                .issues
                .iter()
                .filter(|i| kept_ids.contains(i.id.as_str()))
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
    // Recompute from the final kept (in-scope, post-baseline-suppression) issues.
    // The per-viewport visual score (ratio-based) is taken from the original; the
    // minimum is then taken across viewports for the top-level value.
    // ------------------------------------------------------------------
    let scores = {
        let kept_refs: Vec<&Issue> = kept.iter().collect();
        // Recompute per-viewport visual scores using the original viewport scores.
        let vp_visual_scores: Vec<f64> = viewports.iter().map(|v| v.scores.visual).collect();
        let min_visual = vp_visual_scores
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min);
        let min_visual = if min_visual == f64::INFINITY {
            1.0
        } else {
            min_visual
        };
        let mut s = crate::compute_scores_from_issues(&kept_refs, min_visual);
        s.by_landmark = compute_by_landmark(&kept_refs);
        s
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
    // 14. Run-level warnings (deterministic order — see brief §4).
    //
    // Order:
    //   1. capture_step_failed (old then new, fixed field order)
    //   2. capture_integrity_delta (old then new)
    //   3. capture_retried_without_time_freeze (old then new)
    //   4. baseline_stale_ids
    //   5. extra_warnings (appended last, e.g. volatile_capture from --self-check)
    // ------------------------------------------------------------------
    let mut warnings = build_warnings(&old_det, &new_det, baseline, &suppressed.ids);
    warnings.extend(extra_warnings);

    // ------------------------------------------------------------------
    // 15. Assemble.
    // ------------------------------------------------------------------
    DiffResult {
        schema_version: "1.2".to_string(),
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
            region_count: 0,
            top_fixes,
        },
        scores,
        viewports: viewport_results,
        issues: kept,
        clusters,
        regions: Vec::new(),
        suppressed,
        warnings,
        scoped_to,
        out_of_scope,
        determinism: DeterminismSummary {
            old: old_det,
            new: new_det,
        },
        artifacts,
    }
}

/// Compute per-landmark scores from kept, in-scope issues.
///
/// Groups issues by `locator.anchors.landmark` (None → key "(none)").
/// Uses the same exclude-Info rule as `compute_scores_from_issues`.
fn compute_by_landmark(kept: &[&Issue]) -> BTreeMap<String, LandmarkScores> {
    // Group by landmark key — BTreeMap for deterministic order.
    let mut groups: BTreeMap<String, Vec<&Issue>> = BTreeMap::new();
    for issue in kept {
        let key = issue
            .locator
            .anchors
            .landmark
            .clone()
            .unwrap_or_else(|| "(none)".to_string());
        groups.entry(key).or_default().push(issue);
    }

    groups
        .into_iter()
        .map(|(landmark, issues)| {
            let non_info = |i: &&Issue| i.severity != IssueSeverity::Info;
            let content_n = issues
                .iter()
                .filter(|i| non_info(i) && i.category == IssueCategory::Content)
                .count();
            let structure_n = issues
                .iter()
                .filter(|i| non_info(i) && i.category == IssueCategory::Structure)
                .count();
            let style_n = issues
                .iter()
                .filter(|i| non_info(i) && i.category == IssueCategory::Style)
                .count();
            let a11y_n = issues
                .iter()
                .filter(|i| non_info(i) && i.issue_type == IssueType::AccessibilityRegression)
                .count();
            let technical_n = issues
                .iter()
                .filter(|i| non_info(i) && i.category == IssueCategory::Technical)
                .count();
            let hygiene_n = issues
                .iter()
                .filter(|i| non_info(i) && i.category == IssueCategory::Hygiene)
                .count();
            (
                landmark,
                LandmarkScores {
                    content: 1.0 / (1.0 + content_n as f64),
                    structure: 1.0 / (1.0 + structure_n as f64),
                    style: 1.0 / (1.0 + style_n as f64),
                    accessibility: 1.0 / (1.0 + a11y_n as f64),
                    technical: 1.0 / (1.0 + technical_n as f64),
                    hygiene: 1.0 / (1.0 + hygiene_n as f64),
                },
            )
        })
        .collect()
}

/// Emit a `capture_integrity_delta` warning when the page inventory changed
/// significantly during stabilization for a given side.
///
/// Fires when:
///  - heading count changed (pre != post), OR
///  - image count changed by more than 20% of pre (pre > 0 and |post - pre| > 0.20 * pre).
fn emit_integrity_delta(
    warnings: &mut Vec<RunWarning>,
    integrity: Option<&IntegrityInventory>,
    side: &str,
) {
    let Some(inv) = integrity else { return };
    let pre = &inv.pre;
    let post = &inv.post;

    let heading_delta = pre.heading_count != post.heading_count;
    let image_delta = pre.image_count > 0 && {
        let delta = (post.image_count as i64 - pre.image_count as i64).unsigned_abs();
        delta > (pre.image_count as f64 * 0.20).floor() as u64
    };

    if !heading_delta && !image_delta {
        return;
    }

    let mut parts: Vec<String> = Vec::new();
    if heading_delta {
        parts.push(format!(
            "headings {}→{}",
            pre.heading_count, post.heading_count
        ));
    }
    if image_delta {
        parts.push(format!("images {}→{}", pre.image_count, post.image_count));
    }

    warnings.push(RunWarning {
        code: "capture_integrity_delta".to_string(),
        message: format!(
            "{} capture: page inventory changed during stabilization ({}); the capture may include stabilizer-induced artifacts",
            side,
            parts.join(", ")
        ),
        context: Some(serde_json::json!({
            "side": side,
            "pre": {
                "headingCount": pre.heading_count,
                "imageCount": pre.image_count,
                "landmarkCount": pre.landmark_count,
            },
            "post": {
                "headingCount": post.heading_count,
                "imageCount": post.image_count,
                "landmarkCount": post.landmark_count,
            },
        })),
    });
}

/// Build the run-level warnings array, in deterministic order per WP-H brief.
///
/// Order:
/// 1. capture_step_failed: old then new, fixed field order.
/// 2. capture_integrity_delta: old then new.
/// 3. capture_retried_without_time_freeze: old then new.
/// 4. baseline_stale_ids.
///
/// (extra_warnings appended by caller after this function returns.)
fn build_warnings(
    old_det: &CaptureDeterminism,
    new_det: &CaptureDeterminism,
    baseline: &crate::baseline::Baseline,
    suppressed_ids: &[String],
) -> Vec<RunWarning> {
    let mut warnings: Vec<RunWarning> = Vec::new();

    // Helper: emit step-failure warnings for one side in fixed field order.
    let mut emit_step_failures = |det: &CaptureDeterminism, side: &str| {
        // Fixed declaration order — must match CaptureDeterminism field order.
        let steps: &[(&str, &StepStatus)] = &[
            ("animationsDisabled", &det.animations_disabled),
            ("reducedMotion", &det.reduced_motion),
            ("timeFrozen", &det.time_frozen),
            ("randomStubbed", &det.random_stubbed),
            ("fontsReady", &det.fonts_ready),
            ("imagesDecoded", &det.images_decoded),
            ("lazyLoadPass", &det.lazy_load_pass),
            ("settled", &det.settled),
        ];
        for (step_name, status) in steps {
            if **status == StepStatus::Failed {
                warnings.push(RunWarning {
                    code: "capture_step_failed".to_string(),
                    message: format!(
                        "{} capture: stabilizer step '{}' failed; the capture may not reflect the page's true state",
                        side, step_name
                    ),
                    context: Some(serde_json::json!({
                        "side": side,
                        "step": step_name
                    })),
                });
            }
        }
    };

    emit_step_failures(old_det, "old");
    emit_step_failures(new_det, "new");

    // capture_integrity_delta: old then new.
    emit_integrity_delta(&mut warnings, old_det.integrity.as_ref(), "old");
    emit_integrity_delta(&mut warnings, new_det.integrity.as_ref(), "new");

    // retried_without_time_freeze: old then new.
    if old_det.retried_without_time_freeze {
        warnings.push(RunWarning {
            code: "capture_retried_without_time_freeze".to_string(),
            message: "old capture: time-freeze broke page scripts; the capture was automatically retried with the clock freeze disabled".to_string(),
            context: Some(serde_json::json!({ "side": "old" })),
        });
    }
    if new_det.retried_without_time_freeze {
        warnings.push(RunWarning {
            code: "capture_retried_without_time_freeze".to_string(),
            message: "new capture: time-freeze broke page scripts; the capture was automatically retried with the clock freeze disabled".to_string(),
            context: Some(serde_json::json!({ "side": "new" })),
        });
    }

    // Baseline staleness: baseline ids that matched no issue in this run.
    // stale = baseline ids minus suppressed ids (i.e. those that did NOT suppress anything).
    if !baseline.is_empty() {
        let suppressed_set: BTreeSet<&str> = suppressed_ids.iter().map(|s| s.as_str()).collect();
        // We need all baseline ids — iterate via baseline.iter_ids() but Baseline doesn't
        // expose that. Use the contains check indirectly: collect from baseline.ids.
        // Since Baseline.ids is private, we reconstruct stale count from what we know.
        // The stale ids are: all ids in baseline that are NOT in suppressed_set.
        // We can get them by iterating baseline ids — but Baseline doesn't expose iter.
        // We'll add a method or use the already-available information.
        // The suppressed_ids are the ones that WERE in baseline and appeared in this run.
        // stale = len(baseline) - len(suppressed_set ∩ baseline).
        // Since suppressed_ids are always in the baseline by construction, stale count =
        // baseline.len() - suppressed_ids.len().
        let suppressed_count = suppressed_ids.len();
        let stale_count = baseline.len().saturating_sub(suppressed_count);
        if stale_count > 0 {
            // We can't enumerate baseline ids without a new method.
            // Add stale_ids as empty here; to get the actual ids we need baseline.iter_ids().
            // The baseline module will expose iter_ids below — we call it.
            let mut stale_ids: Vec<String> = baseline
                .iter_ids()
                .filter(|id| !suppressed_set.contains(id.as_str()))
                .cloned()
                .collect();
            stale_ids.sort();
            let n = stale_ids.len();
            // Take first 20 for context.
            let context_ids: Vec<String> = stale_ids.into_iter().take(20).collect();
            warnings.push(RunWarning {
                code: "baseline_stale_ids".to_string(),
                message: format!(
                    "{} baseline id(s) matched no issue in this run; the accept-list may be stale — regenerate it or switch to durable ids",
                    n
                ),
                context: Some(serde_json::json!({
                    "staleCount": n,
                    "staleIds": context_ids
                })),
            });
        }
    }

    warnings
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
        integrity: None,
    }
}

/// Exposed for tests in other modules that need a default determinism value.
#[cfg(test)]
pub fn make_default_det_for_test() -> CaptureDeterminism {
    make_default_determinism()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::baseline::Baseline;
    use crate::contract::{Anchors, IssueCategory, IssueSeverity, IssueType, Locator};

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn det_all_ran() -> CaptureDeterminism {
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
            integrity: None,
        }
    }

    fn det_step_failed(step: &str) -> CaptureDeterminism {
        let mut d = det_all_ran();
        match step {
            "time_frozen" => d.time_frozen = StepStatus::Failed,
            "fonts_ready" => d.fonts_ready = StepStatus::Failed,
            "settled" => d.settled = StepStatus::Failed,
            _ => panic!("unknown step: {}", step),
        }
        d
    }

    fn make_issue(
        id: &str,
        category: IssueCategory,
        severity: IssueSeverity,
        landmark: Option<&str>,
    ) -> Issue {
        Issue {
            id: id.to_string(),
            issue_type: IssueType::ChangedText,
            category,
            severity,
            confidence: 0.9,
            viewport: "desktop".to_string(),
            locale: None,
            goal: None,
            message: "test issue".to_string(),
            locator: Locator {
                anchors: Anchors {
                    text: Some("some text".to_string()),
                    role: None,
                    href: None,
                    alt: None,
                    aria_label: None,
                    nearest_heading: None,
                    landmark: landmark.map(str::to_string),
                    ordinal_in_landmark: None,
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
        }
    }

    fn empty_artifacts() -> crate::contract::Artifacts {
        crate::contract::Artifacts {
            old: "old.png".to_string(),
            new: "new.png".to_string(),
            diff: "diff.png".to_string(),
        }
    }

    // -----------------------------------------------------------------------
    // WP-E: warnings — capture_step_failed
    // -----------------------------------------------------------------------

    /// A `Failed` step on the old capture must emit a `capture_step_failed` warning
    /// with the correct side and step name.
    #[test]
    fn test_warnings_capture_step_failed_old() {
        let old_det = det_step_failed("time_frozen");
        let new_det = det_all_ran();
        let baseline = Baseline::default();
        let warnings = build_warnings(&old_det, &new_det, &baseline, &[]);
        assert_eq!(warnings.len(), 1, "exactly one warning expected");
        let w = &warnings[0];
        assert_eq!(w.code, "capture_step_failed");
        assert_eq!(
            w.context.as_ref().unwrap()["side"],
            serde_json::json!("old")
        );
        assert_eq!(
            w.context.as_ref().unwrap()["step"],
            serde_json::json!("timeFrozen")
        );
    }

    /// A `Failed` step on the new capture must emit a `capture_step_failed` warning
    /// with side = "new".
    #[test]
    fn test_warnings_capture_step_failed_new() {
        let old_det = det_all_ran();
        let new_det = det_step_failed("settled");
        let baseline = Baseline::default();
        let warnings = build_warnings(&old_det, &new_det, &baseline, &[]);
        assert_eq!(warnings.len(), 1);
        let w = &warnings[0];
        assert_eq!(w.code, "capture_step_failed");
        assert_eq!(
            w.context.as_ref().unwrap()["side"],
            serde_json::json!("new")
        );
        assert_eq!(
            w.context.as_ref().unwrap()["step"],
            serde_json::json!("settled")
        );
    }

    /// No failed steps → no warnings.
    #[test]
    fn test_warnings_none_when_all_ran() {
        let baseline = Baseline::default();
        let warnings = build_warnings(&det_all_ran(), &det_all_ran(), &baseline, &[]);
        assert!(
            warnings.is_empty(),
            "no warnings expected when all steps ran"
        );
    }

    // -----------------------------------------------------------------------
    // WP-E: warnings — baseline staleness
    // -----------------------------------------------------------------------

    /// When baseline contains ids that do not appear in suppressed_ids, a
    /// `baseline_stale_ids` warning must be emitted.
    #[test]
    fn test_warnings_baseline_stale_ids() {
        let baseline = Baseline::from_ids(vec![
            "issue_aaa000000000".to_string(),
            "issue_bbb000000000".to_string(),
        ]);
        // Neither id was suppressed (i.e. neither appeared in this run's issues)
        let warnings = build_warnings(&det_all_ran(), &det_all_ran(), &baseline, &[]);
        assert_eq!(
            warnings.len(),
            1,
            "baseline staleness warning must be emitted"
        );
        let w = &warnings[0];
        assert_eq!(w.code, "baseline_stale_ids");
        let ctx = w.context.as_ref().unwrap();
        assert_eq!(ctx["staleCount"], serde_json::json!(2));
    }

    /// When all baseline ids were suppressed this run, no staleness warning.
    #[test]
    fn test_warnings_no_stale_when_all_suppressed() {
        let baseline = Baseline::from_ids(vec!["issue_aaa000000000".to_string()]);
        let suppressed = vec!["issue_aaa000000000".to_string()];
        let warnings = build_warnings(&det_all_ran(), &det_all_ran(), &baseline, &suppressed);
        // No staleness warning expected
        assert!(
            warnings.iter().all(|w| w.code != "baseline_stale_ids"),
            "no staleness warning when all baseline ids were matched"
        );
    }

    // -----------------------------------------------------------------------
    // WP-E: --scope partition — outOfScope ids
    // -----------------------------------------------------------------------

    /// Issues whose landmark is NOT in the scope list must land in out_of_scope.ids;
    /// page-level issues (landmark = None) always stay.
    #[test]
    fn test_scope_partition_out_of_scope_ids() {
        let scope_opts = ScopeOptions {
            scope: vec!["main".to_string()],
        };
        // One issue inside scope, one out of scope, one page-level (no landmark).
        let vp = ViewportAnalysis {
            name: "desktop".to_string(),
            issues: vec![
                make_issue(
                    "issue_main000001",
                    IssueCategory::Content,
                    IssueSeverity::Error,
                    Some("main"),
                ),
                make_issue(
                    "issue_nav0000001",
                    IssueCategory::Content,
                    IssueSeverity::Error,
                    Some("navigation"),
                ),
                make_issue(
                    "issue_page000001",
                    IssueCategory::Content,
                    IssueSeverity::Error,
                    None,
                ),
            ],
            scores: crate::contract::Scores::all_pass(),
            artifacts: empty_artifacts(),
            old_det: det_all_ran(),
            new_det: det_all_ran(),
        };
        let result = assemble_diff_result(
            "run-test",
            "http://old.com/",
            "http://new.com/",
            &crate::scoring::ParityProfile::ContentStructure,
            vec![vp],
            &Baseline::default(),
            &scope_opts,
            vec![],
        );
        assert_eq!(
            result.out_of_scope.count, 1,
            "exactly one out-of-scope issue"
        );
        assert!(result
            .out_of_scope
            .ids
            .contains(&"issue_nav0000001".to_string()));
        // scoped_to must record the requested scope
        assert_eq!(result.scoped_to, Some(vec!["main".to_string()]));
        // In-scope issues: main + page-level
        let kept_ids: Vec<&str> = result.issues.iter().map(|i| i.id.as_str()).collect();
        assert!(
            kept_ids.contains(&"issue_main000001"),
            "main-landmark issue must stay"
        );
        assert!(
            kept_ids.contains(&"issue_page000001"),
            "page-level issue must stay"
        );
        assert!(
            !kept_ids.contains(&"issue_nav0000001"),
            "out-of-scope issue must not be in kept"
        );
    }

    /// When scope is empty, all issues stay and out_of_scope is zero.
    #[test]
    fn test_scope_empty_keeps_all() {
        let vp = ViewportAnalysis {
            name: "desktop".to_string(),
            issues: vec![make_issue(
                "issue_nav0000001",
                IssueCategory::Content,
                IssueSeverity::Error,
                Some("navigation"),
            )],
            scores: crate::contract::Scores::all_pass(),
            artifacts: empty_artifacts(),
            old_det: det_all_ran(),
            new_det: det_all_ran(),
        };
        let result = assemble_diff_result(
            "run-test",
            "http://old.com/",
            "http://new.com/",
            &crate::scoring::ParityProfile::ContentStructure,
            vec![vp],
            &Baseline::default(),
            &ScopeOptions::default(),
            vec![],
        );
        assert_eq!(result.out_of_scope.count, 0);
        assert!(result.out_of_scope.ids.is_empty());
        assert_eq!(result.scoped_to, None);
    }

    // -----------------------------------------------------------------------
    // WP-E: scores.byLandmark
    // -----------------------------------------------------------------------

    /// byLandmark must group issues by landmark, compute per-category scores,
    /// and use "(none)" for issues without a landmark.
    #[test]
    fn test_by_landmark_grouping() {
        // Call compute_by_landmark directly via assemble_diff_result result.
        let vp = ViewportAnalysis {
            name: "desktop".to_string(),
            issues: vec![
                // 2 content errors in "main" → content score = 1/3
                make_issue(
                    "issue_m_c1_00001",
                    IssueCategory::Content,
                    IssueSeverity::Error,
                    Some("main"),
                ),
                make_issue(
                    "issue_m_c2_00001",
                    IssueCategory::Content,
                    IssueSeverity::Error,
                    Some("main"),
                ),
                // 1 style warning in "navigation" → style score = 1/2
                make_issue(
                    "issue_n_s1_00001",
                    IssueCategory::Style,
                    IssueSeverity::Warning,
                    Some("navigation"),
                ),
                // 1 info issue in "main" — must NOT count toward scores
                make_issue(
                    "issue_m_i1_00001",
                    IssueCategory::Content,
                    IssueSeverity::Info,
                    Some("main"),
                ),
            ],
            scores: crate::contract::Scores::all_pass(),
            artifacts: empty_artifacts(),
            old_det: det_all_ran(),
            new_det: det_all_ran(),
        };
        let result = assemble_diff_result(
            "run-test",
            "http://old.com/",
            "http://new.com/",
            &crate::scoring::ParityProfile::ContentStructure,
            vec![vp],
            &Baseline::default(),
            &ScopeOptions::default(),
            vec![],
        );
        let by_lm = &result.scores.by_landmark;
        // "main" must appear
        let main = by_lm
            .get("main")
            .expect("'main' key must exist in by_landmark");
        // 2 non-info content errors → content = 1/(1+2) = 0.333...
        assert!(
            (main.content - 1.0 / 3.0).abs() < 1e-9,
            "main content score must be 1/3, got {}",
            main.content
        );
        // Info issue must NOT count → still 2 non-info content issues
        // structure, style, accessibility, technical, hygiene must all be 1.0 for "main"
        assert_eq!(main.structure, 1.0);
        assert_eq!(main.style, 1.0);

        // "navigation" must appear
        let nav = by_lm
            .get("navigation")
            .expect("'navigation' key must exist in by_landmark");
        // 1 style warning → style = 1/(1+1) = 0.5
        assert!(
            (nav.style - 0.5).abs() < 1e-9,
            "navigation style score must be 0.5, got {}",
            nav.style
        );
        assert_eq!(nav.content, 1.0);
    }

    /// Info-only issues for a landmark still produce a landmark entry with all scores = 1.0.
    #[test]
    fn test_by_landmark_info_only_scores_all_pass() {
        let vp = ViewportAnalysis {
            name: "desktop".to_string(),
            issues: vec![make_issue(
                "issue_x_i1_00001",
                IssueCategory::Content,
                IssueSeverity::Info,
                Some("aside"),
            )],
            scores: crate::contract::Scores::all_pass(),
            artifacts: empty_artifacts(),
            old_det: det_all_ran(),
            new_det: det_all_ran(),
        };
        let result = assemble_diff_result(
            "run-test",
            "http://old.com/",
            "http://new.com/",
            &crate::scoring::ParityProfile::ContentStructure,
            vec![vp],
            &Baseline::default(),
            &ScopeOptions::default(),
            vec![],
        );
        let by_lm = &result.scores.by_landmark;
        let aside = by_lm
            .get("aside")
            .expect("'aside' must appear even if info-only");
        assert_eq!(
            aside.content, 1.0,
            "info-only issues must not reduce content score"
        );
        assert_eq!(aside.style, 1.0);
    }

    // -----------------------------------------------------------------------
    // WP-H: capture_integrity_delta warnings
    // -----------------------------------------------------------------------

    use crate::contract::{IntegrityCounts, IntegrityInventory};

    fn make_integrity(
        pre_h: u32,
        pre_i: u32,
        pre_l: u32,
        post_h: u32,
        post_i: u32,
        post_l: u32,
    ) -> IntegrityInventory {
        IntegrityInventory {
            pre: IntegrityCounts {
                heading_count: pre_h,
                image_count: pre_i,
                landmark_count: pre_l,
            },
            post: IntegrityCounts {
                heading_count: post_h,
                image_count: post_i,
                landmark_count: post_l,
            },
        }
    }

    fn det_with_integrity(inv: IntegrityInventory) -> CaptureDeterminism {
        CaptureDeterminism {
            integrity: Some(inv),
            ..det_all_ran()
        }
    }

    /// Heading count changed → integrity warning fires.
    #[test]
    fn test_integrity_delta_heading_fires() {
        let det = det_with_integrity(make_integrity(12, 20, 5, 9, 20, 5));
        let baseline = Baseline::default();
        let warnings = build_warnings(&det, &det_all_ran(), &baseline, &[]);
        let integrity_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.code == "capture_integrity_delta")
            .collect();
        assert_eq!(integrity_warnings.len(), 1, "heading delta should fire once");
        let ctx = integrity_warnings[0].context.as_ref().unwrap();
        assert_eq!(ctx["side"], serde_json::json!("old"));
        assert!(
            integrity_warnings[0].message.contains("headings 12→9"),
            "message should contain heading delta"
        );
    }

    /// Image count changed by 25% (> 20%) → fires.
    #[test]
    fn test_integrity_delta_image_25pct_fires() {
        // pre=20, post=25 → delta=5, 5/20=25% > 20%
        let det = det_with_integrity(make_integrity(5, 20, 3, 5, 25, 3));
        let baseline = Baseline::default();
        let warnings = build_warnings(&det_all_ran(), &det, &baseline, &[]);
        let integrity_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.code == "capture_integrity_delta")
            .collect();
        assert_eq!(integrity_warnings.len(), 1);
        let ctx = integrity_warnings[0].context.as_ref().unwrap();
        assert_eq!(ctx["side"], serde_json::json!("new"));
    }

    /// Image count changed by 10% (≤ 20%) → does NOT fire.
    #[test]
    fn test_integrity_delta_image_10pct_no_fire() {
        // pre=20, post=22 → delta=2, 2/20=10% ≤ 20%
        let det = det_with_integrity(make_integrity(5, 20, 3, 5, 22, 3));
        let baseline = Baseline::default();
        let warnings = build_warnings(&det, &det_all_ran(), &baseline, &[]);
        let integrity_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.code == "capture_integrity_delta")
            .collect();
        assert!(
            integrity_warnings.is_empty(),
            "10% image delta must not fire"
        );
    }

    /// Absent integrity → no integrity warning.
    #[test]
    fn test_integrity_absent_no_warning() {
        let baseline = Baseline::default();
        let warnings = build_warnings(&det_all_ran(), &det_all_ran(), &baseline, &[]);
        assert!(
            warnings
                .iter()
                .all(|w| w.code != "capture_integrity_delta"),
            "no integrity warning when integrity is None"
        );
    }

    /// Ordering test: step_failed before integrity_delta before retried.
    #[test]
    fn test_warnings_ordering_step_before_integrity_before_retried() {
        let mut det = det_with_integrity(make_integrity(12, 20, 5, 9, 20, 5));
        det.lazy_load_pass = StepStatus::Failed;
        det.retried_without_time_freeze = true;

        let baseline = Baseline::default();
        let warnings = build_warnings(&det, &det_all_ran(), &baseline, &[]);

        // Collect codes in order.
        let codes: Vec<&str> = warnings.iter().map(|w| w.code.as_str()).collect();

        // step_failed must appear before integrity_delta.
        let step_pos = codes.iter().position(|&c| c == "capture_step_failed");
        let integrity_pos = codes.iter().position(|&c| c == "capture_integrity_delta");
        let retried_pos = codes.iter().position(|&c| c == "capture_retried_without_time_freeze");

        assert!(step_pos.is_some(), "step_failed warning must be present");
        assert!(integrity_pos.is_some(), "integrity_delta warning must be present");
        assert!(retried_pos.is_some(), "retried warning must be present");

        assert!(
            step_pos.unwrap() < integrity_pos.unwrap(),
            "capture_step_failed must come before capture_integrity_delta"
        );
        assert!(
            integrity_pos.unwrap() < retried_pos.unwrap(),
            "capture_integrity_delta must come before capture_retried_without_time_freeze"
        );
    }

    /// Full ordering: step_failed < integrity_delta < retried < baseline_stale_ids.
    #[test]
    fn test_warnings_ordering_includes_baseline_stale() {
        let mut det = det_with_integrity(make_integrity(12, 20, 5, 9, 20, 5));
        det.lazy_load_pass = StepStatus::Failed;
        det.retried_without_time_freeze = true;

        // Baseline with a stale id so baseline_stale_ids fires.
        let baseline = Baseline::from_ids(vec!["issue_stale00001".to_string()]);
        let warnings = build_warnings(&det, &det_all_ran(), &baseline, &[]);

        let codes: Vec<&str> = warnings.iter().map(|w| w.code.as_str()).collect();

        let step_pos = codes.iter().position(|&c| c == "capture_step_failed").unwrap();
        let integrity_pos = codes.iter().position(|&c| c == "capture_integrity_delta").unwrap();
        let retried_pos = codes.iter().position(|&c| c == "capture_retried_without_time_freeze").unwrap();
        let stale_pos = codes.iter().position(|&c| c == "baseline_stale_ids").unwrap();

        assert!(step_pos < integrity_pos, "step_failed must precede integrity_delta");
        assert!(integrity_pos < retried_pos, "integrity_delta must precede retried");
        assert!(retried_pos < stale_pos, "retried must precede baseline_stale_ids");
    }

    /// extra_warnings are appended after all generated warnings.
    #[test]
    fn test_extra_warnings_appended_last() {
        let extra = vec![RunWarning {
            code: "volatile_capture".to_string(),
            message: "self-check found issues".to_string(),
            context: None,
        }];
        let vp = ViewportAnalysis {
            name: "desktop".to_string(),
            issues: vec![],
            scores: crate::contract::Scores::all_pass(),
            artifacts: empty_artifacts(),
            old_det: det_all_ran(),
            new_det: det_all_ran(),
        };
        let result = assemble_diff_result(
            "run-test",
            "http://old.com/",
            "http://new.com/",
            &crate::scoring::ParityProfile::ContentStructure,
            vec![vp],
            &Baseline::default(),
            &ScopeOptions::default(),
            extra,
        );
        // The last warning should be volatile_capture.
        let last = result.warnings.last();
        assert!(last.is_some(), "warnings must not be empty");
        assert_eq!(
            last.unwrap().code,
            "volatile_capture",
            "volatile_capture must be last"
        );
    }
}
