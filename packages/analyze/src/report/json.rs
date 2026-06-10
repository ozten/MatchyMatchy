//! JSON report assembly and writing (M1.md §3.2).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;
use chrono::Utc;

use crate::contract::{
    AgentSummary, Artifacts, CaptureDeterminism, Cluster, DeterminismSummary, DiffResult, Issue,
    IssueSeverity, Scores, Status, Suppressed, ViewportResult,
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
/// - issues sorted by descending fix_value, tie-break ascending id
/// - byType uses BTreeMap
/// - topFixes first 5 in sorted order
/// - multi-viewport: scores = min per category; determinism = worst per step
pub fn assemble_diff_result(
    run_id: &str,
    old_url: &str,
    new_url: &str,
    profile: &ParityProfile,
    viewports: Vec<ViewportAnalysis>,
) -> DiffResult {
    // Merge issues from all viewports (already assigned their viewport field).
    let mut all_issues: Vec<Issue> = viewports.iter().flat_map(|v| v.issues.clone()).collect();

    // Sort issues: descending fix_value, ascending id as tiebreaker.
    // Fix value computation is deterministic (no map iteration).
    all_issues.sort_by(|a, b| {
        let fv_a = fix_value(&a.severity, a.confidence, &a.locator.anchors.strength());
        let fv_b = fix_value(&b.severity, b.confidence, &b.locator.anchors.strength());
        fv_b.partial_cmp(&fv_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });

    // Build byType count (BTreeMap for determinism).
    let mut by_type: BTreeMap<String, u32> = BTreeMap::new();
    for issue in &all_issues {
        *by_type
            .entry(issue.issue_type.as_str().to_string())
            .or_insert(0) += 1;
    }

    // topFixes: first 5 issue ids in sorted order.
    let top_fixes: Vec<String> = all_issues.iter().take(5).map(|i| i.id.clone()).collect();

    let fixable_now = count_fixable_now(&all_issues);

    // Per-viewport status.
    let viewport_results: Vec<ViewportResult> = viewports
        .iter()
        .map(|v| {
            let sev: Vec<IssueSeverity> = v.issues.iter().map(|i| i.severity.clone()).collect();
            let vp_status = compute_status(&sev);
            ViewportResult {
                name: v.name.clone(),
                status: vp_status,
                issues: v.issues.iter().map(|i| i.id.clone()).collect(),
                artifacts: v.artifacts.clone(),
            }
        })
        .collect();

    // Overall status: worst across viewports.
    let overall_status = viewport_results
        .iter()
        .map(|v| v.status.clone())
        .fold(Status::Pass, Status::worst);

    // Scores: min per category across viewports.
    let all_scores: Vec<Scores> = viewports.iter().map(|v| v.scores.clone()).collect();
    let scores = Scores::min_per_category(&all_scores);

    // Determinism: worst per step across viewports for old and new sides.
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

    // Top-level artifacts = first viewport's artifacts.
    let artifacts = viewports
        .first()
        .map(|v| v.artifacts.clone())
        .unwrap_or(Artifacts {
            old: "".to_string(),
            new: "".to_string(),
            diff: "".to_string(),
        });

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
            cluster_count: 0,
            top_fixes,
        },
        scores,
        viewports: viewport_results,
        issues: all_issues,
        clusters: Vec::<Cluster>::new(),
        suppressed: Suppressed {
            count: 0,
            ids: vec![],
        },
        determinism: DeterminismSummary {
            old: old_det,
            new: new_det,
        },
        artifacts,
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
