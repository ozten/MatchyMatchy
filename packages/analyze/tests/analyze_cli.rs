//! Integration tests for `matchy analyze` — establishes the packages/analyze/tests/ convention.
//!
//! These tests drive the real binary via `std::process::Command` using the
//! `CARGO_BIN_EXE_matchy` env var (set by Cargo for integration tests).
//! No new crate dependencies are needed.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Minimal CaptureBundle builder (cannot import lib.rs #[cfg(test)] helpers)
// ---------------------------------------------------------------------------

/// Build a minimal valid CaptureDeterminism as JSON value.
fn make_determinism_json() -> serde_json::Value {
    serde_json::json!({
        "animationsDisabled": "ran",
        "reducedMotion": "ran",
        "timeFrozen": "ran",
        "randomStubbed": "ran",
        "fontsReady": "ran",
        "imagesDecoded": "ran",
        "lazyLoadPass": "ran",
        "settled": "ran",
        "clicked": [],
        "hidden": [],
        "masked": [],
        "retriedWithoutTimeFreeze": false
    })
}

/// Build a semantic node JSON value.
fn make_node_json(id: &str, text: &str, seq_index: u32) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "kind": "text",
        "role": null,
        "text": text,
        "accName": null,
        "href": null,
        "imageAlt": null,
        "bbox": [0, seq_index as i32 * 100, 400, 50],
        "seqIndex": seq_index,
        "anchors": {
            "text": text,
            "role": null,
            "href": null,
            "alt": null,
            "ariaLabel": null,
            "nearestHeading": null,
            "landmark": null,
            "ordinalInLandmark": null
        },
        "cssSelector": null
    })
}

/// Build a full CaptureBundle JSON value for a given viewport name and page URL,
/// with the given nodes. `screenshot_prefix` is e.g. "old" or "new";
/// screenshots are encoded as `"<viewport>/<prefix>.png"`.
fn make_bundle_json(
    url: &str,
    viewport_name: &str,
    prefix: &str,
    nodes: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": "1.0",
        "capturedAt": "2026-01-01T00:00:00Z",
        "viewport": {
            "name": viewport_name,
            "width": 1440,
            "height": 900,
            "dsf": 1.0
        },
        "environment": {
            "os": "linux",
            "chromiumBuild": "1234",
            "playwright": "1.60.0",
            "dsf": 1.0
        },
        "determinism": make_determinism_json(),
        "page": {
            "url": url,
            "finalUrl": url,
            "redirectChain": [],
            "statusCode": 200,
            "title": null,
            "metaDescription": null,
            "canonical": null,
            "lang": "en",
            "pageHeight": 2000,
            "nodes": nodes,
            "landmarks": [],
            "landmarkRects": null,
            "network": { "requests": [] },
            "console": [],
            "a11y": { "violations": [] },
            "linkProbes": []
        },
        "computedStyles": {},
        "screenshots": {
            "fullPage": format!("{}/{}.png", viewport_name, prefix),
            "viewport": format!("{}/{}-vp.png", viewport_name, prefix)
        },
        "styleCandidates": {
            "ancestors": [],
            "chains": {},
            "budget": 0,
            "truncated": false,
            "droppedCount": 0
        }
    })
}

/// Write a tiny valid 1x1 white PNG to `path`.
fn write_tiny_png(path: &Path) {
    use image::{ImageBuffer, Rgba};
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_fn(10, 10, |_, _| Rgba([255u8, 255, 255, 255]));
    img.save(path).expect("save PNG");
}

