//! Integration tests for `matchy explain`.
//!
//! Drives the real binary via `std::process::Command` using the
//! `CARGO_BIN_EXE_matchy` env var (set by Cargo for integration tests).
//! No screenshots / PNGs are required — `explain` never touches screenshots.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Minimal bundle JSON builder (no screenshots needed for explain)
// ---------------------------------------------------------------------------

fn make_det_json() -> serde_json::Value {
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

fn make_node_json(
    id: &str,
    text: Option<&str>,
    role: Option<&str>,
    css_selector: Option<&str>,
    bbox: [i32; 4],
    seq_index: u32,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "kind": "button",
        "role": role,
        "text": text,
        "accName": null,
        "href": null,
        "imageAlt": null,
        "bbox": bbox,
        "seqIndex": seq_index,
        "anchors": {
            "text": text,
            "role": role,
            "href": null,
            "alt": null,
            "ariaLabel": null,
            "nearestHeading": null,
            "landmark": null,
            "ordinalInLandmark": null
        },
        "cssSelector": css_selector
    })
}

/// Build a full CaptureBundle JSON value with explicit computedStyles.
fn make_bundle_json(
    url: &str,
    nodes: Vec<serde_json::Value>,
    computed_styles: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": "1.0",
        "capturedAt": "2026-01-01T00:00:00Z",
        "viewport": {
            "name": "desktop",
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
        "determinism": make_det_json(),
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
        "computedStyles": computed_styles,
        "screenshots": {
            "fullPage": "desktop/old.png",
            "viewport": "desktop/old-vp.png"
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

/// Write a bundle JSON to `path`.
fn write_bundle(path: &Path, bundle: &serde_json::Value) {
    std::fs::write(path, serde_json::to_string_pretty(bundle).unwrap()).expect("write bundle");
}

/// Path to the matchy binary (set by Cargo for integration tests).
fn matchy_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_matchy"))
}

// ---------------------------------------------------------------------------
// Shared test fixture builder
// ---------------------------------------------------------------------------

/// Build old+new bundles in `tmp` with a CTA node that has a style change.
///
/// OLD: node_cta_old — text="Get started", css_selector=".cta", background-image=gradient, color=white
/// NEW: node_cta_new — text="Get started", css_selector=".cta", background-image=none, color=white
///
/// Returns (old_bundle_path, new_bundle_path).
fn setup_cta_pair(tmp: &TempDir) -> (PathBuf, PathBuf) {
    let old_cta_node = make_node_json(
        "node_cta_old",
        Some("Get started"),
        Some("button"),
        Some(".hero .cta"),
        [100, 200, 150, 50],
        3,
    );
    let new_cta_node = make_node_json(
        "node_cta_new",
        Some("Get started"),
        Some("button"),
        Some(".hero .cta"),
        [100, 200, 150, 50],
        3,
    );

    let old_styles = serde_json::json!({
        "node_cta_old": {
            "background-image": "linear-gradient(90deg, #0070f3, #00c6ff)",
            "color": "#ffffff",
            "font-family": "Inter, sans-serif"
        }
    });
    let new_styles = serde_json::json!({
        "node_cta_new": {
            "background-image": "none",
            "color": "#ffffff",
            "font-family": "Inter, sans-serif"
        }
    });

    let old_bundle = make_bundle_json("http://old.example.com/", vec![old_cta_node], old_styles);
    let new_bundle = make_bundle_json("http://new.example.com/", vec![new_cta_node], new_styles);

    let old_path = tmp.path().join("old.bundle.json");
    let new_path = tmp.path().join("new.bundle.json");
    write_bundle(&old_path, &old_bundle);
    write_bundle(&new_path, &new_bundle);

    (old_path, new_path)
}

// ---------------------------------------------------------------------------
// Helper: run matchy explain and capture stdout + stderr + exit code
// ---------------------------------------------------------------------------

struct ExplainOutput {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

fn run_explain(old: &Path, new: &Path, extra: &[&str]) -> ExplainOutput {
    let output = Command::new(matchy_bin())
        .arg("explain")
        .arg("--old-bundle")
        .arg(old)
        .arg("--new-bundle")
        .arg(new)
        .args(extra)
        .output()
        .expect("failed to spawn matchy explain");

    ExplainOutput {
        exit_code: output.status.code().unwrap_or(127),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

// ---------------------------------------------------------------------------
// Test B1: anchor match → exit 0 + stdout contains changed property
// ---------------------------------------------------------------------------

#[test]
fn test_anchor_match_exits_zero_and_shows_changed_prop() {
    let tmp = TempDir::new().unwrap();
    let (old, new) = setup_cta_pair(&tmp);

    let out = run_explain(&old, &new, &["--anchor", "text=Get started"]);

    assert_eq!(
        out.exit_code, 0,
        "anchor match must exit 0 (got {}); stderr: {}",
        out.exit_code, out.stderr
    );

    // stdout must include the changed property
    assert!(
        out.stdout.contains("background-image"),
        "stdout must mention background-image; got:\n{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("CHANGED"),
        "stdout must mark a row CHANGED; got:\n{}",
        out.stdout
    );
    // The gradient value must appear on the old side
    assert!(
        out.stdout.contains("gradient"),
        "stdout must show gradient in old value; got:\n{}",
        out.stdout
    );
    // "none" must appear on the new side
    assert!(
        out.stdout.contains("none"),
        "stdout must show 'none' in new value; got:\n{}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// Test B2: non-matching anchor → exit 2 + "not found" on stderr
// ---------------------------------------------------------------------------

#[test]
fn test_nonmatch_anchor_exits_two_and_stderr_not_found() {
    let tmp = TempDir::new().unwrap();
    let (old, new) = setup_cta_pair(&tmp);

    let out = run_explain(
        &old,
        &new,
        &["--anchor", "text=ABSOLUTELY_NO_NODE_HAS_THIS_TEXT"],
    );

    assert_eq!(
        out.exit_code, 2,
        "non-matching anchor must exit 2 (got {}); stdout: {}",
        out.exit_code, out.stdout
    );
    let stderr_lower = out.stderr.to_lowercase();
    assert!(
        stderr_lower.contains("not found"),
        "stderr must contain 'not found'; got: {}",
        out.stderr
    );
}

// ---------------------------------------------------------------------------
// Test B3: --node locator resolves a node
// ---------------------------------------------------------------------------

#[test]
fn test_node_id_locator_exits_zero() {
    let tmp = TempDir::new().unwrap();
    let (old, new) = setup_cta_pair(&tmp);

    // node_cta_old exists in old bundle; not in new → one-side match → exit 0
    let out = run_explain(&old, &new, &["--node", "node_cta_old"]);
    assert_eq!(
        out.exit_code, 0,
        "--node match on old-only must exit 0 (got {}); stderr: {}",
        out.exit_code, out.stderr
    );
    // Asymmetry must be reported in stdout
    assert!(
        out.stdout.to_lowercase().contains("only"),
        "stdout must mention one-side-only asymmetry; got:\n{}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// Test B4: --selector locator resolves both sides
// ---------------------------------------------------------------------------

#[test]
fn test_selector_locator_resolves_both_sides() {
    let tmp = TempDir::new().unwrap();
    let (old, new) = setup_cta_pair(&tmp);

    // .hero .cta exists on both sides
    let out = run_explain(&old, &new, &["--selector", ".hero .cta"]);
    assert_eq!(
        out.exit_code, 0,
        "--selector match must exit 0 (got {}); stderr: {}",
        out.exit_code, out.stderr
    );
    assert!(
        out.stdout.contains("background-image"),
        "stdout must mention background-image; got:\n{}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// Test B5: --props restricts output to exactly those properties
// ---------------------------------------------------------------------------

#[test]
fn test_props_flag_restricts_output() {
    let tmp = TempDir::new().unwrap();
    let (old, new) = setup_cta_pair(&tmp);

    let out = run_explain(
        &old,
        &new,
        &[
            "--anchor",
            "text=Get started",
            "--props",
            "color,font-family",
        ],
    );
    assert_eq!(
        out.exit_code, 0,
        "--props run must exit 0 (got {}); stderr: {}",
        out.exit_code, out.stderr
    );

    // color and font-family must appear
    assert!(
        out.stdout.contains("color"),
        "stdout must contain 'color'; got:\n{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("font-family"),
        "stdout must contain 'font-family'; got:\n{}",
        out.stdout
    );

    // background-image must NOT appear (not in --props)
    assert!(
        !out.stdout.contains("background-image"),
        "stdout must NOT contain 'background-image' when not in --props; got:\n{}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// Test B6: output is deterministic — running twice on the same bundles yields
//          identical stdout.
// ---------------------------------------------------------------------------

#[test]
fn test_output_is_deterministic() {
    let tmp = TempDir::new().unwrap();
    let (old, new) = setup_cta_pair(&tmp);

    let out1 = run_explain(&old, &new, &["--anchor", "text=Get started"]);
    let out2 = run_explain(&old, &new, &["--anchor", "text=Get started"]);

    assert_eq!(out1.exit_code, out2.exit_code, "exit codes must match");
    assert_eq!(
        out1.stdout, out2.stdout,
        "stdout must be byte-identical across runs"
    );
}

// ---------------------------------------------------------------------------
// Test B7: missing bundle file → exit 2
// ---------------------------------------------------------------------------

#[test]
fn test_missing_bundle_exits_2() {
    let tmp = TempDir::new().unwrap();
    let nonexistent = tmp.path().join("does_not_exist.bundle.json");

    let out = run_explain(&nonexistent, &nonexistent, &["--anchor", "text=anything"]);
    assert_eq!(
        out.exit_code, 2,
        "missing bundle must exit 2 (got {}); stderr: {}",
        out.exit_code, out.stderr
    );
}
