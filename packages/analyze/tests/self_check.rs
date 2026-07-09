//! Hermetic end-to-end tests for `matchy --self-check` through the REAL binary.
//!
//! NOTE: requires `node` on PATH (same requirement as running matchy for real — it
//! runs fine under `make verify`). No Chromium, Playwright, network, or testbed
//! servers are involved: the capture layer is replaced by a tiny stub Node script
//! (`stub_capture.cjs`, generated per-test into a TempDir) reached via the
//! `MATCHY_CAPTURE_PATH` env var honored by `resolve_capture_script`
//! (packages/analyze/src/orchestrate.rs). The stub bypasses the TS zod schema
//! entirely, which is exactly why U1's cross-layer vocabulary guard test
//! (packages/capture/tests/schema.test.ts) exists to pin the real schema side.
//!
//! Scenario control travels via a `control.json` file in a per-test "stub dir"
//! (env `MATCHY_TEST_STUB_DIR`), and the stub also appends every CaptureConfig
//! it receives (one JSON line each) to `configs.jsonl` in that same directory so
//! tests can assert what prefix/viewport the Rust runner actually sent.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Stub Node capture script
// ---------------------------------------------------------------------------

/// Write the reusable stub capture script into `dir` and return its path.
///
/// Behavior (see module docs): reads the full CaptureConfig JSON from stdin,
/// appends it as one line to `<MATCHY_TEST_STUB_DIR>/configs.jsonl`, then either
/// fails (per `control.json`'s `failPrefixes`/`failViewportPrefixes`) with the
/// same response shape `run_capture` (orchestrate.rs) treats as a capture failure,
/// or succeeds by writing a synthetic-but-valid CaptureBundle + screenshot PNGs
/// and printing the success CaptureResponse line.
fn write_stub_script(dir: &Path) -> PathBuf {
    let script = r#"#!/usr/bin/env node
'use strict';
const fs = require('fs');
const path = require('path');

function readStdin() {
  try {
    return fs.readFileSync(0, 'utf-8');
  } catch (e) {
    return '';
  }
}

// A minimal valid 10x10 white RGBA PNG, hand-built (zlib deflate + PNG chunk
// framing, no external tools/deps) and confirmed to decode via the Rust `image`
// crate the analyzer itself uses. Reused for every screenshot the stub writes —
// no visual diff is exercised by these tests.
const TINY_PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAoAAAAKCAYAAACNMs+9AAAAEUlEQVR42mP4TyRgGFVIX4UAI/uOgGWVNeQAAAAASUVORK5CYII=',
  'base64'
);

function makeNode(id, text, seqIndex) {
  return {
    id: id,
    kind: 'text',
    role: null,
    text: text,
    accName: null,
    href: null,
    imageAlt: null,
    bbox: [0, seqIndex * 100, 400, 50],
    seqIndex: seqIndex,
    anchors: {
      text: text,
      role: null,
      href: null,
      alt: null,
      ariaLabel: null,
      nearestHeading: null,
      landmark: null,
      ordinalInLandmark: null
    },
    cssSelector: null
  };
}

function buildBundle(config, prefix, viewportName, drift) {
  const nodes = [makeNode('node_0', 'Hello world', 0)];
  if (!drift) {
    nodes.push(makeNode('node_1', 'Static text', 1));
  }
  return {
    schemaVersion: '1.0',
    capturedAt: '2026-01-01T00:00:00Z',
    viewport: {
      name: viewportName,
      width: config.viewport.width,
      height: config.viewport.height,
      dsf: config.viewport.dsf
    },
    environment: {
      os: 'linux',
      chromiumBuild: '1234',
      playwright: '1.60.0',
      dsf: 1.0
    },
    determinism: {
      animationsDisabled: 'ran',
      reducedMotion: 'ran',
      timeFrozen: 'ran',
      randomStubbed: 'ran',
      fontsReady: 'ran',
      imagesDecoded: 'ran',
      lazyLoadPass: 'ran',
      settled: 'ran',
      clicked: [],
      hidden: [],
      masked: [],
      retriedWithoutTimeFreeze: false
    },
    page: {
      url: config.url,
      finalUrl: config.url,
      redirectChain: [],
      statusCode: 200,
      title: null,
      metaDescription: null,
      canonical: null,
      lang: 'en',
      pageHeight: 2000,
      nodes: nodes,
      landmarks: [],
      landmarkRects: null,
      network: { requests: [] },
      console: [],
      a11y: { violations: [] },
      linkProbes: []
    },
    computedStyles: {},
    screenshots: {
      fullPage: viewportName + '/' + prefix + '.png',
      viewport: viewportName + '/' + prefix + '-vp.png'
    },
    styleCandidates: {
      ancestors: [],
      chains: {},
      budget: 0,
      truncated: false,
      droppedCount: 0
    }
  };
}