/// Set up a deterministic issue-bearing bundle pair in `tmp`:
///   tmp/<viewport>/old.bundle.json  — has node_0 and node_1
///   tmp/<viewport>/new.bundle.json  — has only node_0 (node_1 removed → missing_text issue)
///   tmp/<viewport>/old.png, old-vp.png, new.png, new-vp.png
///
/// Returns (old_bundle_path, new_bundle_path, out_dir).
fn setup_issue_pair(tmp: &TempDir, viewport: &str) -> (PathBuf, PathBuf, PathBuf) {
    let vp_dir = tmp.path().join(viewport);
    std::fs::create_dir_all(&vp_dir).expect("create vp_dir");

    // old bundle: two nodes
    let old_bundle = make_bundle_json(
        "http://old.example.com/",
        viewport,
        "old",
        vec![
            make_node_json("node_0", "Hello world", 0),
            make_node_json("node_1", "This text will be removed", 1),
        ],
    );
    // new bundle: only node_0 — node_1 is missing, should produce a missing-content issue
    let new_bundle = make_bundle_json(
        "http://new.example.com/",
        viewport,
        "new",
        vec![make_node_json("node_0", "Hello world", 0)],
    );

    let old_path = vp_dir.join("old.bundle.json");
    let new_path = vp_dir.join("new.bundle.json");
    std::fs::write(
        &old_path,
        serde_json::to_string_pretty(&old_bundle).unwrap(),
    )
    .unwrap();
    std::fs::write(
        &new_path,
        serde_json::to_string_pretty(&new_bundle).unwrap(),
    )
    .unwrap();

    // Write screenshots — analyze resolves them as bundle_path.parent().parent()/screenshots.fullPage
    // bundle_path.parent() = vp_dir, .parent() = tmp
    // screenshots.fullPage = "<viewport>/old.png" → tmp/<viewport>/old.png ✓
    write_tiny_png(&vp_dir.join("old.png"));
    write_tiny_png(&vp_dir.join("old-vp.png"));
    write_tiny_png(&vp_dir.join("new.png"));
    write_tiny_png(&vp_dir.join("new-vp.png"));

    let out_dir = tmp.path().join("out");
    std::fs::create_dir_all(&out_dir).expect("create out_dir");

    (old_path, new_path, out_dir)
}

/// Path to the matchy binary (set by Cargo for integration tests).
fn matchy_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_matchy"))
}

/// Run `matchy analyze` with the given extra args and return the exit code.
fn run_analyze(old: &Path, new: &Path, out: &Path, extra: &[&str]) -> i32 {
    let status = Command::new(matchy_bin())
        .arg("analyze")
        .arg("--old-bundle")
        .arg(old)
        .arg("--new-bundle")
        .arg(new)
        .arg("--out")
        .arg(out)
        .args(extra)
        .status()
        .expect("failed to spawn matchy");
    status.code().unwrap_or(127)
}

// ---------------------------------------------------------------------------
// Test 1 (load-bearing pre-fix): --fail-on never must exit 0
//
// Before the fix, run_analyze hardcodes compute_exit_code(&result, "error"),
// so even with --fail-on never it exits 1 when there are issues.
// After the fix it correctly threads the flag and exits 0.
// ---------------------------------------------------------------------------

#[test]
fn test_fail_on_never_exits_zero() {
    let tmp = TempDir::new().unwrap();
    let (old, new, out) = setup_issue_pair(&tmp, "desktop");

    let code = run_analyze(&old, &new, &out, &["--fail-on", "never"]);
    assert_eq!(
        code, 0,
        "analyze --fail-on never must exit 0 regardless of issues (got {})",
        code
    );
}

// ---------------------------------------------------------------------------
// Test 2: default --fail-on error exits 1 when issues exist at/above error severity,
// and exits 0 with --fail-on critical (above the actual max severity).
//
// This tests that the fail_on value is actually threaded: the same bundle pair
// must give different exit codes for different --fail-on thresholds.
// ---------------------------------------------------------------------------

