//! matchy CLI entry point (M1.md §5.5).
//!
//! Commands:
//!   matchy run --old URL --new URL --out DIR [options]
//!   matchy analyze --old-bundle PATH --new-bundle PATH --out DIR
//!   matchy doctor

use std::path::{Path, PathBuf};
use std::process;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};

use matchy_analyze::contract::ViewportConfig;
use matchy_analyze::orchestrate::{
    build_capture_config, load_bundle, resolve_capture_script, run_capture,
};
use matchy_analyze::report::json::{
    assemble_diff_result, make_run_id, write_diff_result, ViewportAnalysis,
};
use matchy_analyze::scoring::ParityProfile;

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(name = "matchy", version, about = "Page pair visual diff tool")]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,

    /// Old (baseline) URL
    #[arg(long, global = false)]
    old: Option<String>,

    /// New (candidate) URL
    #[arg(long, global = false)]
    new: Option<String>,

    /// Output directory
    #[arg(long, short = 'o', global = false)]
    out: Option<String>,

    /// Viewport name=WxH (repeatable). Default: desktop=1440x1000 and mobile=390x844
    #[arg(long, global = false)]
    viewport: Vec<String>,

    /// Parity profile: content-structure (default) or strict-visual
    #[arg(long, default_value = "content-structure", global = false)]
    profile: String,

    /// CSS selectors to hide (visibility:hidden)
    #[arg(long, global = false)]
    hide: Vec<String>,

    /// CSS selectors to mask (neutral fill)
    #[arg(long, global = false)]
    mask: Vec<String>,

    /// CSS selectors to click before capture
    #[arg(long, global = false)]
    click: Vec<String>,

    /// Disable time freezing
    #[arg(long, global = false, default_value_t = false)]
    no_freeze_time: bool,

    /// Disable Math.random stubbing
    #[arg(long, global = false, default_value_t = false)]
    no_stub_random: bool,

    /// Fail on issues at or above this severity (info|warning|error|critical|never)
    #[arg(long, default_value = "error", global = false)]
    fail_on: String,

    /// Always write JSON (reserved/no-op in M1 — JSON is always written)
    #[arg(long, global = false, default_value_t = true)]
    json: bool,
}

#[derive(Subcommand, Debug)]
enum CliCommand {
    /// Verify runtime environment
    Doctor,
    /// Run analysis from existing bundles (for determinism verification)
    Analyze(AnalyzeArgs),
}

#[derive(Args, Debug)]
struct AnalyzeArgs {
    /// Path to old CaptureBundle JSON
    #[arg(long)]
    old_bundle: String,

    /// Path to new CaptureBundle JSON
    #[arg(long)]
    new_bundle: String,

    /// Output directory
    #[arg(long, short = 'o')]
    out: String,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let cli = Cli::parse();

    let exit_code = match &cli.command {
        Some(CliCommand::Doctor) => {
            let ok = matchy_analyze::doctor::run_doctor();
            if ok {
                0
            } else {
                1
            }
        }
        Some(CliCommand::Analyze(args)) => {
            match run_analyze(&args.old_bundle, &args.new_bundle, &args.out, &cli.profile) {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("error: {:#}", e);
                    2
                }
            }
        }
        None => {
            // Default: matchy run
            match (cli.old.as_deref(), cli.new.as_deref(), cli.out.as_deref()) {
                (Some(old_url), Some(new_url), Some(out_dir)) => {
                    match run_full(
                        old_url,
                        new_url,
                        out_dir,
                        &cli.viewport,
                        &cli.profile,
                        &cli.hide,
                        &cli.mask,
                        &cli.click,
                        !cli.no_freeze_time,
                        !cli.no_stub_random,
                        &cli.fail_on,
                    ) {
                        Ok(code) => code,
                        Err(e) => {
                            eprintln!("error: {:#}", e);
                            2
                        }
                    }
                }
                _ => {
                    eprintln!("Usage: matchy --old URL --new URL --out DIR");
                    eprintln!("       matchy doctor");
                    eprintln!(
                        "       matchy analyze --old-bundle PATH --new-bundle PATH --out DIR"
                    );
                    2
                }
            }
        }
    };

    process::exit(exit_code);
}