function main() {
  const raw = readStdin();
  const config = JSON.parse(raw);

  const stubDir = process.env.MATCHY_TEST_STUB_DIR;
  if (!stubDir) {
    process.stdout.write(JSON.stringify({ ok: false, error: { code: 'INVALID_CONFIG', message: 'MATCHY_TEST_STUB_DIR not set' } }) + '\n');
    process.exit(1);
  }

  fs.mkdirSync(stubDir, { recursive: true });
  fs.appendFileSync(path.join(stubDir, 'configs.jsonl'), JSON.stringify(config) + '\n');

  let control = { failPrefixes: [], failViewportPrefixes: [], driftPrefixes: [], driftViewportPrefixes: [] };
  const controlPath = path.join(stubDir, 'control.json');
  if (fs.existsSync(controlPath)) {
    control = Object.assign(control, JSON.parse(fs.readFileSync(controlPath, 'utf-8')));
  }

  const prefix = config.prefix || 'unnamed';
  const viewportName = config.viewport.name;
  const key = viewportName + ':' + prefix;

  const shouldFail = control.failPrefixes.includes(prefix) || control.failViewportPrefixes.includes(key);
  if (shouldFail) {
    process.stdout.write(JSON.stringify({ ok: false, error: { code: 'INVALID_CONFIG', message: 'stub-forced failure' } }) + '\n');
    process.exit(1);
  }

  const outDir = config.outDir;
  const vpDir = path.join(outDir, viewportName);
  fs.mkdirSync(vpDir, { recursive: true });

  const drift = control.driftPrefixes.includes(prefix) || control.driftViewportPrefixes.includes(key);

  const bundle = buildBundle(config, prefix, viewportName, drift);
  const bundlePath = path.join(vpDir, prefix + '.bundle.json');
  fs.writeFileSync(bundlePath, JSON.stringify(bundle));
  fs.writeFileSync(path.join(vpDir, prefix + '.png'), TINY_PNG);
  fs.writeFileSync(path.join(vpDir, prefix + '-vp.png'), TINY_PNG);

  process.stdout.write(JSON.stringify({ ok: true, bundlePath: bundlePath }) + '\n');
  process.exit(0);
}

main();
"#;
    let script_path = dir.join("stub_capture.cjs");
    fs::write(&script_path, script).expect("write stub_capture.cjs");
    script_path
}

/// Write the scenario-control file the stub reads (`control.json`).
fn write_control(stub_dir: &Path, control: &serde_json::Value) {
    fs::write(
        stub_dir.join("control.json"),
        serde_json::to_string(control).unwrap(),
    )
    .expect("write control.json");
}

/// Read and parse every line of `configs.jsonl` (one CaptureConfig JSON object per line).
fn read_configs(stub_dir: &Path) -> Vec<serde_json::Value> {
    let path = stub_dir.join("configs.jsonl");
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e));
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("configs.jsonl line must be valid JSON"))
        .collect()
}

// ---------------------------------------------------------------------------
// matchy binary invocation
// ---------------------------------------------------------------------------

fn matchy_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_matchy"))
}

struct RunOutcome {
    code: i32,
    #[allow(dead_code)]
    stdout: String,
    stderr: String,
}

