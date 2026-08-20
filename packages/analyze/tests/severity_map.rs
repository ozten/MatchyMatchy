//! Integration tests for `--severity-map` (port-parity U3).
//!
//! Follows the `tests/analyze_cli.rs` convention: drives the real binary via
//! `std::process::Command` using `CARGO_BIN_EXE_matchy`. Fixture helpers are
//! duplicated here (each `tests/*.rs` file compiles as its own binary and
//! cannot import `lib.rs`'s `#[cfg(test)]` helpers).

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Minimal CaptureBundle builder
// ---------------------------------------------------------------------------

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

/// Build a semantic node JSON value carrying a `cssSelector` (needed so the
/// node id also works as a `computedStyles` key lookup target).
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
        "cssSelector": format!("#{}", id)
    })
}

/// Build a full CaptureBundle JSON value, with an explicit `computedStyles`
/// map (keyed by node id) — the field `make_bundle_json` in `analyze_cli.rs`
/// always leaves empty.
fn make_bundle_json(
    url: &str,
    viewport_name: &str,
    prefix: &str,
    nodes: Vec<serde_json::Value>,
    computed_styles: serde_json::Value,
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
        "computedStyles": computed_styles,
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

fn write_tiny_png(path: &Path) {
    use image::{ImageBuffer, Rgba};
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_fn(10, 10, |_, _| Rgba([255u8, 255, 255, 255]));
    img.save(path).expect("save PNG");
}

/// Set up a bundle pair whose single matched node ("node_0", identical text
/// both sides) differs in TWO computed-style properties: `letter-spacing`
/// (built-in-demoted to Info by default) and `color` (stays at profile
/// severity — Warning under content-structure). Returns (old, new, out).
fn setup_style_change_pair(tmp: &TempDir, viewport: &str) -> (PathBuf, PathBuf, PathBuf) {
    let vp_dir = tmp.path().join(viewport);
    std::fs::create_dir_all(&vp_dir).expect("create vp_dir");

    let old_bundle = make_bundle_json(
        "http://old.example.com/",
        viewport,
        "old",
        vec![make_node_json("node_0", "Hello world", 0)],
        serde_json::json!({
            "node_0": {
                "letter-spacing": "0.5px",
                "color": "rgb(0, 0, 0)"
            }
        }),
    );
    let new_bundle = make_bundle_json(
        "http://new.example.com/",
        viewport,
        "new",
        vec![make_node_json("node_0", "Hello world", 0)],
        serde_json::json!({
            "node_0": {
                "letter-spacing": "1px",
                "color": "rgb(255, 0, 0)"
            }
        }),
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

    write_tiny_png(&vp_dir.join("old.png"));
    write_tiny_png(&vp_dir.join("old-vp.png"));
    write_tiny_png(&vp_dir.join("new.png"));
    write_tiny_png(&vp_dir.join("new-vp.png"));

    let out_dir = tmp.path().join("out");
    std::fs::create_dir_all(&out_dir).expect("create out_dir");

    (old_path, new_path, out_dir)
}

/// A trivial bundle pair with zero issues — enough to exercise
/// `--severity-map` validation, which happens before any diffing.
fn setup_trivial_pair(tmp: &TempDir, viewport: &str) -> (PathBuf, PathBuf, PathBuf) {
    let vp_dir = tmp.path().join(viewport);
    std::fs::create_dir_all(&vp_dir).expect("create vp_dir");

    let old_bundle = make_bundle_json(
        "http://old.example.com/",
        viewport,
        "old",
        vec![make_node_json("node_0", "Hello world", 0)],
        serde_json::json!({}),
    );
    let new_bundle = make_bundle_json(
        "http://new.example.com/",
        viewport,
        "new",
        vec![make_node_json("node_0", "Hello world", 0)],
        serde_json::json!({}),
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

    write_tiny_png(&vp_dir.join("old.png"));
    write_tiny_png(&vp_dir.join("old-vp.png"));
    write_tiny_png(&vp_dir.join("new.png"));
    write_tiny_png(&vp_dir.join("new-vp.png"));

    let out_dir = tmp.path().join("out");
    std::fs::create_dir_all(&out_dir).expect("create out_dir");

    (old_path, new_path, out_dir)
}

fn matchy_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_matchy"))
}

/// Run `matchy analyze` with the given extra args; returns exit code.
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

/// Run `matchy analyze` and capture stderr too (for the exit-2 message checks).
fn run_analyze_capture(old: &Path, new: &Path, out: &Path, extra: &[&str]) -> (i32, String) {
    let output = Command::new(matchy_bin())
        .arg("analyze")
        .arg("--old-bundle")
        .arg(old)
        .arg("--new-bundle")
        .arg(new)
        .arg("--out")
        .arg(out)
        .args(extra)
        .output()
        .expect("failed to spawn matchy");
    (
        output.status.code().unwrap_or(127),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

fn write_map_file(tmp: &TempDir, name: &str, json: &serde_json::Value) -> PathBuf {
    let path = tmp.path().join(name);
    std::fs::write(&path, serde_json::to_string_pretty(json).unwrap()).unwrap();
    path
}

// ---------------------------------------------------------------------------
// Validation: unknown keys / malformed JSON -> exit 2
// ---------------------------------------------------------------------------

#[test]
fn test_unknown_type_key_exits_2() {
    let tmp = TempDir::new().unwrap();
    let (old, new, out) = setup_trivial_pair(&tmp, "desktop");
    let map_path = write_map_file(
        &tmp,
        "map.json",
        &serde_json::json!({ "types": { "not_a_real_type": "error" } }),
    );

    let (code, stderr) = run_analyze_capture(
        &old,
        &new,
        &out,
        &["--severity-map", map_path.to_str().unwrap()],
    );
    assert_eq!(code, 2, "unknown type key must exit 2; stderr: {}", stderr);
    assert!(
        stderr.contains("not_a_real_type"),
        "stderr must name the bad key; got: {}",
        stderr
    );
}

#[test]
fn test_unknown_property_key_exits_2() {
    let tmp = TempDir::new().unwrap();
    let (old, new, out) = setup_trivial_pair(&tmp, "desktop");
    let map_path = write_map_file(
        &tmp,
        "map.json",
        &serde_json::json!({ "properties": { "not-a-real-property": "info" } }),
    );

    let (code, stderr) = run_analyze_capture(
        &old,
        &new,
        &out,
        &["--severity-map", map_path.to_str().unwrap()],
    );
    assert_eq!(
        code, 2,
        "unknown property key must exit 2; stderr: {}",
        stderr
    );
    assert!(
        stderr.contains("not-a-real-property"),
        "stderr must name the bad key; got: {}",
        stderr
    );
}

#[test]
fn test_malformed_json_exits_2() {
    let tmp = TempDir::new().unwrap();
    let (old, new, out) = setup_trivial_pair(&tmp, "desktop");
    let map_path = tmp.path().join("bad.json");
    std::fs::write(&map_path, "{not valid json").unwrap();

    let (code, _stderr) = run_analyze_capture(
        &old,
        &new,
        &out,
        &["--severity-map", map_path.to_str().unwrap()],
    );
    assert_eq!(code, 2, "malformed JSON must exit 2");
}

// ---------------------------------------------------------------------------
// Deny-list
// ---------------------------------------------------------------------------

/// Demoting a hard-Critical type is ignored (never lowers the exit code
/// consequence) and surfaces a `severity_map_denied` warning.
#[test]
fn test_deny_list_denies_hard_critical_demotion() {
    let tmp = TempDir::new().unwrap();
    let (old, new, out) = setup_trivial_pair(&tmp, "desktop");
    let map_path = write_map_file(
        &tmp,
        "map.json",
        &serde_json::json!({ "types": { "status_code_mismatch": "info" } }),
    );

    let code = run_analyze(
        &old,
        &new,
        &out,
        &["--severity-map", map_path.to_str().unwrap()],
    );
    assert_eq!(code, 0, "a trivial zero-issue pair still exits 0");

    let dr: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("diff-result.json")).unwrap()).unwrap();
    let warnings = dr["warnings"].as_array().unwrap();
    let denied = warnings
        .iter()
        .find(|w| w["code"] == "severity_map_denied")
        .expect("severity_map_denied warning must be present");
    assert_eq!(denied["context"]["denied"]["status_code_mismatch"], "info");

    // The (accepted) echo must be empty — the denied entry is excluded.
    let echo = &dr["severityMap"];
    assert!(
        !echo.is_null(),
        "severityMap echo must be present when --severity-map is used"
    );
    assert_eq!(
        echo["overrides"]["types"].as_object().unwrap().len(),
        0,
        "the denied type must be excluded from the echo"
    );
}

// ---------------------------------------------------------------------------
// End-to-end: two maps -> different scores.style, each output carries its echo
// ---------------------------------------------------------------------------

#[test]
fn test_letter_spacing_demoted_color_stays_default_run() {
    let tmp = TempDir::new().unwrap();
    let (old, new, out) = setup_style_change_pair(&tmp, "desktop");

    let code = run_analyze(&old, &new, &out, &[]);
    let dr: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("diff-result.json")).unwrap()).unwrap();
    let issues = dr["issues"].as_array().unwrap();

    let letter_spacing_issue = issues
        .iter()
        .find(|i| i["remediation"]["property"] == "letter-spacing")
        .expect("letter-spacing style_changed issue must be present");
    assert_eq!(letter_spacing_issue["severity"], "info");

    let color_issue = issues
        .iter()
        .find(|i| i["remediation"]["property"] == "color")
        .expect("color style_changed issue must be present");
    assert_eq!(color_issue["severity"], "warning");

    // --fail-on warning must reflect the color issue (not the demoted letter-spacing one).
    let code_fail_on_warning = run_analyze(
        &old,
        &new,
        &tmp.path().join("out2"),
        &["--fail-on", "warning"],
    );
    assert_ne!(
        code_fail_on_warning, 0,
        "--fail-on warning must fail given the surviving warning-severity color issue"
    );
    let _ = code;
}