// ---------------------------------------------------------------------------
// Run from URLs (full capture + analyze)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn run_full(
    old_url: &str,
    new_url: &str,
    out_dir: &str,
    viewport_args: &[String],
    profile_str: &str,
    hide: &[String],
    mask: &[String],
    click: &[String],
    freeze_time: bool,
    stub_random: bool,
    fail_on: &str,
) -> anyhow::Result<i32> {
    let run_id = make_run_id();
    let out_path = PathBuf::from(out_dir);
    let capture_script =
        resolve_capture_script().context("capture.cjs not found — run `matchy doctor`")?;

    let profile = ParityProfile::parse(profile_str).unwrap_or(ParityProfile::ContentStructure);

    let viewports = parse_viewports(viewport_args);

    let mut viewport_analyses: Vec<ViewportAnalysis> = Vec::new();

    for vp in &viewports {
        let vp_dir = out_path.join(&vp.name);
        std::fs::create_dir_all(&vp_dir)?;

        // Capture old
        let old_config = build_capture_config(&matchy_analyze::orchestrate::CaptureConfigParams {
            url: old_url,
            prefix: "old",
            // capture.cjs appends <viewport.name>/ itself; pass the run root
            out_dir: &out_path,
            viewport: vp,
            freeze_time,
            stub_random,
            hide_selectors: hide,
            mask_selectors: mask,
            click_selectors: click,
        });
        let old_bundle_path_result = run_capture(&capture_script, &old_config);

        // Capture new
        let new_config = build_capture_config(&matchy_analyze::orchestrate::CaptureConfigParams {
            url: new_url,
            prefix: "new",
            out_dir: &out_path,
            viewport: vp,
            freeze_time,
            stub_random,
            hide_selectors: hide,
            mask_selectors: mask,
            click_selectors: click,
        });
        let new_bundle_path_result = run_capture(&capture_script, &new_config);

        match (old_bundle_path_result, new_bundle_path_result) {
            (Err(old_err), Err(new_err)) => {
                eprintln!(
                    "Both captures failed for viewport '{}':\n  old: {}\n  new: {}",
                    vp.name, old_err, new_err
                );
                return Ok(2);
            }
            (Err(e), Ok(_)) | (Ok(_), Err(e)) => {
                // One side failed: emit load_error
                let vp_analysis =
                    make_load_error_analysis(&vp.name, &e.to_string(), &vp_dir, &profile);
                viewport_analyses.push(vp_analysis);
            }
            (Ok(old_bundle_path), Ok(new_bundle_path)) => {
                let vp_analysis = analyze_bundle_pair(
                    &old_bundle_path,
                    &new_bundle_path,
                    &vp_dir,
                    &vp.name,
                    &profile,
                )?;
                viewport_analyses.push(vp_analysis);
            }
        }
    }

    let result = assemble_diff_result(&run_id, old_url, new_url, &profile, viewport_analyses);
    write_diff_result(&result, &out_path)?;

    let exit_code = compute_exit_code(&result.status, fail_on);
    Ok(exit_code)
}

// ---------------------------------------------------------------------------
// Run from existing bundles (matchy analyze subcommand)
// ---------------------------------------------------------------------------