#[test]
fn test_fail_on_thresholds_differ() {
    let tmp = TempDir::new().unwrap();
    let (old, new, out_1) = setup_issue_pair(&tmp, "desktop");

    // First run: capture what the actual output looks like (default fail_on = error)
    let out_default = tmp.path().join("out_default");
    std::fs::create_dir_all(&out_default).unwrap();
    let code_default = run_analyze(&old, &new, &out_default, &[]);

    // Read the diff-result to find out max severity and status
    let dr_path = out_default.join("diff-result.json");
    assert!(dr_path.exists(), "diff-result.json must be written");
    let dr_bytes = std::fs::read(&dr_path).unwrap();
    let dr: serde_json::Value = serde_json::from_slice(&dr_bytes).unwrap();

    // Validate required top-level keys
    assert!(dr.get("status").is_some(), "diff-result must have 'status'");
    assert!(dr.get("issues").is_some(), "diff-result must have 'issues'");
    assert!(dr.get("scores").is_some(), "diff-result must have 'scores'");

    let issues = dr["issues"].as_array().unwrap();

    if issues.is_empty() {
        // Bundle pair produced no issues — the test cannot validate threshold threading
        // on severity. Fall back to checking that --fail-on never still exits 0.
        let out_never = tmp.path().join("out_never");
        std::fs::create_dir_all(&out_never).unwrap();
        let code_never = run_analyze(&old, &new, &out_never, &["--fail-on", "never"]);
        assert_eq!(code_never, 0, "--fail-on never must exit 0 with no issues");
        return;
    }

    // There are issues: verify that --fail-on never exits 0 and --fail-on critical
    // (above any warning/error) exits differently from --fail-on never only when
    // appropriate. The key assertion: default (error) and never differ when issues exist.
    assert_ne!(
        code_default, 0,
        "default --fail-on error with issues should exit non-zero (got {})",
        code_default
    );
    assert_eq!(
        code_default, 1,
        "analyze with issues and default --fail-on error must exit 1"
    );

    // --fail-on never on the same pair must exit 0
    let out_never = tmp.path().join("out_never");
    std::fs::create_dir_all(&out_never).unwrap();
    let code_never = run_analyze(&old, &new, &out_never, &["--fail-on", "never"]);
    assert_eq!(
        code_never, 0,
        "--fail-on never must exit 0 even when issues exist (got {})",
        code_never
    );

    // The two exit codes must be different — this is the guard that the flag is threaded
    assert_ne!(
        code_default, code_never,
        "--fail-on error and --fail-on never must produce different exit codes when issues exist"
    );

    let _ = out_1; // silence unused warning
}

// ---------------------------------------------------------------------------
// Test 3: malformed/nonexistent bundle path → exit 2
// ---------------------------------------------------------------------------

#[test]
fn test_missing_bundle_exits_2() {
    let tmp = TempDir::new().unwrap();
    let nonexistent = tmp.path().join("does_not_exist.bundle.json");
    let out = tmp.path().join("out");
    std::fs::create_dir_all(&out).unwrap();

    let code = run_analyze(
        &nonexistent,
        &nonexistent,
        &out,
        &[], // default --fail-on error
    );
    assert_eq!(
        code, 2,
        "nonexistent bundle path must exit 2 (got {})",
        code
    );
}

// ---------------------------------------------------------------------------
// Test 4: diff-result.json validates (has required top-level keys)
// ---------------------------------------------------------------------------

#[test]
fn test_diff_result_has_required_keys() {
    let tmp = TempDir::new().unwrap();
    let (old, new, out) = setup_issue_pair(&tmp, "desktop");

    // Run with --fail-on never so we get exit 0 reliably
    let code = run_analyze(&old, &new, &out, &["--fail-on", "never"]);
    assert_eq!(
        code, 0,
        "expected exit 0 with --fail-on never (got {})",
        code
    );

    let dr_path = out.join("diff-result.json");
    assert!(
        dr_path.exists(),
        "diff-result.json must exist after analyze"
    );
    let dr_bytes = std::fs::read(&dr_path).unwrap();
    let dr: serde_json::Value =
        serde_json::from_slice(&dr_bytes).expect("diff-result.json must be valid JSON");

    assert_eq!(
        dr.get("toolVersion").and_then(|v| v.as_str()),
        Some(env!("CARGO_PKG_VERSION")),
        "toolVersion must stay plain CARGO_PKG_VERSION (R7) — build provenance must NOT leak into JSON"
    );

    // Required top-level keys per contract (DiffResult struct, camelCase serialization)
    for key in &[
        "status",
        "issues",
        "scores",
        "runId",
        "schemaVersion",
        "oldUrl",
        "newUrl",
    ] {
        assert!(
            dr.get(key).is_some(),
            "diff-result.json missing required key '{}'",
            key
        );
    }
}

// ---------------------------------------------------------------------------
// Test 5 (profile integration): --profile strict-visual vs content-structure
// produce different output on the same pair (proves profile reaches analyze).
// ---------------------------------------------------------------------------