#[test]
fn test_two_maps_same_bundle_pair_different_style_scores_and_echo() {
    let tmp = TempDir::new().unwrap();
    let (old, new, out_a) = setup_style_change_pair(&tmp, "desktop");

    // Map A: no override (default built-ins only) -- run with no map at all.
    run_analyze(&old, &new, &out_a, &[]);
    let dr_a: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out_a.join("diff-result.json")).unwrap()).unwrap();
    assert!(
        dr_a["severityMap"].is_null(),
        "no --severity-map -> no echo"
    );

    // Map B: also demote color to info.
    let out_b = tmp.path().join("out_b");
    std::fs::create_dir_all(&out_b).unwrap();
    let map_path = write_map_file(
        &tmp,
        "map_b.json",
        &serde_json::json!({ "properties": { "color": "info" } }),
    );
    run_analyze(
        &old,
        &new,
        &out_b,
        &["--severity-map", map_path.to_str().unwrap()],
    );
    let dr_b: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out_b.join("diff-result.json")).unwrap()).unwrap();

    let style_a = dr_a["scores"]["style"].as_f64().unwrap();
    let style_b = dr_b["scores"]["style"].as_f64().unwrap();
    assert!(
        style_b > style_a,
        "demoting color to info must raise scores.style (a={}, b={})",
        style_a,
        style_b
    );

    // Map B's echo carries the accepted override.
    assert!(!dr_b["severityMap"].is_null());
    assert_eq!(dr_b["severityMap"]["source"], "file");
    assert_eq!(
        dr_b["severityMap"]["overrides"]["properties"]["color"],
        "info"
    );
}
