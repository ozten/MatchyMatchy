//! Orchestration: spawn capture.cjs per page per viewport, parse bundles, run analysis.
//!
//! Capture.cjs resolution order (M1.md §5.5):
//!   1. MATCHY_CAPTURE_PATH env
//!   2. sibling of current_exe()
//!   3. ancestor walk — exe_dir, its parent, … up to root, checking
//!      <ancestor>/packages/capture/dist/capture.cjs each level
//!   4. packages/capture/dist/capture.cjs relative to CWD (dev)

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;

use anyhow::{bail, Context};

use crate::contract::{
    CaptureBundle, CaptureConfig, CaptureResponse, StabilizationConfig, ViewportConfig,
};

// ---------------------------------------------------------------------------
// Browser-not-found marker — shared across all capture attempts in this process.
// Stores the executable path from the first `Executable doesn't exist at <path>`
// message seen in any capture's stderr.
// ---------------------------------------------------------------------------
static BROWSER_NOT_FOUND_PATH: OnceLock<String> = OnceLock::new();
static BROWSER_REMEDY_PRINTED: OnceLock<()> = OnceLock::new();

/// Repo root found during capture-script resolution (for remedy messages).
static REPO_ROOT: OnceLock<PathBuf> = OnceLock::new();

// ---------------------------------------------------------------------------
// Candidate generation (pure, testable)
// ---------------------------------------------------------------------------

/// Generate candidate paths for capture.cjs, in resolution order.
///
/// Does NOT read `MATCHY_CAPTURE_PATH` — the caller handles that first.
///
/// Order:
///   1. `<exe_dir>/capture.cjs`   (sibling of binary)
///   2. For each ancestor of exe_dir (exe_dir, parent, parent-of-parent, … root):
///      `<ancestor>/packages/capture/dist/capture.cjs`
///   3. `<cwd>/packages/capture/dist/capture.cjs`
pub fn capture_script_candidates(exe: Option<&Path>, cwd: &Path) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(exe_path) = exe {
        if let Some(exe_dir) = exe_path.parent() {
            // 1. Sibling of binary
            candidates.push(exe_dir.join("capture.cjs"));

            // 2. Ancestor walk
            let mut ancestor: &Path = exe_dir;
            loop {
                candidates.push(ancestor.join("packages/capture/dist/capture.cjs"));
                match ancestor.parent() {
                    Some(p) if p != ancestor => ancestor = p,
                    _ => break,
                }
            }
        }
    }

    // 3. CWD-relative dev path
    candidates.push(cwd.join("packages/capture/dist/capture.cjs"));

    candidates
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// Resolve the path to capture.cjs.
///
/// Records the repo root (ancestor that owns `packages/capture/`) in
/// `REPO_ROOT` for use in remedy messages.
pub fn resolve_capture_script() -> anyhow::Result<PathBuf> {
    // 1. MATCHY_CAPTURE_PATH env
    if let Ok(env_path) = std::env::var("MATCHY_CAPTURE_PATH") {
        let p = PathBuf::from(env_path);
        if p.exists() {
            return Ok(p);
        }
    }

    let exe = std::env::current_exe().ok();
    let cwd = std::env::current_dir().context("failed to get CWD")?;
    let candidates = capture_script_candidates(exe.as_deref(), &cwd);

    // Probe each candidate
    for path in &candidates {
        if path.exists() {
            // Record repo root: the ancestor whose packages/capture dir we used.
            if let Some(parent) = path
                .ancestors()
                .find(|a| a.join("packages/capture").is_dir())
            {
                let _ = REPO_ROOT.set(parent.to_path_buf());
            }
            return Ok(path.clone());
        }
    }

    // Build error message
    let mut searched_lines = String::from("  MATCHY_CAPTURE_PATH (not set or not found)\n");
    for p in &candidates {
        searched_lines.push_str(&format!("  {}\n", p.display()));
    }

    // Find the first exe-ancestor that has a packages/capture directory (even without dist/).
    let hint = exe.as_deref().and_then(|e| e.parent()).and_then(|exe_dir| {
        let mut a: &Path = exe_dir;
        loop {
            if a.join("packages/capture").is_dir() {
                return Some(a.to_path_buf());
            }
            match a.parent() {
                Some(p) if p != a => a = p,
                _ => return None,
            }
        }
    });

    let remedy = match hint {
        Some(root) => format!(
            "  cd {} && make build\n  export MATCHY_CAPTURE_PATH={}/packages/capture/dist/capture.cjs",
            root.display(),
            root.display()
        ),
        None => "  export MATCHY_CAPTURE_PATH=/absolute/path/to/capture.cjs".to_string(),
    };

    bail!(
        "capture.cjs not found.\nSearched:\n{}Remedy:\n{}",
        searched_lines,
        remedy
    )
}

