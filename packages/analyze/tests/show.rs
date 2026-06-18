//! Integration tests for `matchy show`.
//!
//! Drives the real binary via `std::process::Command` using the
//! `CARGO_BIN_EXE_matchy` env var (set by Cargo for integration tests).
//! No browser, network, capture bundles, or re-analysis — hermetic file-only.
//!
//! The test fixture is constructed programmatically via `matchy_analyze::contract`
//! structs + `to_json()` so the camelCase serialization is always correct.

use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixture builder
// ---------------------------------------------------------------------------

use matchy_analyze::contract::{
    AgentSummary, Anchors, Artifacts, Cluster, DeterminismSummary, DiffResult, Issue,
    IssueCategory, IssueSeverity, IssueType, Locator, OutOfScope, Region, Scores, Status,
    Suppressed, ViewportResult,
};
use std::collections::BTreeMap;

fn make_default_det() -> matchy_analyze::contract::CaptureDeterminism {
    use matchy_analyze::contract::StepStatus;
    matchy_analyze::contract::CaptureDeterminism {
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
) -> Issue {
    Issue {
        id: id.to_string(),
        issue_type,
        category: IssueCategory::Content,
        severity,
        confidence: 0.85,
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
        evidence: serde_json::json!({}),
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
        status: Status::Fail,
        agent_summary: AgentSummary {
            fixable_now: 2,
            by_type,
            cluster_count: 0,
            region_count: 0,
            top_fixes: vec![],
        },
        scores: Scores {
            visual: 0.9,
            content: 0.8,
            structure: 0.95,
            style: 1.0,
            accessibility: 1.0,
            technical: 1.0,
            hygiene: 1.0,
            by_landmark: BTreeMap::new(),
        },
        viewports: vec![ViewportResult {
            name: "desktop".to_string(),
            status: Status::Fail,
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

fn make_region(landmark: &str, saturation: f64, member_ids: Vec<String>) -> Region {
    Region {
        id: format!("region_{}", landmark),
        landmark: landmark.to_string(),
        saturation,
        structural_count: member_ids.len() as u32,
        old_node_count: (member_ids.len() + 3) as u32,
        member_issue_ids: member_ids,
        severity: IssueSeverity::Warning,
        summary: format!("{} region rollup", landmark),
    }
}

/// Write a DiffResult to `<dir>/diff-result.json` and return its path.
fn write_diff_result(dir: &TempDir, result: &DiffResult) -> PathBuf {
    let json = result.to_json().expect("to_json must not fail");
    let path = dir.path().join("diff-result.json");
    std::fs::write(&path, json).expect("write must not fail");
    path
}

/// Run `matchy show` with the given args.
fn run_show(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_matchy"))
        .arg("show")
        .args(args)
        .output()
        .expect("failed to spawn matchy")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// AE4 (TEST-FIRST): region handle → all member ids + messages, exit 0.
#[test]
fn test_show_region_expands_members_exit0() {
    let dir = TempDir::new().unwrap();
    let mut result = make_empty_result();

    let id1 = "issue_footer_001".to_string();
    let id2 = "issue_footer_002".to_string();
    result.issues.push(make_issue(
        &id1,
        IssueType::ChangedText,
        IssueSeverity::Warning,
        "footer text A changed",
        Some("contentinfo"),
        Some("Products"),
    ));
    result.issues.push(make_issue(
        &id2,
        IssueType::BrokenLink,
        IssueSeverity::Error,
        "footer link B broken",
        Some("contentinfo"),
        Some("Links"),
    ));

    let mut member_ids = vec![id1.clone(), id2.clone()];
    member_ids.sort();
    result.regions = vec![make_region("contentinfo", 0.82, member_ids)];
    result.agent_summary.region_count = 1;

    write_diff_result(&dir, &result);

    let out = run_show(&["--region", "contentinfo", "--out", dir.path().to_str().unwrap()]);

    assert_eq!(
        out.status.code(),
        Some(0),
        "exit code must be 0, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&id1),
        "stdout must contain id1 '{}', got: {}",
        id1,
        stdout
    );
    assert!(
        stdout.contains(&id2),
        "stdout must contain id2 '{}', got: {}",
        id2,
        stdout
    );
    assert!(
        stdout.contains("footer text A changed"),
        "stdout must contain message for id1, got: {}",
        stdout
    );
    assert!(
        stdout.contains("footer link B broken"),
        "stdout must contain message for id2, got: {}",
        stdout
    );
}

/// Section with heading → exit 0, member present.
#[test]
fn test_show_section_with_heading_exit0() {
    let dir = TempDir::new().unwrap();
    let mut result = make_empty_result();

    result.issues.push(make_issue(
        "issue_faq_001",
        IssueType::ChangedText,
        IssueSeverity::Warning,
        "FAQ text changed",
        Some("main"),
        Some("FAQs"),
    ));
    // Another heading — must NOT appear.
    result.issues.push(make_issue(
        "issue_hero_001",
        IssueType::ChangedText,
        IssueSeverity::Warning,
        "hero text changed",
        Some("main"),
        Some("Hero"),
    ));

    write_diff_result(&dir, &result);

    let out = run_show(&[
        "--section",
        "main",
        "--heading",
        "FAQs",
        "--out",
        dir.path().to_str().unwrap(),
    ]);

    assert_eq!(
        out.status.code(),
        Some(0),
        "exit 0 expected, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("issue_faq_001"),
        "must contain FAQ issue id, got: {}",
        stdout
    );
    assert!(
        stdout.contains("FAQ text changed"),
        "must contain FAQ message, got: {}",
        stdout
    );
    // hero issue must not be in this scoped expansion.
    assert!(
        !stdout.contains("issue_hero_001"),
        "must NOT contain hero issue id, got: {}",
        stdout
    );
}

/// Section without heading → whole-landmark superset, exit 0, both headings' issues present.
#[test]
fn test_show_section_no_heading_superset() {
    let dir = TempDir::new().unwrap();
    let mut result = make_empty_result();

    result.issues.push(make_issue(
        "issue_main_h1_001",
        IssueType::ChangedText,
        IssueSeverity::Warning,
        "heading one text changed",
        Some("main"),
        Some("Heading One"),
    ));
    result.issues.push(make_issue(
        "issue_main_h2_001",
        IssueType::BrokenLink,
        IssueSeverity::Error,
        "heading two link broken",
        Some("main"),
        Some("Heading Two"),
    ));
    // Different landmark — must NOT appear.
    result.issues.push(make_issue(
        "issue_nav_001",
        IssueType::ChangedText,
        IssueSeverity::Warning,
        "nav text changed",
        Some("navigation"),
        Some("Heading One"),
    ));

    write_diff_result(&dir, &result);

    let out = run_show(&["--section", "main", "--out", dir.path().to_str().unwrap()]);

    assert_eq!(
        out.status.code(),
        Some(0),
        "exit 0 expected, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("issue_main_h1_001"),
        "superset must include h1 issue, got: {}",
        stdout
    );
    assert!(
        stdout.contains("issue_main_h2_001"),
        "superset must include h2 issue, got: {}",
        stdout
    );
    assert!(
        !stdout.contains("issue_nav_001"),
        "must NOT include nav issue, got: {}",
        stdout
    );
}

/// Section heading with spaces — passed as a single quoted arg.
#[test]
fn test_show_section_heading_with_spaces() {
    let dir = TempDir::new().unwrap();
    let mut result = make_empty_result();

    result.issues.push(make_issue(
        "issue_free_001",
        IssueType::ChangedCta,
        IssueSeverity::Warning,
        "CTA copy changed",
        Some("main"),
        Some("Start for free"),
    ));
    // Different heading.
    result.issues.push(make_issue(
        "issue_other_001",
        IssueType::ChangedText,
        IssueSeverity::Warning,
        "other section text",
        Some("main"),
        Some("Other"),
    ));

    write_diff_result(&dir, &result);

    // The heading "Start for free" is passed as a single argument (shell quoting
    // is handled by the parent — Command splits args individually).
    let out = run_show(&[
        "--section",
        "main",
        "--heading",
        "Start for free",
        "--out",
        dir.path().to_str().unwrap(),
    ]);

    assert_eq!(
        out.status.code(),
        Some(0),
        "exit 0 expected, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("issue_free_001"),
        "must contain the spaced-heading issue, got: {}",
        stdout
    );
    assert!(
        !stdout.contains("issue_other_001"),
        "must NOT contain other section issue, got: {}",
        stdout
    );
}

/// --cluster and --issue both resolve, exit 0.
#[test]
fn test_show_cluster_and_issue() {
    let dir = TempDir::new().unwrap();
    let mut result = make_empty_result();

    result.issues.push(make_issue(
        "issue_cl_001",
        IssueType::StyleChanged,
        IssueSeverity::Warning,
        "style changed A",
        Some("main"),
        Some("Hero"),
    ));
    result.issues.push(make_issue(
        "issue_cl_002",
        IssueType::StyleChanged,
        IssueSeverity::Warning,
        "style changed B",
        Some("main"),
        Some("Hero"),
    ));
    result.clusters = vec![Cluster {
        id: "cluster_aabbccdd001".to_string(),
        issue_ids: vec!["issue_cl_001".to_string(), "issue_cl_002".to_string()],
        shared_property: Some("color".to_string()),
        shared_landmark: None,
        summary: Some("2 style_changed share color".to_string()),
    }];

    result.issues.push(make_issue(
        "issue_single_001",
        IssueType::BrokenLink,
        IssueSeverity::Error,
        "single broken link",
        Some("main"),
        Some("Body"),
    ));

    write_diff_result(&dir, &result);

    // Test --cluster.
    let out_cl = run_show(&[
        "--cluster",
        "cluster_aabbccdd001",
        "--out",
        dir.path().to_str().unwrap(),
    ]);
    assert_eq!(
        out_cl.status.code(),
        Some(0),
        "cluster exit 0 expected, stderr: {}",
        String::from_utf8_lossy(&out_cl.stderr)
    );
    let stdout_cl = String::from_utf8_lossy(&out_cl.stdout);
    assert!(
        stdout_cl.contains("issue_cl_001"),
        "cluster output must contain issue_cl_001, got: {}",
        stdout_cl
    );
    assert!(
        stdout_cl.contains("issue_cl_002"),
        "cluster output must contain issue_cl_002, got: {}",
        stdout_cl
    );

    // Test --issue.
    let out_is = run_show(&[
        "--issue",
        "issue_single_001",
        "--out",
        dir.path().to_str().unwrap(),
    ]);
    assert_eq!(
        out_is.status.code(),
        Some(0),
        "issue exit 0 expected, stderr: {}",
        String::from_utf8_lossy(&out_is.stderr)
    );
    let stdout_is = String::from_utf8_lossy(&out_is.stdout);
    assert!(
        stdout_is.contains("issue_single_001"),
        "issue output must contain issue_single_001, got: {}",
        stdout_is
    );
    assert!(
        stdout_is.contains("single broken link"),
        "issue output must contain message, got: {}",
        stdout_is
    );
}

/// Unknown region handle → exit 2, stderr mentions "resolved to no issues".
#[test]
fn test_show_unknown_handle_exit2() {
    let dir = TempDir::new().unwrap();
    let result = make_empty_result();
    write_diff_result(&dir, &result);

    let out = run_show(&[
        "--region",
        "does_not_exist",
        "--out",
        dir.path().to_str().unwrap(),
    ]);

    assert_eq!(
        out.status.code(),
        Some(2),
        "exit 2 expected for unknown handle, got: {:?}",
        out.status.code()
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("resolved to no issues"),
        "stderr must mention 'resolved to no issues', got: {}",
        stderr
    );
}

/// Missing diff-result.json → non-zero exit, stderr mentions failed to read.
#[test]
fn test_show_missing_file_nonzero() {
    let dir = TempDir::new().unwrap();
    // Do NOT write any diff-result.json.

    let out = run_show(&["--region", "x", "--out", dir.path().to_str().unwrap()]);

    assert_ne!(
        out.status.code(),
        Some(0),
        "non-zero exit expected for missing file"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("failed to read"),
        "stderr must mention 'failed to read', got: {}",
        stderr
    );
}

/// diff-result.json with schemaVersion "2.0" → exit 2, stderr mentions schemaVersion/newer.
#[test]
fn test_show_newer_schema_exit2() {
    let dir = TempDir::new().unwrap();
    let mut result = make_empty_result();
    result.schema_version = "2.0".to_string();
    write_diff_result(&dir, &result);

    let out = run_show(&["--region", "x", "--out", dir.path().to_str().unwrap()]);

    assert_eq!(
        out.status.code(),
        Some(2),
        "exit 2 expected for newer schema, got: {:?}",
        out.status.code()
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("schemaVersion") || stderr.contains("newer"),
        "stderr must mention schemaVersion or newer, got: {}",
        stderr
    );
}

/// Point --out at the JSON file directly (not its directory) → exit 0, member id in stdout.
/// Exercises the `p.is_file()` true branch in run_show.
#[test]
fn test_show_direct_file_path() {
    let dir = TempDir::new().unwrap();
    let mut result = make_empty_result();

    let id1 = "issue_direct_001".to_string();
    result.issues.push(make_issue(
        &id1,
        IssueType::ChangedText,
        IssueSeverity::Warning,
        "direct file path test",
        Some("contentinfo"),
        Some("Footer"),
    ));
    let mut member_ids = vec![id1.clone()];
    member_ids.sort();
    result.regions = vec![make_region("contentinfo", 0.75, member_ids)];
    result.agent_summary.region_count = 1;

    // write_diff_result returns the path to the JSON file itself.
    let json_path = write_diff_result(&dir, &result);

    // Pass the direct path to the JSON file as --out (not the directory).
    let out = run_show(&[
        "--region",
        "contentinfo",
        "--out",
        json_path.to_str().unwrap(),
    ]);

    assert_eq!(
        out.status.code(),
        Some(0),
        "exit code must be 0 when --out points directly at the JSON file, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&id1),
        "stdout must contain the member id '{}', got: {}",
        id1,
        stdout
    );
}

/// diff-result.json with schemaVersion "abc" → exit 2, stderr contains "unrecognized schemaVersion".
/// Exercises the new None arm from Fix 1.
#[test]
fn test_show_malformed_schema_version() {
    let dir = TempDir::new().unwrap();
    let mut result = make_empty_result();
    result.schema_version = "abc".to_string();
    write_diff_result(&dir, &result);

    let out = run_show(&["--region", "contentinfo", "--out", dir.path().to_str().unwrap()]);

    assert_eq!(
        out.status.code(),
        Some(2),
        "exit 2 expected for malformed schemaVersion, got: {:?}",
        out.status.code()
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unrecognized schemaVersion"),
        "stderr must contain 'unrecognized schemaVersion', got: {}",
        stderr
    );
}

/// Same region handle run twice → identical stdout (determinism).
#[test]
fn test_show_deterministic() {
    let dir = TempDir::new().unwrap();
    let mut result = make_empty_result();

    let ids: Vec<String> = (0u8..3).map(|i| format!("issue_det_{i:016x}")).collect();
    for id in &ids {
        result.issues.push(make_issue(
            id,
            IssueType::ChangedText,
            IssueSeverity::Warning,
            &format!("det message for {}", id),
            Some("contentinfo"),
            Some("Section"),
        ));
    }
    let mut sorted_ids = ids.clone();
    sorted_ids.sort();
    result.regions = vec![make_region("contentinfo", 0.78, sorted_ids)];
    result.agent_summary.region_count = 1;

    write_diff_result(&dir, &result);

    let out_dir = dir.path().to_str().unwrap();
    let out1 = run_show(&["--region", "contentinfo", "--out", out_dir]);
    let out2 = run_show(&["--region", "contentinfo", "--out", out_dir]);

    assert_eq!(out1.status.code(), Some(0), "first run must succeed");
    assert_eq!(out2.status.code(), Some(0), "second run must succeed");
    assert_eq!(
        out1.stdout, out2.stdout,
        "stdout must be byte-identical across runs"
    );
}
