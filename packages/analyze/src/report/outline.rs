//! Compact progressive-disclosure projection over a complete DiffResult
//! (R1-R5, R9, R13). Pure & deterministic. This is the SINGLE canonical
//! collapsible-branch enumeration + handle derivation + collapsed-pointer
//! templates, reused by the markdown compact view (U4), the HTML <details open>
//! decision (U4), and the `matchy show` drill resolver (U5). One enumeration
//! drives every surface (R12).

use std::collections::{BTreeMap, HashMap};

use crate::contract::{DiffResult, Issue, IssueSeverity};
use crate::scoring::fix_value;

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn is_uncertain_pairing(evidence: &serde_json::Value) -> bool {
    evidence
        .get("match")
        .and_then(|m| m.get("uncertainPairing"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Severity label for display in collapsed-pointer lines.
fn sev_label(sev: &IssueSeverity) -> &'static str {
    match sev {
        IssueSeverity::Info => "info",
        IssueSeverity::Warning => "warning",
        IssueSeverity::Error => "error",
        IssueSeverity::Critical => "critical",
    }
}

/// Markdown table-cell sanitizer: replace `|` → `\|`, newlines → space, trim.
fn md_cell(s: &str) -> String {
    s.replace('|', "\\|")
        .replace(['\r', '\n'], " ")
        .trim()
        .to_string()
}

/// Return the worst of two severities (by rank).
fn worst_sev<'a>(a: &'a IssueSeverity, b: &'a IssueSeverity) -> &'a IssueSeverity {
    if b.rank() > a.rank() {
        b
    } else {
        a
    }
}

fn quote_landmark(s: &str) -> String {
    if s.chars().any(|c| c.is_whitespace()) {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn quote_heading(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\\\""))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// A stable, ordinal-independent drill handle for one collapsible branch (R5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchHandle {
    Region { landmark: String },
    Section { landmark: String, heading: Option<String> },
    Cluster { id: String },
    Issue { id: String },
}

impl BranchHandle {
    /// Copy-pasteable `matchy show …` command (R1/R12 parity). `out_dir` is the
    /// directory containing diff-result.json. Heading values are always quoted;
    /// landmark is quoted only if it contains whitespace; ids are bare.
    pub fn drill_command(&self, out_dir: &str) -> String {
        match self {
            BranchHandle::Region { landmark } => {
                format!(
                    "matchy show --region {} --out {}",
                    quote_landmark(landmark),
                    out_dir
                )
            }
            BranchHandle::Section { landmark, heading: Some(h) } => {
                format!(
                    "matchy show --section {} --heading {} --out {}",
                    quote_landmark(landmark),
                    quote_heading(h),
                    out_dir
                )
            }
            BranchHandle::Section { landmark, heading: None } => {
                format!(
                    "matchy show --section {} --out {}",
                    quote_landmark(landmark),
                    out_dir
                )
            }
            BranchHandle::Cluster { id } => {
                format!("matchy show --cluster {} --out {}", id, out_dir)
            }
            BranchHandle::Issue { id } => {
                format!("matchy show --issue {} --out {}", id, out_dir)
            }
        }
    }
}

/// Display section key (landmark, heading) — IDENTICAL semantics to
/// markdown.rs::section_key_of: landmark None -> "(page)", heading None -> em dash.
pub fn section_key_of(issue: &Issue) -> (String, String) {
    let lm = issue
        .locator
        .anchors
        .landmark
        .as_deref()
        .unwrap_or("(page)")
        .to_string();
    let hd = issue
        .locator
        .anchors
        .nearest_heading
        .as_deref()
        .unwrap_or("\u{2014}") // em dash
        .to_string();
    (lm, hd)
}

/// Render options. Use `DisclosureOptions::new(out_dir)` for config defaults.
pub struct DisclosureOptions {
    pub out_dir: String,
    pub budget: usize,
    pub section_ceiling: usize,
}

impl DisclosureOptions {
    pub fn new(out_dir: impl Into<String>) -> Self {
        Self {
            out_dir: out_dir.into(),
            budget: crate::config::DISCLOSURE_BUDGET,
            section_ceiling: crate::config::DISCLOSURE_SECTION_CEILING,
        }
    }
}

/// One region branch in the compact view (always collapsed — high watermark).
pub struct RegionBranch {
    pub handle: BranchHandle,   // Region{landmark}
    pub landmark: String,
    pub severity: IssueSeverity,
    pub count: usize,           // member_issue_ids.len()
    pub saturation: f64,
}

/// One section branch (group of non-claimed, non-uncertain, non-critical issues).
pub struct SectionBranch {
    pub handle: BranchHandle,       // Section{landmark, heading}
    pub key: (String, String),      // display key
    pub severity: IssueSeverity,    // worst in section
    pub count: usize,
    pub fix_value: f64,             // max member fix_value (ordering key)
    pub collapsed: bool,            // budget/band decision
    /// Cached inline-rendered size (chars) used for budget accounting.
    /// Set during compute_outline; callers should treat this as internal.
    pub inline_size: usize,
}

/// The structured compact-disclosure model — computed ONCE, rendered by many.
pub struct OutlineModel {
    pub critical_lead: Vec<String>,  // issue ids, always inlined (R13)
    pub regions: Vec<RegionBranch>,  // always collapsed pointers
    pub sections: Vec<SectionBranch>, // ordered; each carries its collapsed flag
    pub clean_pass: bool,            // no regions, no critical lead, no sections
}

// ---------------------------------------------------------------------------
// Internal: render one section inline (factored out for sizing + emission)
// ---------------------------------------------------------------------------

/// Render one section as inlined markdown text.
/// Format:
/// ```
/// ### <landmark> › <heading> (<count> issues) — <drill_command>
/// - [<sev>] <type> ×<n> — <message>
/// ...
/// ```
/// Returns the rendered String. Called both for sizing (Step 3) and for
/// actual emission (Step 5).
// The determinism-safe fold pattern uses a HashMap for O(1) lookup and a
// parallel Vec<FoldKey> for first-appearance order. The Entry API cannot
// express the side-effecting `fold_order.push` on the vacant branch without
// restructuring, so we suppress the map_entry lint here intentionally.
#[allow(clippy::map_entry)]
fn render_section_inline(
    key: &(String, String),
    issues: &[&Issue],
    handle: &BranchHandle,
    out_dir: &str,
) -> String {
    let mut out = String::new();
    let n = issues.len();
    out.push_str(&format!(
        "### {} \u{203a} {} ({} issue{}) \u{2014} {}\n",
        md_cell(&key.0),
        md_cell(&key.1),
        n,
        if n == 1 { "" } else { "s" },
        handle.drill_command(out_dir)
    ));

    // Fold by (type, message) — determinism-safe: HashMap for O(1) lookup,
    // fold_order Vec for first-appearance output order.
    type FoldKey = (String, String);
    struct FoldEntry<'a> {
        worst_sev: &'a IssueSeverity,
        count: u32,
    }

    let mut fold_order: Vec<FoldKey> = Vec::new();
    let mut fold_map: HashMap<FoldKey, FoldEntry<'_>> = HashMap::new();

    for issue in issues.iter() {
        let fk: FoldKey = (issue.issue_type.as_str().to_string(), issue.message.clone());
        if !fold_map.contains_key(&fk) {
            fold_order.push(fk.clone());
            fold_map.insert(
                fk,
                FoldEntry {
                    worst_sev: &issue.severity,
                    count: 1,
                },
            );
        } else {
            let entry = fold_map
                .get_mut(&(issue.issue_type.as_str().to_string(), issue.message.clone()))
                .unwrap();
            entry.worst_sev = worst_sev(entry.worst_sev, &issue.severity);
            entry.count += 1;
        }
    }

    // Emit in first-appearance order.
    for fk in &fold_order {
        let entry = &fold_map[fk];
        out.push_str(&format!(
            "- [{}] {} \u{d7}{} \u{2014} {}\n",
            sev_label(entry.worst_sev),
            md_cell(&fk.0),
            entry.count,
            md_cell(&fk.1)
        ));
    }

    out
}

// ---------------------------------------------------------------------------
// compute_outline
// ---------------------------------------------------------------------------

/// Compute the structured model (the canonical enumeration). Pure & deterministic.
pub fn compute_outline(result: &DiffResult, opts: &DisclosureOptions) -> OutlineModel {
    let claimed = crate::report::claimed_issue_ids(result);

    // ------------------------------------------------------------------
    // 1. Critical lead (R13): all non-uncertain issues with severity == Critical.
    //    Always inline, claimed or not, regardless of budget.
    // ------------------------------------------------------------------
    let critical_lead: Vec<String> = result
        .issues
        .iter()
        .filter(|i| !is_uncertain_pairing(&i.evidence) && i.severity == IssueSeverity::Critical)
        .map(|i| i.id.clone())
        .collect();
    let critical_set: std::collections::BTreeSet<&str> =
        critical_lead.iter().map(|s| s.as_str()).collect();

    // ------------------------------------------------------------------
    // 2. Regions: one RegionBranch each, ALWAYS collapsed (high watermark).
    //    Order = result.regions order (already deterministic).
    // ------------------------------------------------------------------
    let regions: Vec<RegionBranch> = result
        .regions
        .iter()
        .map(|r| RegionBranch {
            handle: BranchHandle::Region {
                landmark: r.landmark.clone(),
            },
            landmark: r.landmark.clone(),
            severity: r.severity.clone(),
            count: r.member_issue_ids.len(),
            saturation: r.saturation,
        })
        .collect();

    // ------------------------------------------------------------------
    // 3. Sections: group remaining issues = non-uncertain AND NOT claimed
    //    AND NOT critical.
    //    Use a BTreeMap<(String,String), Vec<&Issue>> for deterministic grouping.
    // ------------------------------------------------------------------
    let mut section_groups: BTreeMap<(String, String), Vec<&Issue>> = BTreeMap::new();
    for issue in &result.issues {
        if is_uncertain_pairing(&issue.evidence) {
            continue;
        }
        if claimed.contains(issue.id.as_str()) {
            continue;
        }
        if critical_set.contains(issue.id.as_str()) {
            continue;
        }
        let key = section_key_of(issue);
        section_groups.entry(key).or_default().push(issue);
    }

    // Build SectionBranch for each group, computing inline size.
    let mut sections_with_size: Vec<SectionBranch> = section_groups
        .into_iter()
        .map(|(key, issues)| {
            let count = issues.len();

            // Worst severity.
            let severity = issues
                .iter()
                .fold(&IssueSeverity::Info, |acc, i| worst_sev(acc, &i.severity))
                .clone();

            // Max fix_value.
            let fv = issues.iter().fold(f64::NEG_INFINITY, |acc, i| {
                f64::max(
                    acc,
                    fix_value(&i.severity, i.confidence, &i.locator.anchors.strength()),
                )
            });

            // Build the handle: heading=None when heading_display == em dash.
            let em_dash = "\u{2014}";
            let heading = if key.1 == em_dash {
                None
            } else {
                Some(key.1.clone())
            };
            let handle = BranchHandle::Section {
                landmark: key.0.clone(),
                heading,
            };

            // Compute inline size proxy (must equal what render_section_inline produces).
            let inline_text = render_section_inline(&key, &issues, &handle, &opts.out_dir);
            let inline_size = inline_text.len();

            SectionBranch {
                handle,
                key,
                severity,
                count,
                fix_value: fv,
                collapsed: false, // determined in budget step below
                inline_size,
            }
        })
        .collect();

    // Sort sections by (fix_value DESC, key.0 ASC, key.1 ASC) — total order.
    sections_with_size.sort_by(|a, b| {
        b.fix_value
            .total_cmp(&a.fix_value)
            .then_with(|| a.key.0.cmp(&b.key.0))
            .then_with(|| a.key.1.cmp(&b.key.1))
    });

    // ------------------------------------------------------------------
    // 4. Budget decision (bands).
    //
    // The per-section ceiling is a STRUCTURAL high watermark that fires
    // independently of the cumulative budget: a section whose inline size
    // exceeds section_ceiling always collapses, even when the total of all
    // sections would otherwise satisfy the low-watermark condition.
    //
    // After forcing ceiling-violating sections to collapsed, we check the
    // low watermark over the *remaining* (non-ceiling-forced) sections:
    // if their combined size fits the budget, inline all of them (R4).
    // Otherwise greedy inline in sort order until the budget is exhausted.
    // ------------------------------------------------------------------

    // Pass 1: mark ceiling-forced collapses (high watermark, always).
    for s in &mut sections_with_size {
        if s.inline_size > opts.section_ceiling {
            s.collapsed = true;
        }
    }

    // Pass 2: budget decision over the non-ceiling-forced sections.
    let eligible_total: usize = sections_with_size
        .iter()
        .filter(|s| !s.collapsed)
        .map(|s| s.inline_size)
        .sum();

    if eligible_total <= opts.budget {
        // Low watermark: inline all eligible sections (R4).
        for s in &mut sections_with_size {
            if !s.collapsed {
                s.collapsed = false; // already false, but explicit for clarity
            }
        }
    } else {
        // Greedy inline: walk in sort order, inline until budget would be exceeded,
        // collapse the rest (sticky).
        let mut spent: usize = 0;
        let mut budget_exhausted = false;
        for s in &mut sections_with_size {
            if s.collapsed {
                // Already ceiling-collapsed; skip.
                continue;
            }
            if !budget_exhausted && spent + s.inline_size <= opts.budget {
                s.collapsed = false;
                spent += s.inline_size;
            } else {
                s.collapsed = true;
                budget_exhausted = true;
            }
        }
    }

    let clean_pass =
        regions.is_empty() && critical_lead.is_empty() && sections_with_size.is_empty();

    OutlineModel {
        critical_lead,
        regions,
        sections: sections_with_size,
        clean_pass,
    }
}

// ---------------------------------------------------------------------------
// render_outline
// ---------------------------------------------------------------------------

/// Render the compact disclosure markdown body (critical lead + ## Regions
/// pointers + ## Issues ToC). Self-contained block; the caller (U4) prepends the
/// shared header/summary/scores. MUST contain a `## Issues` heading (check-m8).
pub fn render_outline(result: &DiffResult, opts: &DisclosureOptions) -> String {
    let model = compute_outline(result, opts);
    let mut out = String::new();

    // 1. Critical lead (R13).
    if !model.critical_lead.is_empty() {
        out.push_str("## Critical defects\n\n");
        // Build id → &Issue lookup.
        let id_to_issue: BTreeMap<&str, &Issue> =
            result.issues.iter().map(|i| (i.id.as_str(), i)).collect();
        for cid in &model.critical_lead {
            if let Some(issue) = id_to_issue.get(cid.as_str()) {
                let handle = BranchHandle::Issue { id: cid.clone() };
                out.push_str(&format!(
                    "- [critical] {} \u{2014} {} \u{2014} drill: {}\n",
                    md_cell(issue.issue_type.as_str()),
                    md_cell(&issue.message),
                    handle.drill_command(&opts.out_dir)
                ));
            }
        }
        out.push('\n');
    }

    // 2. Regions.
    if !model.regions.is_empty() {
        out.push_str("## Regions\n\n");
        for rb in &model.regions {
            out.push_str(&format!(
                "- [{}] {} \u{2014} {} issues, saturation {:.2} \u{2014} drill: {}\n",
                sev_label(&rb.severity),
                md_cell(&rb.landmark),
                rb.count,
                rb.saturation,
                rb.handle.drill_command(&opts.out_dir)
            ));
        }
        out.push('\n');
    }

    // 3. Issues table of contents (always present — check-m8 guard).
    out.push_str("## Issues (table of contents)\n\n");

    if model.clean_pass {
        out.push_str("\nNo issues \u{2014} clean pass.\n");
        return out;
    }

    // Count collapsed sections.
    let collapsed_count = model.sections.iter().filter(|s| s.collapsed).count();
    if collapsed_count > 0 {
        out.push_str(&format!(
            "> {} section(s) collapsed to fit the budget \u{2014} expand with the drill command shown, or re-run with --full.\n\n",
            collapsed_count
        ));
    }

    // Inlined sections first (in model order).
    for s in model.sections.iter().filter(|s| !s.collapsed) {
        // Re-render the inline text (size was pre-computed; now emit it).
        let inline_text = render_section_inline(&s.key, &collect_section_issues(result, s), &s.handle, &opts.out_dir);
        out.push_str(&inline_text);
    }

    // Then collapsed section pointers (in model order).
    for s in model.sections.iter().filter(|s| s.collapsed) {
        out.push_str(&format!(
            "- [{}] {} \u{203a} {} \u{2014} {} issues \u{2014} drill: {}\n",
            sev_label(&s.severity),
            md_cell(&s.key.0),
            md_cell(&s.key.1),
            s.count,
            s.handle.drill_command(&opts.out_dir)
        ));
    }

    out
}

/// The compact-section members for a display section key: non-uncertain,
/// non-claimed, non-critical issues whose section_key_of == key, in result order.
/// Shared by render_outline (markdown) and the HTML compact grouping (U4).
pub fn section_issues<'a>(result: &'a DiffResult, key: &(String, String)) -> Vec<&'a Issue> {
    let claimed = crate::report::claimed_issue_ids(result);
    let critical_lead_set: std::collections::BTreeSet<String> = result
        .issues
        .iter()
        .filter(|i| !is_uncertain_pairing(&i.evidence) && i.severity == IssueSeverity::Critical)
        .map(|i| i.id.clone())
        .collect();

    result
        .issues
        .iter()
        .filter(|i| {
            !is_uncertain_pairing(&i.evidence)
                && !claimed.contains(i.id.as_str())
                && !critical_lead_set.contains(&i.id)
                && &section_key_of(i) == key
        })
        .collect()
}