// ---------------------------------------------------------------------------
// Browser-revision remedy
// ---------------------------------------------------------------------------

/// Extract the Chromium build revision number from a headless-shell path segment.
///
/// Recognises `chromium_headless_shell-NNNN` and `chromium-NNNN`.
pub fn extract_chromium_rev(path: &str) -> Option<String> {
    for segment in path.split('/') {
        if let Some(rev) = segment
            .strip_prefix("chromium_headless_shell-")
            .or_else(|| segment.strip_prefix("chromium-"))
        {
            // rev may be followed by a slash (already split) but just in case:
            let rev = rev.split('/').next().unwrap_or(rev);
            if !rev.is_empty() && rev.chars().all(|c| c.is_ascii_digit()) {
                return Some(rev.to_string());
            }
        }
    }
    None
}

/// Format the browser-not-found remedy block.
///
/// `missing_path` — the executable path from the Playwright error.
/// `repo_root`    — resolved repo root (or None → use placeholder).
pub fn format_browser_remedy(missing_path: &str, repo_root: Option<&Path>) -> String {
    let rev = extract_chromium_rev(missing_path).unwrap_or_else(|| "unknown".to_string());
    let repo = repo_root
        .map(|r| r.display().to_string())
        .unwrap_or_else(|| "<repo-root>".to_string());

    format!(
        "Chromium build {} not found at:\n  {}\nInstall it with:\n  cd {}/packages/capture && PLAYWRIGHT_BROWSERS_PATH={}/.pw-browsers npx playwright install chromium\nThen run matchy with:\n  export PLAYWRIGHT_BROWSERS_PATH={}/.pw-browsers",
        rev, missing_path, repo, repo, repo
    )
}

// ---------------------------------------------------------------------------
// Playwright advisory-box filter
// ---------------------------------------------------------------------------

/// Returns true if the line is a box-drawing border line that starts an advisory box.
fn is_box_top(line: &str) -> bool {
    line.starts_with('╔')
}
fn is_box_bottom(line: &str) -> bool {
    line.starts_with('╚')
}
#[allow(dead_code)]
fn is_box_side(line: &str) -> bool {
    line.starts_with('║')
}
fn is_advisory_content(line: &str) -> bool {
    line.contains("Looks like Playwright")
        || line.contains("npx playwright install")
        || line.contains("Please run the following command")
}

