//! Orchestration: spawn capture.cjs per page per viewport, parse bundles, run analysis.
//!
//! Capture.cjs resolution order (M1.md §5.5):
//!   1. MATCHY_CAPTURE_PATH env
//!   2. sibling of current_exe()
//!   3. packages/capture/dist/capture.cjs relative to CWD (dev)

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context};

use crate::contract::{
    CaptureBundle, CaptureConfig, CaptureResponse, StabilizationConfig, ViewportConfig,
};

/// Resolve the path to capture.cjs.
pub fn resolve_capture_script() -> anyhow::Result<PathBuf> {
    // 1. MATCHY_CAPTURE_PATH env
    if let Ok(env_path) = std::env::var("MATCHY_CAPTURE_PATH") {
        let p = PathBuf::from(env_path);
        if p.exists() {
            return Ok(p);
        }
    }

    // 2. Sibling of current_exe()
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("capture.cjs");
            if sibling.exists() {
                return Ok(sibling);
            }
        }
    }

    // 3. Dev: packages/capture/dist/capture.cjs relative to CWD
    let cwd = std::env::current_dir().context("failed to get CWD")?;
    let dev_path = cwd.join("packages/capture/dist/capture.cjs");
    if dev_path.exists() {
        return Ok(dev_path);
    }

    bail!(
        "capture.cjs not found. Set MATCHY_CAPTURE_PATH or ensure packages/capture/dist/capture.cjs exists.\n\
         Searched:\n  MATCHY_CAPTURE_PATH (not set or not found)\n  sibling of binary\n  {}",
        dev_path.display()
    )
}

/// Spawn capture.cjs and return the bundle path, or an error.
///
/// Returns Ok(bundle_path) or Err with a structured message.
pub fn run_capture(capture_script: &Path, config: &CaptureConfig) -> anyhow::Result<PathBuf> {
    let config_json = serde_json::to_string(config).context("failed to serialize CaptureConfig")?;

    let mut child = Command::new("node")
        .arg(capture_script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn node capture.cjs")?;

    // Write config to stdin
    {
        use std::io::Write;
        let stdin = child
            .stdin
            .as_mut()
            .context("failed to get capture stdin")?;
        stdin
            .write_all(config_json.as_bytes())
            .context("failed to write CaptureConfig to stdin")?;
    }

    let output = child
        .wait_with_output()
        .context("failed to wait for capture.cjs")?;

    if !output.status.success() && output.stdout.is_empty() {
        bail!(
            "capture.cjs exited with status {} and no output",
            output.status
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .next()
        .context("capture.cjs produced no output line")?;

    let response: CaptureResponse = serde_json::from_str(line)
        .with_context(|| format!("failed to parse capture.cjs response: {}", line))?;

    match response {
        CaptureResponse::Ok {
            ok: true,
            bundle_path,
        } => Ok(PathBuf::from(bundle_path)),
        CaptureResponse::Ok { ok: false, .. } => bail!("capture returned ok:false with no error"),
        CaptureResponse::Err { error, .. } => {
            bail!("capture failed: [{}] {}", error.code, error.message)
        }
    }
}

/// Load and parse a CaptureBundle from a JSON file.
pub fn load_bundle(bundle_path: &Path) -> anyhow::Result<CaptureBundle> {
    let content = std::fs::read_to_string(bundle_path)
        .with_context(|| format!("failed to read bundle: {}", bundle_path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse bundle: {}", bundle_path.display()))
}

/// Check if two environments have mismatched fingerprints.
pub fn env_mismatch(old_bundle: &CaptureBundle, new_bundle: &CaptureBundle) -> bool {
    old_bundle.environment.os != new_bundle.environment.os
        || old_bundle.environment.chromium_build != new_bundle.environment.chromium_build
        || old_bundle.environment.dsf != new_bundle.environment.dsf
}

/// Parameters for building a capture config.
pub struct CaptureConfigParams<'a> {
    pub url: &'a str,
    pub prefix: &'a str,
    pub out_dir: &'a Path,
    pub viewport: &'a ViewportConfig,
    pub freeze_time: bool,
    pub stub_random: bool,
    pub hide_selectors: &'a [String],
    pub mask_selectors: &'a [String],
    pub click_selectors: &'a [String],
}

/// Build the default capture config for a given URL, prefix, out_dir, and viewport.
pub fn build_capture_config(params: &CaptureConfigParams<'_>) -> CaptureConfig {
    let CaptureConfigParams {
        url,
        prefix,
        out_dir,
        viewport,
        freeze_time,
        stub_random,
        hide_selectors,
        mask_selectors,
        click_selectors,
    } = params;
    CaptureConfig {
        mode: "capture".to_string(),
        url: url.to_string(),
        out_dir: out_dir.to_string_lossy().to_string(),
        prefix: prefix.to_string(),
        viewport: (*viewport).clone(),
        stabilization: StabilizationConfig {
            freeze_time: *freeze_time,
            stub_random: *stub_random,
            ..Default::default()
        },
        hide_selectors: hide_selectors.to_vec(),
        mask_selectors: mask_selectors.to_vec(),
        click_before_capture: click_selectors.to_vec(),
        max_text_length: 500,
        redact_params: vec![
            "token".to_string(),
            "sig".to_string(),
            "signature".to_string(),
            "key".to_string(),
            "auth".to_string(),
            "apikey".to_string(),
            "access_token".to_string(),
        ],
        // Probe links for BOTH sides (M3.md D4: old-side probes feed broken_link parity —
        // pre-existing 404s on both sides are suppressed, keeping v01 at zero issues).
        probe_links: true,
    }
}