fn run_analyze(
    old_bundle_arg: &str,
    new_bundle_arg: &str,
    out_dir: &str,
    profile_str: &str,
) -> anyhow::Result<i32> {
    let run_id = make_run_id();
    let out_path = PathBuf::from(out_dir);
    let old_bundle_path = PathBuf::from(old_bundle_arg);
    let new_bundle_path = PathBuf::from(new_bundle_arg);

    let profile = ParityProfile::parse(profile_str).unwrap_or(ParityProfile::ContentStructure);

    let old_bundle = load_bundle(&old_bundle_path)?;
    let new_bundle = load_bundle(&new_bundle_path)?;

    // Bundle's parent is <viewport>/, parent's parent is the outDir used during capture.
    // Screenshot paths in bundle are relative to the capture outDir.
    let old_out_dir = old_bundle_path
        .parent() // <viewport>/
        .and_then(|p| p.parent()) // outDir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let new_out_dir = new_bundle_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let old_img = old_out_dir.join(&old_bundle.screenshots.full_page);
    let new_img = new_out_dir.join(&new_bundle.screenshots.full_page);

    let viewport_name = old_bundle.viewport.name.clone();
    let vp_dir = out_path.join(&viewport_name);
    std::fs::create_dir_all(&vp_dir)?;

    let diff_img_path = vp_dir.join("diff.png");
    let issues_dir = vp_dir.join("issues");
    std::fs::create_dir_all(&issues_dir)?;

    let (issues, scores) =
        matchy_analyze::analyze_viewport(&matchy_analyze::ViewportAnalysisParams {
            old_bundle: &old_bundle,
            new_bundle: &new_bundle,
            old_img_path: &old_img,
            new_img_path: &new_img,
            diff_img_path: &diff_img_path,
            issues_dir: &issues_dir,
            viewport_name: &viewport_name,
            profile: &profile,
        })?;

    let artifacts = make_artifacts(&viewport_name, &old_bundle, &new_bundle);

    let old_url = old_bundle.page.url.clone();
    let new_url = new_bundle.page.url.clone();

    let vp_analysis = ViewportAnalysis {
        name: viewport_name,
        issues,
        scores,
        artifacts,
        old_det: old_bundle.determinism,
        new_det: new_bundle.determinism,
    };

    let result = assemble_diff_result(&run_id, &old_url, &new_url, &profile, vec![vp_analysis]);
    write_diff_result(&result, &out_path)?;

    Ok(compute_exit_code(&result.status, "error"))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn analyze_bundle_pair(
    old_bundle_path: &Path,
    new_bundle_path: &Path,
    vp_dir: &Path,
    viewport_name: &str,
    profile: &ParityProfile,
) -> anyhow::Result<ViewportAnalysis> {
    let old_bundle = load_bundle(old_bundle_path)?;
    let new_bundle = load_bundle(new_bundle_path)?;

    // Screenshot paths are relative to the bundle file's parent's parent (the capture outDir).
    let old_out_dir = old_bundle_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let new_out_dir = new_bundle_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let old_img = old_out_dir.join(&old_bundle.screenshots.full_page);
    let new_img = new_out_dir.join(&new_bundle.screenshots.full_page);

    let diff_img_path = vp_dir.join("diff.png");
    let issues_dir = vp_dir.join("issues");
    std::fs::create_dir_all(&issues_dir)?;

    let (issues, scores) =
        matchy_analyze::analyze_viewport(&matchy_analyze::ViewportAnalysisParams {
            old_bundle: &old_bundle,
            new_bundle: &new_bundle,
            old_img_path: &old_img,
            new_img_path: &new_img,
            diff_img_path: &diff_img_path,
            issues_dir: &issues_dir,
            viewport_name,
            profile,
        })?;

    let artifacts = make_artifacts(viewport_name, &old_bundle, &new_bundle);

    Ok(ViewportAnalysis {
        name: viewport_name.to_string(),
        issues,
        scores,
        artifacts,
        old_det: old_bundle.determinism,
        new_det: new_bundle.determinism,
    })
}

fn make_artifacts(
    viewport_name: &str,
    old_bundle: &matchy_analyze::contract::CaptureBundle,
    new_bundle: &matchy_analyze::contract::CaptureBundle,
) -> matchy_analyze::contract::Artifacts {
    matchy_analyze::contract::Artifacts {
        old: old_bundle.screenshots.full_page.clone(),
        new: new_bundle.screenshots.full_page.clone(),
        diff: format!("{}/diff.png", viewport_name),
    }
}

fn make_load_error_analysis(
    viewport_name: &str,
    error_message: &str,
    _vp_dir: &Path,
    profile: &ParityProfile,
) -> ViewportAnalysis {
    use matchy_analyze::contract::{Anchors, IssueCategory, IssueType, Locator};
    use matchy_analyze::issue::compute_issue_id;

    let null_anchors = Anchors::null();
    let id = compute_issue_id(&IssueType::LoadError, viewport_name, &null_anchors, None);
    let severity = profile.severity_for(&IssueType::LoadError, &IssueCategory::Technical);

    let issue = matchy_analyze::contract::Issue {
        id,
        issue_type: IssueType::LoadError,
        category: IssueCategory::Technical,
        severity,
        confidence: matchy_analyze::config::base_confidence::LOAD_ERROR,
        viewport: viewport_name.to_string(),
        locale: None,
        goal: None,
        message: format!("Page failed to load: {}", error_message),
        locator: Locator {
            anchors: null_anchors,
            css_selector_old: None,
            css_selector_new: None,
            bbox_old: None,
            bbox_new: None,
            seq_index_old: None,
            seq_index_new: None,
        },
        evidence: serde_json::json!({ "error": error_message }),
        remediation: None,
    };

    let placeholder = format!("{}/old.png", viewport_name);
    ViewportAnalysis {
        name: viewport_name.to_string(),
        issues: vec![issue],
        scores: matchy_analyze::contract::Scores::all_pass(),
        artifacts: matchy_analyze::contract::Artifacts {
            old: placeholder.clone(),
            new: placeholder.clone(),
            diff: format!("{}/diff.png", viewport_name),
        },
        old_det: make_default_determinism(),
        new_det: make_default_determinism(),
    }
}

fn make_default_determinism() -> matchy_analyze::contract::CaptureDeterminism {
    use matchy_analyze::contract::StepStatus;
    matchy_analyze::contract::CaptureDeterminism {
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

fn parse_viewports(args: &[String]) -> Vec<ViewportConfig> {
    if args.is_empty() {
        return vec![
            ViewportConfig {
                name: "desktop".to_string(),
                width: 1440,
                height: 1000,
                dsf: 1.0,
            },
            ViewportConfig {
                name: "mobile".to_string(),
                width: 390,
                height: 844,
                dsf: 1.0,
            },
        ];
    }
    args.iter().filter_map(|s| parse_viewport_arg(s)).collect()
}

fn parse_viewport_arg(s: &str) -> Option<ViewportConfig> {
    // Format: name=WxH
    let (name, dims) = s.split_once('=')?;
    let (w_str, h_str) = dims.split_once('x')?;
    let width: u32 = w_str.parse().ok()?;
    let height: u32 = h_str.parse().ok()?;
    Some(ViewportConfig {
        name: name.to_string(),
        width,
        height,
        dsf: 1.0,
    })
}

fn compute_exit_code(status: &matchy_analyze::contract::Status, fail_on: &str) -> i32 {
    use matchy_analyze::contract::Status;
    match fail_on {
        "never" => 0,
        "info" => match status {
            Status::Pass => 0,
            _ => 1,
        },
        "warning" => match status {
            Status::Pass | Status::Warn => {
                if matches!(status, Status::Warn) {
                    1
                } else {
                    0
                }
            }
            _ => 1,
        },
        "error" => match status {
            Status::Fail => 1,
            Status::Error => 1,
            _ => 0,
        },
        "critical" => {
            // Only fail on critical (we don't track critical vs error separately in Status)
            match status {
                Status::Fail | Status::Error => 1,
                _ => 0,
            }
        }
        _ => match status {
            Status::Fail | Status::Error => 1,
            _ => 0,
        },
    }
}