/// Filter thread: reads piped stderr line-by-line, applies Playwright advisory
/// suppression, and writes surviving lines to the process's stderr.
///
/// The first `Executable doesn't exist at <path>` sets `BROWSER_NOT_FOUND_PATH`.
/// The first advisory box is replaced by a single note; subsequent advisory boxes
/// are silently dropped. Non-advisory content passes through unchanged.
fn run_stderr_filter(reader: impl std::io::Read + Send + 'static) {
    thread::spawn(move || {
        let buf = BufReader::new(reader);
        let mut in_box = false;
        let mut box_buf: Vec<String> = Vec::new();
        let mut advisory_emitted = false; // has the "note:" line been printed once

        for line_result in buf.lines() {
            let line = match line_result {
                Ok(l) => l,
                Err(_) => break,
            };

            // Detect "Executable doesn't exist at <path>"
            if let Some(rest) = line.strip_prefix("Executable doesn't exist at ") {
                let path_str = rest.trim().to_string();
                let _ = BROWSER_NOT_FOUND_PATH.set(path_str);
            }

            if is_box_top(&line) {
                // Start buffering a new box
                in_box = true;
                box_buf.clear();
                box_buf.push(line);
                continue;
            }

            if in_box {
                box_buf.push(line.clone());
                if is_box_bottom(&line) {
                    // Box complete — decide whether to emit it
                    let is_advisory = box_buf.iter().any(|l| is_advisory_content(l));
                    if is_advisory {
                        if !advisory_emitted {
                            advisory_emitted = true;
                            eprintln!("note: Playwright reported missing browsers (advisory suppressed; see remedy below)");
                        }
                        // else: silently drop
                    } else {
                        // Not an advisory — emit verbatim
                        for boxline in &box_buf {
                            eprintln!("{}", boxline);
                        }
                    }
                    in_box = false;
                    box_buf.clear();
                }
                continue;
            }

            // Normal line — pass through
            // (box-side lines outside a box block pass through too, unlikely but safe)
            eprintln!("{}", line);
        }

        // If we somehow ended while still in a box (truncated stream), emit it verbatim
        // unless it's advisory.
        if in_box && !box_buf.is_empty() {
            let is_advisory = box_buf.iter().any(|l| is_advisory_content(l));
            if !is_advisory {
                for boxline in &box_buf {
                    eprintln!("{}", boxline);
                }
            } else if !advisory_emitted {
                eprintln!("note: Playwright reported missing browsers (advisory suppressed; see remedy below)");
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Capture invocation
// ---------------------------------------------------------------------------

/// Spawn capture.cjs and return the bundle path, or an error.
///
/// Returns Ok(bundle_path) or Err with a structured message.
pub fn run_capture(capture_script: &Path, config: &CaptureConfig) -> anyhow::Result<PathBuf> {
    let config_json = serde_json::to_string(config).context("failed to serialize CaptureConfig")?;

    let mut child = Command::new("node")
        .arg(capture_script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn node capture.cjs")?;

    // Attach stderr filter thread
    if let Some(stderr) = child.stderr.take() {
        run_stderr_filter(stderr);
    }

    // Write config to stdin
    {
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
        // Emit browser-not-found remedy if applicable (once per process)
        maybe_print_browser_remedy();

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
        CaptureResponse::Ok { ok: false, .. } => {
            maybe_print_browser_remedy();
            bail!("capture returned ok:false with no error")
        }
        CaptureResponse::Err { error, .. } => {
            // Check for browser-not-found marker in the error message itself
            if error.message.contains("Executable doesn't exist at ") {
                if let Some(rest) = error
                    .message
                    .find("Executable doesn't exist at ")
                    .map(|i| &error.message[i + "Executable doesn't exist at ".len()..])
                {
                    let path_str = rest.split('\n').next().unwrap_or(rest).trim().to_string();
                    let _ = BROWSER_NOT_FOUND_PATH.set(path_str);
                }
            }
            maybe_print_browser_remedy();
            bail!("capture failed: [{}] {}", error.code, error.message)
        }
    }
}

/// Print the browser-not-found remedy block once per process.
fn maybe_print_browser_remedy() {
    if let Some(bad_path) = BROWSER_NOT_FOUND_PATH.get() {
        // Ensure we print this at most once
        if BROWSER_REMEDY_PRINTED.set(()).is_ok() {
            let repo_root = REPO_ROOT.get().map(|p| p.as_path());
            eprintln!("\n{}", format_browser_remedy(bad_path, repo_root));
        }
    }
}

// ---------------------------------------------------------------------------
// Bundle / config helpers (unchanged)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // Helper: create a temp dir structure mimicking
    //   <root>/target/release/matchy   (binary)
    //   <root>/packages/capture/dist/capture.cjs
    //
    // Returns (root, fake_exe_path, capture_path)
    fn make_repo_tree(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("matchy_orchestrate_test_{}", name));
        let _ = fs::remove_dir_all(&base); // clean prior run
        let exe_path = base.join("target/release/matchy");
        let capture_path = base.join("packages/capture/dist/capture.cjs");

        fs::create_dir_all(exe_path.parent().unwrap()).unwrap();
        fs::create_dir_all(capture_path.parent().unwrap()).unwrap();

        // Write placeholder files so `.exists()` returns true
        fs::write(&exe_path, b"fake-binary").unwrap();
        fs::write(&capture_path, b"// fake capture.cjs").unwrap();

        (base, exe_path, capture_path)
    }

    #[test]
    fn candidate_list_includes_ancestor_path() {
        let (root, exe_path, expected_capture) = make_repo_tree("ancestor_walk");

        let cwd = PathBuf::from("/tmp");
        let candidates = capture_script_candidates(Some(&exe_path), &cwd);

        // The ancestor walk should include <root>/packages/capture/dist/capture.cjs
        assert!(
            candidates.contains(&expected_capture),
            "expected {:?} in candidates:\n{:#?}",
            expected_capture,
            candidates
        );

        // Cleanup
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn candidate_list_order_sibling_before_ancestor() {
        let (root, exe_path, ancestor_capture) = make_repo_tree("order_check");
        let cwd = PathBuf::from("/tmp");
        let candidates = capture_script_candidates(Some(&exe_path), &cwd);

        // First candidate is the sibling
        let sibling = exe_path.parent().unwrap().join("capture.cjs");
        assert_eq!(candidates[0], sibling);

        // The ancestor-walk capture path comes after the sibling
        let sibling_idx = 0usize;
        let ancestor_idx = candidates
            .iter()
            .position(|p| p == &ancestor_capture)
            .expect("ancestor capture path not in list");
        assert!(
            ancestor_idx > sibling_idx,
            "ancestor should come after sibling: sibling_idx={}, ancestor_idx={}",
            sibling_idx,
            ancestor_idx
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn candidate_list_cwd_path_last() {
        let (root, exe_path, _) = make_repo_tree("cwd_last");
        let cwd = PathBuf::from("/some/other/place");
        let candidates = capture_script_candidates(Some(&exe_path), &cwd);

        let last = candidates.last().unwrap();
        assert_eq!(
            last,
            &cwd.join("packages/capture/dist/capture.cjs"),
            "last candidate should be CWD-relative"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolution_finds_ancestor_capture() {
        // Build a tree, then use capture_script_candidates directly to verify
        // that the real file is found.
        let (root, exe_path, expected_capture) = make_repo_tree("resolution");
        let cwd = PathBuf::from("/tmp");
        let candidates = capture_script_candidates(Some(&exe_path), &cwd);

        let found = candidates.into_iter().find(|p| p.exists());
        assert_eq!(
            found.as_ref(),
            Some(&expected_capture),
            "resolution should pick the ancestor capture.cjs"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn extract_chromium_rev_headless_shell() {
        let path =
            "/home/admin/.cache/ms-playwright/chromium_headless_shell-1223/chrome-linux/chrome";
        assert_eq!(extract_chromium_rev(path), Some("1223".to_string()));
    }

    #[test]
    fn extract_chromium_rev_chromium() {
        let path = "/home/admin/.cache/ms-playwright/chromium-1217/chrome-linux/chrome";
        assert_eq!(extract_chromium_rev(path), Some("1217".to_string()));
    }

    #[test]
    fn extract_chromium_rev_no_match() {
        let path = "/some/random/path/chrome";
        assert_eq!(extract_chromium_rev(path), None);
    }

    #[test]
    fn format_browser_remedy_with_repo_root() {
        let missing =
            "/home/admin/.cache/ms-playwright/chromium_headless_shell-1223/chrome-linux/chrome";
        let repo = PathBuf::from("/home/admin/MatchyMatchy");
        let msg = format_browser_remedy(missing, Some(&repo));

        assert!(msg.contains("Chromium build 1223 not found at:"));
        assert!(msg.contains(missing));
        assert!(msg.contains("cd /home/admin/MatchyMatchy/packages/capture"));
        assert!(msg.contains("PLAYWRIGHT_BROWSERS_PATH=/home/admin/MatchyMatchy/.pw-browsers"));
        assert!(
            msg.contains("export PLAYWRIGHT_BROWSERS_PATH=/home/admin/MatchyMatchy/.pw-browsers")
        );
    }

    #[test]
    fn format_browser_remedy_without_repo_root() {
        let missing = "/some/.cache/ms-playwright/chromium-1217/chrome-linux/chrome";
        let msg = format_browser_remedy(missing, None);

        assert!(msg.contains("Chromium build 1217 not found at:"));
        assert!(msg.contains("<repo-root>"));
    }
}