/// Collect the issues for a given SectionBranch from the result, respecting the
/// same filter as compute_outline (non-uncertain, non-claimed, non-critical,
/// matching key). Preserves result.issues order. Delegates to section_issues.
fn collect_section_issues<'a>(result: &'a DiffResult, s: &SectionBranch) -> Vec<&'a Issue> {
    section_issues(result, &s.key)
}

// ---------------------------------------------------------------------------
// resolve_handle
// ---------------------------------------------------------------------------

/// Resolve a handle to the issues it expands to (for U5 `matchy show`).
/// - Region{landmark}: issues whose id ∈ that region's member_issue_ids.
/// - Section{landmark, heading: Some(h)}: issues whose section_key_of == (landmark, h).
/// - Section{landmark, heading: None}: ALL issues whose display landmark == landmark
///   (the whole-landmark SUPERSET — the defined R7 contract, not a leak).
/// - Cluster{id}: issues whose id ∈ that cluster's issue_ids.
/// - Issue{id}: the single issue with that id.
///
/// Returns issues in result.issues order (fix-value order). Empty Vec = unresolved.
pub fn resolve_handle<'a>(result: &'a DiffResult, handle: &BranchHandle) -> Vec<&'a Issue> {
    match handle {
        BranchHandle::Region { landmark } => {
            // Find the region, collect its member ids into a BTreeSet.
            let member_ids: std::collections::BTreeSet<&str> = result
                .regions
                .iter()
                .find(|r| &r.landmark == landmark)
                .map(|r| r.member_issue_ids.iter().map(|s| s.as_str()).collect())
                .unwrap_or_default();
            result
                .issues
                .iter()
                .filter(|i| member_ids.contains(i.id.as_str()))
                .collect()
        }
        BranchHandle::Section { landmark, heading: Some(h) } => {
            let target_key = (landmark.clone(), h.clone());
            result
                .issues
                .iter()
                .filter(|i| section_key_of(i) == target_key)
                .collect()
        }
        BranchHandle::Section { landmark, heading: None } => {
            // Whole-landmark superset: all issues whose display landmark == landmark.
            result
                .issues
                .iter()
                .filter(|i| {
                    i.locator
                        .anchors
                        .landmark
                        .as_deref()
                        .unwrap_or("(page)")
                        == landmark.as_str()
                })
                .collect()
        }
        BranchHandle::Cluster { id } => {
            let member_ids: std::collections::BTreeSet<&str> = result
                .clusters
                .iter()
                .find(|c| &c.id == id)
                .map(|c| c.issue_ids.iter().map(|s| s.as_str()).collect())
                .unwrap_or_default();
            result
                .issues
                .iter()
                .filter(|i| member_ids.contains(i.id.as_str()))
                .collect()
        }
        BranchHandle::Issue { id } => result
            .issues
            .iter()
            .filter(|i| &i.id == id)
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// render_branch_detail
// ---------------------------------------------------------------------------

/// Human-readable, byte-deterministic full detail for one expanded branch
/// (`matchy show`). `issues` are the resolved members (already in result order).
///
/// Output format (exact shape; deterministic — issues in given order, fixed field order):
/// ```text
/// matchy show — <handle-desc>
/// <N> issue(s) in this branch:
///
/// <id>  [<severity>] <type>  (<category>, confidence <conf:.2>, viewport <vp>)
///   section: <landmark> › <heading>
///   message: <message>            (single line; collapse newlines to spaces)
///   evidence: old=<compact-json> new=<compact-json>     (only if evidence.old or .new present)
///   remediation: <action>; grep: <t1>, <t2>             (only if remediation present)
/// ```
pub fn render_branch_detail(handle: &BranchHandle, issues: &[&Issue]) -> String {
    // --- handle description ---
    let handle_desc = match handle {
        BranchHandle::Region { landmark } => format!("region {}", landmark),
        BranchHandle::Section { landmark, heading: Some(h) } => {
            format!("section {} \u{203a} {}", landmark, h)
        }
        BranchHandle::Section { landmark, heading: None } => {
            format!("section {} (whole landmark)", landmark)
        }
        BranchHandle::Cluster { id } => format!("cluster {}", id),
        BranchHandle::Issue { id } => format!("issue {}", id),
    };

    let mut out = String::new();
    out.push_str(&format!("matchy show \u{2014} {}\n", handle_desc));

    let n = issues.len();
    out.push_str(&format!(
        "{} issue(s) in this branch:\n",
        n
    ));

    for issue in issues {
        out.push('\n');

        // Category as lowercase string (mirrors serde serialization).
        let cat_str = match &issue.category {
            crate::contract::IssueCategory::Visual => "visual",
            crate::contract::IssueCategory::Content => "content",
            crate::contract::IssueCategory::Structure => "structure",
            crate::contract::IssueCategory::Style => "style",
            crate::contract::IssueCategory::Accessibility => "accessibility",
            crate::contract::IssueCategory::Technical => "technical",
            crate::contract::IssueCategory::Hygiene => "hygiene",
        };

        // Header line: <id>  [<severity>] <type>  (<category>, confidence <conf:.2>, viewport <vp>)
        out.push_str(&format!(
            "{}  [{}] {}  ({}, confidence {:.2}, viewport {})\n",
            issue.id,
            sev_label(&issue.severity),
            issue.issue_type.as_str(),
            cat_str,
            issue.confidence,
            issue.viewport
        ));

        // section: line using section_key_of.
        let (lm, hd) = section_key_of(issue);
        out.push_str(&format!("  section: {} \u{203a} {}\n", lm, hd));

        // message: collapse newlines to spaces, trim.
        let msg = issue
            .message
            .replace(['\r', '\n'], " ");
        let msg = msg.trim();
        out.push_str(&format!("  message: {}\n", msg));

        // evidence: only if old or new present.
        let ev_old = issue.evidence.get("old");
        let ev_new = issue.evidence.get("new");
        if ev_old.is_some() || ev_new.is_some() {
            let mut ev_parts = Vec::new();
            if let Some(v) = ev_old {
                ev_parts.push(format!(
                    "old={}",
                    serde_json::to_string(v).unwrap_or_else(|_| "null".to_string())
                ));
            }
            if let Some(v) = ev_new {
                ev_parts.push(format!(
                    "new={}",
                    serde_json::to_string(v).unwrap_or_else(|_| "null".to_string())
                ));
            }
            out.push_str(&format!("  evidence: {}\n", ev_parts.join(" ")));
        }

        // remediation: only if present and has action.
        if let Some(rem) = &issue.remediation {
            if let Some(action) = rem.get("action").and_then(|v| v.as_str()) {
                let mut rem_str = action.to_string();
                // Append grep targets if non-empty.
                if let Some(targets) = rem.get("grepTargets").and_then(|v| v.as_array()) {
                    let ts: Vec<&str> = targets
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect();
                    if !ts.is_empty() {
                        rem_str.push_str(&format!("; grep: {}", ts.join(", ")));
                    }
                }
                out.push_str(&format!("  remediation: {}\n", rem_str));
            }
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        AgentSummary, Anchors, Artifacts, Cluster, DeterminismSummary, DiffResult, Issue,
        IssueCategory, IssueSeverity, IssueType, Locator, OutOfScope, Region, Scores,
        Status, Suppressed, ViewportResult,
    };
    use std::collections::BTreeMap;

    // -----------------------------------------------------------------------
    // Fixture helpers
    // -----------------------------------------------------------------------

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
            viewport: "desktop".to_string(),
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

    fn make_empty_result() -> DiffResult {
        let mut by_type = BTreeMap::new();
        by_type.insert("changed_text".to_string(), 0u32);

        DiffResult {
            schema_version: "1.2".to_string(),
            tool_version: "0.0.0".to_string(),
            run_id: "2026-01-01T00-00-00Z".to_string(),
            old_url: "https://example.com/old".to_string(),
            new_url: "https://example.com/new".to_string(),
            parity_profile: "content-structure".to_string(),
            status: Status::Pass,
            agent_summary: AgentSummary {
                fixable_now: 0,
                by_type,
                cluster_count: 0,
                region_count: 0,
                top_fixes: vec![],
            },
            scores: Scores {
                visual: 1.0,
                content: 1.0,
                structure: 1.0,
                style: 1.0,
                accessibility: 1.0,
                technical: 1.0,
                hygiene: 1.0,
                by_landmark: BTreeMap::new(),
            },
            viewports: vec![ViewportResult {
                name: "desktop".to_string(),
                status: Status::Pass,
                issues: vec![],
                artifacts: Artifacts {
                    old: "desktop/old.png".to_string(),
                    new: "desktop/new.png".to_string(),
                    diff: "desktop/diff.png".to_string(),
                },
            }],
            issues: vec![],
            clusters: vec![],
            regions: vec![],
            suppressed: Suppressed {
                count: 0,
                ids: vec![],
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

    fn make_region(
        id: &str,
        landmark: &str,
        saturation: f64,
        severity: IssueSeverity,
        member_ids: Vec<String>,
    ) -> Region {
        Region {
            id: id.to_string(),
            landmark: landmark.to_string(),
            saturation,
            structural_count: member_ids.len() as u32,
            old_node_count: (member_ids.len() + 2) as u32,
            member_issue_ids: member_ids,
            severity,
            summary: format!("{} region rollup", landmark),
        }
    }

    // -----------------------------------------------------------------------
    // Test 1: regions always collapsed
    // -----------------------------------------------------------------------

    #[test]
    fn test_compute_outline_regions_always_collapsed() {
        let mut result = make_empty_result();
        let member_ids: Vec<String> = (0..5u8).map(|i| format!("issue_{i:016x}")).collect();
        for (i, id) in member_ids.iter().enumerate() {
            result.issues.push(make_issue(
                id,
                IssueType::ChangedText,
                IssueSeverity::Warning,
                &format!("msg {i}"),
                Some("contentinfo"),
                Some("Products"),
                false,
            ));
        }
        let mut sorted_ids = member_ids.clone();
        sorted_ids.sort();
        result.regions = vec![make_region(
            "region_cinfo_0001",
            "contentinfo",
            0.86,
            IssueSeverity::Error,
            sorted_ids,
        )];
        result.agent_summary.region_count = 1;

        let opts = DisclosureOptions::new("/tmp/out");
        let model = compute_outline(&result, &opts);

        assert_eq!(model.regions.len(), 1);
        // Regions are always collapsed (high watermark) — the model doesn't have a
        // per-region collapsed flag, their presence IS the always-collapsed contract.
        let cmd = model.regions[0].handle.drill_command("/tmp/out");
        assert!(
            cmd.contains("matchy show --region"),
            "drill_command must contain 'matchy show --region', got: {cmd}"
        );
        assert!(
            cmd.contains("contentinfo"),
            "drill_command must contain the landmark, got: {cmd}"
        );
    }

    // -----------------------------------------------------------------------
    // Test 2: low watermark inlines everything (AE3/R4)
    // -----------------------------------------------------------------------

    #[test]
    fn test_low_watermark_inlines_everything() {
        let mut result = make_empty_result();
        // A few small issues in two sections.
        result.issues.push(make_issue(
            "issue_main_0000001",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "text changed",
            Some("main"),
            Some("Hero"),
            false,
        ));
        result.issues.push(make_issue(
            "issue_main_0000002",
            IssueType::BrokenLink,
            IssueSeverity::Error,
            "link broken",
            Some("main"),
            Some("Links"),
            false,
        ));

        // Use a very large budget so everything is always inlined.
        let opts = DisclosureOptions {
            out_dir: "/tmp/out".to_string(),
            budget: 100_000,
            section_ceiling: 100_000,
        };
        let model = compute_outline(&result, &opts);

        // All sections should have collapsed == false.
        for s in &model.sections {
            assert!(
                !s.collapsed,
                "section {:?} must be inlined with huge budget",
                s.key
            );
        }

        // render_outline should have no collapsed pointer lines.
        let rendered = render_outline(&result, &opts);
        assert!(
            !rendered.contains("collapsed to fit the budget"),
            "no collapse breadcrumb expected when budget huge"
        );
    }

    // -----------------------------------------------------------------------
    // Test 3: high watermark budget collapses
    // -----------------------------------------------------------------------

    #[test]
    fn test_high_watermark_budget_collapses() {
        let mut result = make_empty_result();
        // Several sections.
        for i in 0u8..8 {
            result.issues.push(make_issue(
                &format!("issue_sec_{i:016x}"),
                IssueType::ChangedText,
                IssueSeverity::Warning,
                &format!("message for issue {i}"),
                Some(&format!("main")),
                Some(&format!("Section {i}")),
                false,
            ));
        }

        // Tiny budget — almost certainly not enough for all 8 sections.
        let opts = DisclosureOptions {
            out_dir: "/tmp/out".to_string(),
            budget: 50,
            section_ceiling: 10_000,
        };
        let model = compute_outline(&result, &opts);

        let any_collapsed = model.sections.iter().any(|s| s.collapsed);
        assert!(any_collapsed, "at least one section must be collapsed with tiny budget");

        let rendered = render_outline(&result, &opts);
        // A collapsed pointer line must contain "drill: matchy show --section".
        assert!(
            rendered.contains("drill: matchy show --section"),
            "rendered output must contain collapsed section pointer, got: {rendered}"
        );
        assert!(
            rendered.contains("collapsed to fit the budget"),
            "breadcrumb must be present when sections collapsed"
        );
    }

    // -----------------------------------------------------------------------
    // Test 4: section_ceiling forces collapse
    // -----------------------------------------------------------------------

    #[test]
    fn test_section_ceiling_forces_collapse() {
        let mut result = make_empty_result();
        // One section with several issues (will produce a long inline block).
        for i in 0u8..10 {
            result.issues.push(make_issue(
                &format!("issue_ceil_{i:016x}"),
                IssueType::ChangedText,
                IssueSeverity::Warning,
                &format!("message for the ceiling test issue number {i} with some extra text"),
                Some("main"),
                Some("Big Section"),
                false,
            ));
        }

        // Large budget but very small per-section ceiling.
        let opts = DisclosureOptions {
            out_dir: "/tmp/out".to_string(),
            budget: 100_000,
            section_ceiling: 1, // forces collapse
        };
        let model = compute_outline(&result, &opts);

        // The section must be collapsed.
        assert_eq!(model.sections.len(), 1);
        assert!(
            model.sections[0].collapsed,
            "section must be collapsed when inline_size > section_ceiling"
        );
    }

    // -----------------------------------------------------------------------
    // Test 5: critical member always in lead (R13)
    // -----------------------------------------------------------------------

    #[test]
    fn test_critical_member_always_in_lead() {
        let mut result = make_empty_result();
        // A region claiming a critical issue.
        let critical_id = "issue_critical_0001".to_string();
        result.issues.push(make_issue(
            &critical_id,
            IssueType::LoadError,
            IssueSeverity::Critical,
            "page failed to load",
            Some("contentinfo"),
            Some("Header"),
            false,
        ));
        // A few more non-critical footer issues.
        let mut member_ids = vec![critical_id.clone()];
        for i in 0u8..4 {
            let id = format!("issue_footer_{i:016x}");
            result.issues.push(make_issue(
                &id,
                IssueType::ChangedText,
                IssueSeverity::Warning,
                &format!("footer msg {i}"),
                Some("contentinfo"),
                Some("Products"),
                false,
            ));
            member_ids.push(id);
        }
        member_ids.sort();
        result.regions = vec![make_region(
            "region_cinfo_crit",
            "contentinfo",
            0.90,
            IssueSeverity::Critical,
            member_ids,
        )];
        result.agent_summary.region_count = 1;

        // Tiny budget.
        let opts = DisclosureOptions {
            out_dir: "/tmp/out".to_string(),
            budget: 1,
            section_ceiling: 1,
        };
        let model = compute_outline(&result, &opts);

        // Critical id must be in the lead.
        assert!(
            model.critical_lead.contains(&critical_id),
            "critical issue must be in critical_lead"
        );

        // render_outline must show it under ## Critical defects.
        let rendered = render_outline(&result, &opts);
        assert!(
            rendered.contains("## Critical defects"),
            "## Critical defects section must appear"
        );
        assert!(
            rendered.contains(&critical_id) || rendered.contains("load_error"),
            "critical issue or its type must appear in render: {rendered}"
        );
    }

    // -----------------------------------------------------------------------
    // Test 6: standalone broken_link surfaces (R13)
    // -----------------------------------------------------------------------

    #[test]
    fn test_standalone_broken_link_surfaces() {
        let mut result = make_empty_result();

        // Saturated contentinfo region.
        let mut member_ids: Vec<String> = (0u8..5).map(|i| format!("issue_ft_{i:016x}")).collect();
        for id in &member_ids {
            result.issues.push(make_issue(
                id,
                IssueType::ChangedText,
                IssueSeverity::Warning,
                "footer text changed",
                Some("contentinfo"),
                Some("Products"),
                false,
            ));
        }
        member_ids.sort();
        result.regions = vec![make_region(
            "region_cinfo_0002",
            "contentinfo",
            0.80,
            IssueSeverity::Warning,
            member_ids,
        )];
        result.agent_summary.region_count = 1;

        // Non-claimed broken_link in unsaturated main.
        result.issues.push(make_issue(
            "issue_broken_link_01",
            IssueType::BrokenLink,
            IssueSeverity::Error,
            "Link target missing",
            Some("main"),
            Some("Body"),
            false,
        ));

        // Default budget should inline the broken_link section.
        let opts = DisclosureOptions::new("/tmp/out");
        let model = compute_outline(&result, &opts);

        // Find the main section branch.
        let main_section = model
            .sections
            .iter()
            .find(|s| s.key.0 == "main")
            .expect("main section must be in model");

        assert!(
            !main_section.collapsed,
            "broken_link section in main must be inlined (not collapsed) with default budget"
        );

        let rendered = render_outline(&result, &opts);
        assert!(
            rendered.contains("Link target missing"),
            "broken_link message must appear in render"
        );
    }

    // -----------------------------------------------------------------------
    // Test 7: handle roundtrip — region, section, cluster, issue
    // -----------------------------------------------------------------------

    #[test]
    fn test_handle_roundtrip_region() {
        let mut result = make_empty_result();
        let member_ids: Vec<String> = vec![
            "issue_region_m001".to_string(),
            "issue_region_m002".to_string(),
        ];
        for id in &member_ids {
            result.issues.push(make_issue(
                id,
                IssueType::ChangedText,
                IssueSeverity::Warning,
                "region msg",
                Some("contentinfo"),
                Some("Prods"),
                false,
            ));
        }
        let mut sorted = member_ids.clone();
        sorted.sort();
        result.regions = vec![make_region(
            "region_rt_0001",
            "contentinfo",
            0.85,
            IssueSeverity::Warning,
            sorted,
        )];

        let handle = BranchHandle::Region {
            landmark: "contentinfo".to_string(),
        };
        let resolved = resolve_handle(&result, &handle);
        let resolved_ids: Vec<&str> = resolved.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(resolved_ids.len(), 2);
        for id in &member_ids {
            assert!(
                resolved_ids.contains(&id.as_str()),
                "member {id} must be resolved"
            );
        }
    }

    #[test]
    fn test_handle_roundtrip_section_with_heading() {
        let mut result = make_empty_result();
        result.issues.push(make_issue(
            "issue_sec_h_001",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "section heading msg",
            Some("main"),
            Some("FAQs"),
            false,
        ));
        result.issues.push(make_issue(
            "issue_sec_h_002",
            IssueType::BrokenLink,
            IssueSeverity::Error,
            "another main faq msg",
            Some("main"),
            Some("FAQs"),
            false,
        ));
        // Issue in a different heading — must NOT be resolved.
        result.issues.push(make_issue(
            "issue_sec_h_003",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "about msg",
            Some("main"),
            Some("About"),
            false,
        ));

        let handle = BranchHandle::Section {
            landmark: "main".to_string(),
            heading: Some("FAQs".to_string()),
        };
        let resolved = resolve_handle(&result, &handle);
        assert_eq!(resolved.len(), 2);
        let ids: Vec<&str> = resolved.iter().map(|i| i.id.as_str()).collect();
        assert!(ids.contains(&"issue_sec_h_001"));
        assert!(ids.contains(&"issue_sec_h_002"));
        assert!(!ids.contains(&"issue_sec_h_003"));
    }

    #[test]
    fn test_handle_roundtrip_section_no_heading_superset() {
        // Section{heading:None} must return ALL issues in that landmark, across headings.
        let mut result = make_empty_result();
        result.issues.push(make_issue(
            "issue_lm_h1_001",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "h1 msg",
            Some("main"),
            Some("Heading1"),
            false,
        ));
        result.issues.push(make_issue(
            "issue_lm_h2_001",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "h2 msg",
            Some("main"),
            Some("Heading2"),
            false,
        ));
        // Different landmark — must NOT be included.
        result.issues.push(make_issue(
            "issue_nav_001",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "nav msg",
            Some("nav"),
            Some("Heading1"),
            false,
        ));

        let handle = BranchHandle::Section {
            landmark: "main".to_string(),
            heading: None,
        };
        let resolved = resolve_handle(&result, &handle);
        assert_eq!(resolved.len(), 2, "superset must return all main issues");
        let ids: Vec<&str> = resolved.iter().map(|i| i.id.as_str()).collect();
        assert!(ids.contains(&"issue_lm_h1_001"));
        assert!(ids.contains(&"issue_lm_h2_001"));
        assert!(!ids.contains(&"issue_nav_001"));
    }

    #[test]
    fn test_handle_roundtrip_cluster() {
        let mut result = make_empty_result();
        result.issues.push(make_issue(
            "issue_cl_001",
            IssueType::StyleChanged,
            IssueSeverity::Warning,
            "style changed 1",
            Some("main"),
            Some("Hero"),
            false,
        ));
        result.issues.push(make_issue(
            "issue_cl_002",
            IssueType::StyleChanged,
            IssueSeverity::Warning,
            "style changed 2",
            Some("main"),
            Some("Hero"),
            false,
        ));
        result.clusters = vec![Cluster {
            id: "cluster_abc123".to_string(),
            issue_ids: vec!["issue_cl_001".to_string(), "issue_cl_002".to_string()],
            shared_property: Some("color".to_string()),
            shared_landmark: None,
            summary: Some("2 style_changed issues share color".to_string()),
        }];

        let handle = BranchHandle::Cluster {
            id: "cluster_abc123".to_string(),
        };
        let resolved = resolve_handle(&result, &handle);
        assert_eq!(resolved.len(), 2);
        let ids: Vec<&str> = resolved.iter().map(|i| i.id.as_str()).collect();
        assert!(ids.contains(&"issue_cl_001"));
        assert!(ids.contains(&"issue_cl_002"));
    }

    #[test]
    fn test_handle_roundtrip_issue() {
        let mut result = make_empty_result();
        result.issues.push(make_issue(
            "issue_single_0001",
            IssueType::BrokenLink,
            IssueSeverity::Error,
            "single issue",
            Some("main"),
            Some("Body"),
            false,
        ));

        let handle = BranchHandle::Issue {
            id: "issue_single_0001".to_string(),
        };
        let resolved = resolve_handle(&result, &handle);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].id, "issue_single_0001");
    }

    // -----------------------------------------------------------------------
    // Test 8: render deterministic (AE2)
    // -----------------------------------------------------------------------

    #[test]
    fn test_render_deterministic() {
        let mut result = make_empty_result();
        for i in 0u8..5 {
            result.issues.push(make_issue(
                &format!("issue_det_{i:016x}"),
                IssueType::ChangedText,
                IssueSeverity::Warning,
                &format!("det msg {i}"),
                Some("main"),
                Some("Section"),
                false,
            ));
        }

        let opts = DisclosureOptions::new("/tmp/out");
        let r1 = render_outline(&result, &opts);
        let r2 = render_outline(&result, &opts);
        assert_eq!(r1, r2, "render_outline must be byte-deterministic");
    }

    // -----------------------------------------------------------------------
    // Test 9: clean pass
    // -----------------------------------------------------------------------

    #[test]
    fn test_clean_pass() {
        let result = make_empty_result();
        let opts = DisclosureOptions::new("/tmp/out");
        let model = compute_outline(&result, &opts);

        assert!(model.clean_pass, "empty result must be clean_pass");
        assert!(model.critical_lead.is_empty());
        assert!(model.regions.is_empty());
        assert!(model.sections.is_empty());

        let rendered = render_outline(&result, &opts);
        assert!(
            rendered.contains("## Issues"),
            "## Issues heading must always be present"
        );
        assert!(
            rendered.contains("No issues"),
            "clean pass must say 'No issues'"
        );
        assert!(
            rendered.contains("clean pass"),
            "clean pass must say 'clean pass'"
        );
        assert!(
            !rendered.contains("drill:"),
            "clean pass must have no drill pointers"
        );
        assert!(
            !rendered.contains("## Regions"),
            "clean pass must have no ## Regions"
        );
    }

    // -----------------------------------------------------------------------
    // Test 10: drill_command quoting
    // -----------------------------------------------------------------------

    #[test]
    fn test_drill_command_quoting() {
        // Heading with spaces must be quoted; landmark without spaces stays bare.
        let handle = BranchHandle::Section {
            landmark: "main".to_string(),
            heading: Some("Start for free".to_string()),
        };
        let cmd = handle.drill_command("/tmp/out");
        assert!(
            cmd.contains("--heading \"Start for free\""),
            "heading with spaces must be quoted, got: {cmd}"
        );
        // landmark "main" has no whitespace — must appear bare.
        assert!(
            cmd.contains("--section main "),
            "landmark without spaces must be bare, got: {cmd}"
        );
    }

    #[test]
    fn test_drill_command_quoting_landmark_with_spaces() {
        let handle = BranchHandle::Section {
            landmark: "my section".to_string(),
            heading: None,
        };
        let cmd = handle.drill_command("/tmp/out");
        assert!(
            cmd.contains("--section \"my section\""),
            "landmark with spaces must be quoted, got: {cmd}"
        );
    }

    // -----------------------------------------------------------------------
    // Test 11: section_key_of matches markdown semantics
    // -----------------------------------------------------------------------

    #[test]
    fn test_section_key_of_matches_markdown() {
        let em_dash = "\u{2014}";

        // Page-level issue (landmark None) -> key ("(page)", em-dash).
        let page_issue = make_issue(
            "issue_page_001",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "page msg",
            None,
            None,
            false,
        );
        let key = section_key_of(&page_issue);
        assert_eq!(key.0, "(page)", "landmark None must map to (page)");
        assert_eq!(key.1, em_dash, "heading None must map to em dash");

        // landmark Some("main"), heading None -> ("main", em-dash).
        let main_issue = make_issue(
            "issue_main_001",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "main msg",
            Some("main"),
            None,
            false,
        );
        let key2 = section_key_of(&main_issue);
        assert_eq!(key2.0, "main");
        assert_eq!(key2.1, em_dash, "heading None must map to em dash");
    }

    // -----------------------------------------------------------------------
    // Tests for render_branch_detail
    // -----------------------------------------------------------------------

    #[test]
    fn test_render_branch_detail_region_basic() {
        let issue = make_issue(
            "issue_rd_001",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "footer text changed",
            Some("contentinfo"),
            Some("Products"),
            false,
        );
        let handle = BranchHandle::Region {
            landmark: "contentinfo".to_string(),
        };
        let issues = vec![&issue];
        let detail = render_branch_detail(&handle, &issues);

        assert!(
            detail.starts_with("matchy show \u{2014} region contentinfo\n"),
            "must start with handle desc, got: {detail}"
        );
        assert!(
            detail.contains("1 issue(s) in this branch:"),
            "must contain count, got: {detail}"
        );
        assert!(
            detail.contains("issue_rd_001"),
            "must contain the issue id, got: {detail}"
        );
        assert!(
            detail.contains("[warning]"),
            "must contain severity label, got: {detail}"
        );
        assert!(
            detail.contains("changed_text"),
            "must contain issue type, got: {detail}"
        );
        assert!(
            detail.contains("content"),
            "must contain category, got: {detail}"
        );
        assert!(
            detail.contains("confidence 0.90"),
            "must contain formatted confidence, got: {detail}"
        );
        assert!(
            detail.contains("section: contentinfo \u{203a} Products"),
            "must contain section line, got: {detail}"
        );
        assert!(
            detail.contains("message: footer text changed"),
            "must contain message line, got: {detail}"
        );
        // No evidence/remediation in this issue.
        assert!(
            !detail.contains("evidence:"),
            "no evidence line expected, got: {detail}"
        );
        assert!(
            !detail.contains("remediation:"),
            "no remediation line expected, got: {detail}"
        );
    }

    #[test]
    fn test_render_branch_detail_with_evidence_and_remediation() {
        let mut issue = make_issue(
            "issue_ev_001",
            IssueType::StyleChanged,
            IssueSeverity::Error,
            "color changed",
            Some("main"),
            Some("Hero"),
            false,
        );
        issue.evidence = serde_json::json!({
            "old": "#ff0000",
            "new": "#0000ff"
        });
        issue.remediation = Some(serde_json::json!({
            "action": "Update color token",
            "grepTargets": ["color-primary", "brand-red"]
        }));

        let handle = BranchHandle::Section {
            landmark: "main".to_string(),
            heading: Some("Hero".to_string()),
        };
        let issues = vec![&issue];
        let detail = render_branch_detail(&handle, &issues);

        assert!(
            detail.starts_with("matchy show \u{2014} section main \u{203a} Hero\n"),
            "handle desc must be section main › Hero, got: {detail}"
        );
        assert!(
            detail.contains("evidence:"),
            "evidence line expected, got: {detail}"
        );
        assert!(
            detail.contains("old="),
            "evidence old= expected, got: {detail}"
        );
        assert!(
            detail.contains("new="),
            "evidence new= expected, got: {detail}"
        );
        assert!(
            detail.contains("remediation: Update color token"),
            "remediation action expected, got: {detail}"
        );
        assert!(
            detail.contains("grep: color-primary, brand-red"),
            "grep targets expected, got: {detail}"
        );
    }

    #[test]
    fn test_render_branch_detail_section_no_heading() {
        let issue = make_issue(
            "issue_whole_001",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "whole landmark text changed",
            Some("main"),
            Some("Any Heading"),
            false,
        );
        let handle = BranchHandle::Section {
            landmark: "main".to_string(),
            heading: None,
        };
        let issues = vec![&issue];
        let detail = render_branch_detail(&handle, &issues);

        assert!(
            detail.contains("section main (whole landmark)"),
            "whole-landmark desc expected, got: {detail}"
        );
    }

    #[test]
    fn test_render_branch_detail_cluster_handle() {
        let issue = make_issue(
            "issue_cl_x01",
            IssueType::StyleChanged,
            IssueSeverity::Warning,
            "cluster style changed",
            Some("main"),
            Some("Sect"),
            false,
        );
        let handle = BranchHandle::Cluster {
            id: "cluster_aabbccddeeff".to_string(),
        };
        let issues = vec![&issue];
        let detail = render_branch_detail(&handle, &issues);

        assert!(
            detail.contains("cluster cluster_aabbccddeeff"),
            "cluster handle desc expected, got: {detail}"
        );
    }

    #[test]
    fn test_render_branch_detail_message_newline_collapse() {
        let mut issue = make_issue(
            "issue_nl_001",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "line one\nline two\r\nline three",
            Some("main"),
            Some("Body"),
            false,
        );
        issue.message = "line one\nline two\r\nline three".to_string();

        let handle = BranchHandle::Issue {
            id: "issue_nl_001".to_string(),
        };
        let issues = vec![&issue];
        let detail = render_branch_detail(&handle, &issues);

        // Newlines must be collapsed to spaces and trimmed.
        assert!(
            detail.contains("message: line one line two  line three"),
            "newlines must be replaced with spaces, got: {detail}"
        );
        // The message line itself must not contain a newline (other than the
        // trailing \n that terminates the "message: ..." line).
        let msg_line = detail
            .lines()
            .find(|l| l.contains("message:"))
            .expect("message line must be present");
        assert!(
            !msg_line.contains('\n'),
            "message line must not contain embedded newline, got: {msg_line}"
        );
    }

    #[test]
    fn test_render_branch_detail_deterministic() {
        let issue1 = make_issue(
            "issue_deta_001",
            IssueType::ChangedText,
            IssueSeverity::Warning,
            "msg A",
            Some("main"),
            Some("Sec"),
            false,
        );
        let issue2 = make_issue(
            "issue_deta_002",
            IssueType::BrokenLink,
            IssueSeverity::Error,
            "msg B",
            Some("main"),
            Some("Sec"),
            false,
        );
        let handle = BranchHandle::Region {
            landmark: "main".to_string(),
        };
        let issues = vec![&issue1, &issue2];

        let r1 = render_branch_detail(&handle, &issues);
        let r2 = render_branch_detail(&handle, &issues);
        assert_eq!(r1, r2, "render_branch_detail must be byte-deterministic");
    }
}