/// Run `matchy --old <old_url> --new <new_url> --out <out_dir> <extra_args...>` with
/// `MATCHY_CAPTURE_PATH` pointed at the stub and `MATCHY_TEST_STUB_DIR` pointed at
/// `stub_dir` (inherited by the stub's `node` child process).
fn run_matchy(
    capture_script: &Path,
    stub_dir: &Path,
    old_url: &str,
    new_url: &str,
    out_dir: &Path,
    extra_args: &[&str],
) -> RunOutcome {
    let output = Command::new(matchy_bin())
        .arg("--old")
        .arg(old_url)
        .arg("--new")
        .arg(new_url)
        .arg("--out")
        .arg(out_dir)
        .args(extra_args)
        .env("MATCHY_CAPTURE_PATH", capture_script)
        .env("MATCHY_TEST_STUB_DIR", stub_dir)
        .output()
        .expect("failed to spawn matchy");

    RunOutcome {
        code: output.status.code().unwrap_or(127),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

/// Read `<out_dir>/diff-result.json` and return the parsed `warnings[]` array.
fn read_warnings(out_dir: &Path) -> Vec<serde_json::Value> {
    let dr_path = out_dir.join("diff-result.json");
    let bytes = fs::read(&dr_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", dr_path.display(), e));
    let dr: serde_json::Value =
        serde_json::from_slice(&bytes).expect("diff-result.json must parse");
    dr["warnings"]
        .as_array()
        .cloned()
        .expect("diff-result.json must have a warnings array")
}

fn warning_codes(warnings: &[serde_json::Value]) -> Vec<&str> {
    warnings.iter().filter_map(|w| w["code"].as_str()).collect()
}

fn find_warning<'a>(
    warnings: &'a [serde_json::Value],
    code: &str,
) -> Option<&'a serde_json::Value> {
    warnings.iter().find(|w| w["code"] == code)
}

// ---------------------------------------------------------------------------
// Scenario 1: happy path — all captures succeed, self-check bundle identical
// to old bundle (no drift).
//
// Uses a single explicit viewport so the capture sequence is exactly
// [old, new, old-selfcheck] — this test's configs.jsonl doubles as the fixture
// for the prefix-regression guard (scenario 5, below).
// ---------------------------------------------------------------------------

#[test]
fn self_check_happy_path_no_drift_and_prefix_sequence() {
    let tmp = TempDir::new().unwrap();
    let stub_dir = tmp.path().join("stub");
    fs::create_dir_all(&stub_dir).unwrap();
    let capture_script = write_stub_script(&stub_dir);
    write_control(&stub_dir, &serde_json::json!({}));

    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();

    let outcome = run_matchy(
        &capture_script,
        &stub_dir,
        "http://old.example.com/",
        "http://new.example.com/",
        &out,
        &["--self-check", "--viewport", "only=800x600"],
    );
    assert_eq!(
        outcome.code, 0,
        "expected exit 0 on a clean self-check run; stderr:\n{}",
        outcome.stderr
    );

    let sc_path = out.join("self-check.json");
    assert!(sc_path.exists(), "self-check.json should be written");
    let sc_bytes = fs::read(&sc_path).unwrap();
    let _sc: serde_json::Value =
        serde_json::from_slice(&sc_bytes).expect("self-check.json must parse as JSON");

    let warnings = read_warnings(&out);
    let codes = warning_codes(&warnings);
    assert!(
        !codes.contains(&"volatile_capture"),
        "no volatile_capture expected when self-check bundle is identical: {:?}",
        codes
    );
    assert!(
        !codes.contains(&"self_check_failed"),
        "no self_check_failed expected on a fully healthy self-check run: {:?}",
        codes
    );

    // ---- Scenario 5: prefix-regression guard -----------------------------
    // Reuses this run's configs.jsonl: exactly 3 captures were requested
    // (single viewport), in order old, new, old-selfcheck. Pins the literal
    // the Rust runner sends end-to-end (run_self_check in
    // packages/analyze/src/bin/matchy.rs).
    let configs = read_configs(&stub_dir);
    assert_eq!(
        configs.len(),
        3,
        "expected exactly 3 capture configs for a single-viewport self-check run: {:#?}",
        configs
    );
    let prefixes: Vec<&str> = configs
        .iter()
        .map(|c| c["prefix"].as_str().expect("prefix must be a string"))
        .collect();
    assert_eq!(
        prefixes,
        vec!["old", "new", "old-selfcheck"],
        "capture prefix sequence must be old, new, old-selfcheck"
    );
}

// ---------------------------------------------------------------------------
// Scenario 2: drift — self-check bundle differs meaningfully from old (one
// node dropped), producing >=1 issue in the old-vs-selfcheck diff.
// ---------------------------------------------------------------------------

#[test]
fn self_check_drift_produces_volatile_capture_warning() {
    let tmp = TempDir::new().unwrap();
    let stub_dir = tmp.path().join("stub");
    fs::create_dir_all(&stub_dir).unwrap();
    let capture_script = write_stub_script(&stub_dir);
    write_control(
        &stub_dir,
        &serde_json::json!({ "driftPrefixes": ["old-selfcheck"] }),
    );

    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();

    let outcome = run_matchy(
        &capture_script,
        &stub_dir,
        "http://old.example.com/",
        "http://new.example.com/",
        &out,
        &["--self-check", "--viewport", "only=800x600"],
    );
    assert_eq!(
        outcome.code, 0,
        "self-check drift must not affect exit code; stderr:\n{}",
        outcome.stderr
    );

    assert!(
        out.join("self-check.json").exists(),
        "self-check.json should still be written when drift is found"
    );

    let warnings = read_warnings(&out);
    let vol = find_warning(&warnings, "volatile_capture")
        .unwrap_or_else(|| panic!("volatile_capture warning expected; got {:?}", warnings));
    let issue_count = vol["context"]["issueCount"]
        .as_u64()
        .expect("context.issueCount must be a number");
    assert!(
        issue_count >= 1,
        "context.issueCount must be >= 1, got {}",
        issue_count
    );

    let codes = warning_codes(&warnings);
    assert!(
        !codes.contains(&"self_check_failed"),
        "no self_check_failed expected when the probe merely finds drift: {:?}",
        codes
    );
}

// ---------------------------------------------------------------------------
// Scenario 3: total failure — the stub fails every self-check capture
// (prefix "old-selfcheck", both default viewports). The run completes with
// the same exit code as an equivalent run without --self-check; no
// self-check.json; self_check_failed lists both viewports at stage "capture";
// stderr carries the "[self-check]" line.
// ---------------------------------------------------------------------------

#[test]
fn self_check_total_failure_is_visible_but_exit_code_unchanged() {
    let tmp = TempDir::new().unwrap();
    let stub_dir = tmp.path().join("stub");
    fs::create_dir_all(&stub_dir).unwrap();
    let capture_script = write_stub_script(&stub_dir);
    write_control(
        &stub_dir,
        &serde_json::json!({ "failPrefixes": ["old-selfcheck"] }),
    );

    let out_with = tmp.path().join("out_with_self_check");
    fs::create_dir_all(&out_with).unwrap();
    let with_sc = run_matchy(
        &capture_script,
        &stub_dir,
        "http://old.example.com/",
        "http://new.example.com/",
        &out_with,
        &["--self-check"],
    );

    let out_without = tmp.path().join("out_without_self_check");
    fs::create_dir_all(&out_without).unwrap();
    let without_sc = run_matchy(
        &capture_script,
        &stub_dir,
        "http://old.example.com/",
        "http://new.example.com/",
        &out_without,
        &[],
    );

    assert_eq!(
        with_sc.code, without_sc.code,
        "self-check failure must never change the exit code (with={}, without={})\n\
         --self-check stderr:\n{}\nno-self-check stderr:\n{}",
        with_sc.code, without_sc.code, with_sc.stderr, without_sc.stderr
    );

    assert!(
        !out_with.join("self-check.json").exists(),
        "self-check.json must not be written when every viewport's self-check capture fails"
    );

    let warnings = read_warnings(&out_with);
    let sc_failed = find_warning(&warnings, "self_check_failed")
        .unwrap_or_else(|| panic!("self_check_failed warning expected; got {:?}", warnings));
    let failed_viewports = sc_failed["context"]["failedViewports"]
        .as_object()
        .expect("context.failedViewports must be an object");
    // Default viewports: desktop + mobile (parse_viewports with no --viewport args).
    assert_eq!(
        failed_viewports.len(),
        2,
        "both default viewports should be recorded as failed: {:?}",
        failed_viewports
    );
    for vp_name in ["desktop", "mobile"] {
        assert_eq!(
            failed_viewports.get(vp_name).and_then(|v| v.as_str()),
            Some("capture"),
            "viewport '{}' should be recorded at stage 'capture': {:?}",
            vp_name,
            failed_viewports
        );
    }

    assert!(
        with_sc.stderr.contains("[self-check]"),
        "stderr should contain a [self-check] diagnostic line:\n{}",
        with_sc.stderr
    );
}

// ---------------------------------------------------------------------------
// Scenario 4: partial failure — two viewports (default desktop + mobile), the
// "mobile" self-check capture fails while "desktop" succeeds AND drifts.
// self-check.json is written from the survivor; self_check_failed lists only
// "mobile"; volatile_capture and self_check_failed coexist, in that order
// (extra-warnings block: volatile_capture before self_check_failed, matching
// test_extra_warnings_appended_last in packages/analyze/src/report/json.rs).
// ---------------------------------------------------------------------------

#[test]
fn self_check_partial_failure_one_viewport_survives_with_drift() {
    let tmp = TempDir::new().unwrap();
    let stub_dir = tmp.path().join("stub");
    fs::create_dir_all(&stub_dir).unwrap();
    let capture_script = write_stub_script(&stub_dir);
    write_control(
        &stub_dir,
        &serde_json::json!({
            "failViewportPrefixes": ["mobile:old-selfcheck"],
            "driftViewportPrefixes": ["desktop:old-selfcheck"],
        }),
    );

    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();

    // Default viewports (desktop + mobile) — no --viewport flag.
    let outcome = run_matchy(
        &capture_script,
        &stub_dir,
        "http://old.example.com/",
        "http://new.example.com/",
        &out,
        &["--self-check"],
    );
    assert_eq!(
        outcome.code, 0,
        "partial self-check failure must not affect exit code; stderr:\n{}",
        outcome.stderr
    );

    assert!(
        out.join("self-check.json").exists(),
        "self-check.json should be written from the surviving viewport (desktop)"
    );

    let warnings = read_warnings(&out);

    let sc_failed = find_warning(&warnings, "self_check_failed")
        .unwrap_or_else(|| panic!("self_check_failed warning expected; got {:?}", warnings));
    let failed_viewports = sc_failed["context"]["failedViewports"]
        .as_object()
        .expect("context.failedViewports must be an object");
    assert_eq!(
        failed_viewports.len(),
        1,
        "only the failed viewport should be listed: {:?}",
        failed_viewports
    );
    assert_eq!(
        failed_viewports.get("mobile").and_then(|v| v.as_str()),
        Some("capture"),
        "mobile should be recorded at stage 'capture': {:?}",
        failed_viewports
    );
    assert!(
        !failed_viewports.contains_key("desktop"),
        "desktop must not be listed as failed: {:?}",
        failed_viewports
    );

    let vol = find_warning(&warnings, "volatile_capture").unwrap_or_else(|| {
        panic!(
            "volatile_capture warning expected from the surviving (drifted) viewport; got {:?}",
            warnings
        )
    });
    let issue_count = vol["context"]["issueCount"]
        .as_u64()
        .expect("context.issueCount must be a number");
    assert!(
        issue_count >= 1,
        "context.issueCount must be >= 1, got {}",
        issue_count
    );

    // Ordering: volatile_capture must precede self_check_failed in warnings[].
    let codes = warning_codes(&warnings);
    let vol_idx = codes
        .iter()
        .position(|c| *c == "volatile_capture")
        .expect("volatile_capture must be present");
    let failed_idx = codes
        .iter()
        .position(|c| *c == "self_check_failed")
        .expect("self_check_failed must be present");
    assert!(
        vol_idx < failed_idx,
        "volatile_capture must precede self_check_failed in warnings[]: {:?}",
        codes
    );
}