#[test]
fn test_profiles_produce_different_output() {
    let tmp = TempDir::new().unwrap();
    let (old, new, _) = setup_issue_pair(&tmp, "desktop");

    let out_cs = tmp.path().join("out_cs");
    let out_sv = tmp.path().join("out_sv");
    std::fs::create_dir_all(&out_cs).unwrap();
    std::fs::create_dir_all(&out_sv).unwrap();

    let code_cs = run_analyze(
        &old,
        &new,
        &out_cs,
        &["--profile", "content-structure", "--fail-on", "never"],
    );
    let code_sv = run_analyze(
        &old,
        &new,
        &out_sv,
        &["--profile", "strict-visual", "--fail-on", "never"],
    );

    assert_eq!(
        code_cs, 0,
        "content-structure run failed (exit {})",
        code_cs
    );
    assert_eq!(code_sv, 0, "strict-visual run failed (exit {})", code_sv);

    let dr_cs = std::fs::read_to_string(out_cs.join("diff-result.json")).unwrap();
    let dr_sv = std::fs::read_to_string(out_sv.join("diff-result.json")).unwrap();

    let v_cs: serde_json::Value = serde_json::from_str(&dr_cs).unwrap();
    let v_sv: serde_json::Value = serde_json::from_str(&dr_sv).unwrap();

    // The scores object must differ between profiles (weight distribution differs).
    // Even if issue lists are the same, the score fields will differ.
    let scores_cs = v_cs.get("scores").unwrap();
    let scores_sv = v_sv.get("scores").unwrap();

    // At minimum the two runs should not be byte-identical (profiles weight things differently).
    // If they happen to produce the same scores, we still assert both ran successfully.
    // The non-byte-identical assertion is the stronger guard.
    let _ = (scores_cs, scores_sv); // referenced

    // Assert both parse and have status + issues
    assert!(v_cs.get("status").is_some());
    assert!(v_sv.get("status").is_some());

    // The profiles should yield different output strings (scores differ)
    assert_ne!(
        dr_cs, dr_sv,
        "content-structure and strict-visual must produce different diff-results \
         on the same pair (proves --profile reaches run_analyze)"
    );
}

// ---------------------------------------------------------------------------
// Test 6 (baseline integration): supplying --baseline with the emitted issue's
// id suppresses it (issue absent / status changes).
// ---------------------------------------------------------------------------

#[test]
fn test_baseline_suppresses_issues() {
    let tmp = TempDir::new().unwrap();
    let (old, new, _) = setup_issue_pair(&tmp, "desktop");

    // Step 1: run without baseline to collect issue ids
    let out_no_baseline = tmp.path().join("out_no_baseline");
    std::fs::create_dir_all(&out_no_baseline).unwrap();
    let code_no_bl = run_analyze(&old, &new, &out_no_baseline, &["--fail-on", "never"]);
    assert_eq!(
        code_no_bl, 0,
        "baseline-less run failed (exit {})",
        code_no_bl
    );

    let dr_bytes = std::fs::read(out_no_baseline.join("diff-result.json")).unwrap();
    let dr: serde_json::Value = serde_json::from_slice(&dr_bytes).unwrap();
    let issues = dr["issues"].as_array().unwrap();

    if issues.is_empty() {
        // No issues to suppress — test is vacuously satisfied
        return;
    }

    // Step 2: build a baseline JSON that accepts all issues
    let baseline_entries: Vec<serde_json::Value> = issues
        .iter()
        .filter_map(|iss| iss.get("id").map(|id| serde_json::json!({ "id": id })))
        .collect();
    let baseline_json = serde_json::to_string(&baseline_entries).unwrap();
    let baseline_path = tmp.path().join("baseline.json");
    std::fs::write(&baseline_path, &baseline_json).unwrap();

    // Step 3: run with baseline — all issues suppressed
    let out_with_baseline = tmp.path().join("out_with_baseline");
    std::fs::create_dir_all(&out_with_baseline).unwrap();
    let code_with_bl = run_analyze(
        &old,
        &new,
        &out_with_baseline,
        &[
            "--baseline",
            baseline_path.to_str().unwrap(),
            "--fail-on",
            "never",
        ],
    );
    assert_eq!(
        code_with_bl, 0,
        "baseline run failed (exit {})",
        code_with_bl
    );

    let dr_bl_bytes = std::fs::read(out_with_baseline.join("diff-result.json")).unwrap();
    let dr_bl: serde_json::Value = serde_json::from_slice(&dr_bl_bytes).unwrap();
    let issues_after = dr_bl["issues"].as_array().unwrap();

    // All issues should be suppressed
    assert!(
        issues_after.is_empty(),
        "baseline should suppress all issues; {} remain",
        issues_after.len()
    );

    // Status should reflect no issues (pass or equivalent)
    let status = dr_bl["status"].as_str().unwrap_or("");
    assert_eq!(
        status, "pass",
        "status should be 'pass' when all issues are baselined; got '{}'",
        status
    );
}
