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

use matchy_analyze::config::ImageDimensionsMode;
use matchy_analyze::contract::ViewportConfig;
use matchy_analyze::orchestrate::{
    build_capture_config, load_bundle, resolve_capture_script, run_capture,
};
use matchy_analyze::report::json::{
    assemble_diff_result, make_run_id, write_diff_result, ScopeOptions, ViewportAnalysis,
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
    #[arg(long, default_value = "content-structure", global = true)]
    profile: String,

    /// Image-dimension comparison mode.
    /// strict: flag any naturalWidth/Height change (default).
    /// responsive: pass aspect-preserving downscales that still cover the rendered box;
    /// flag upscales, aspect changes, and undersized images.
    #[arg(long, default_value = "strict", global = true)]
    image_dims_mode: String,

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
    #[arg(long, default_value = "error", global = true)]
    fail_on: String,

    /// Always write JSON (reserved/no-op in M1 — JSON is always written)
    #[arg(long, global = false, default_value_t = true)]
    json: bool,

    /// Write a static HTML report (report.html) alongside the JSON output
    #[arg(long, global = true, default_value_t = false)]
    html: bool,

    /// Write a Markdown report (report.md) alongside the JSON output
    #[arg(long, global = true, default_value_t = false)]
    markdown: bool,

    /// Path to baseline accept-list JSON (array of {"id": "..."}).
    #[arg(long, global = true)]
    baseline: Option<String>,

    /// Restrict issues, scores and status to these landmark roles; out-of-scope issue ids are
    /// recorded in outOfScope. Page-level issues (no landmark) stay in scope.
    #[arg(long, global = true)]
    scope: Vec<String>,

    /// Capture the old page twice and diff the two captures against each other; any issues
    /// found are capture volatility, not real differences. Adds a volatile_capture warning
    /// and writes self-check.json. Run subcommand only; no-op on analyze subcommand.
    #[arg(long, global = false, default_value_t = false)]
    self_check: bool,
}

#[derive(Subcommand, Debug)]
enum CliCommand {
    /// Verify runtime environment
    Doctor,
    /// Replay two saved bundles offline and produce a DiffResult byte-deterministically.
    ///
    /// Supports the full global flag set: --profile, --baseline, --scope, --fail-on,
    /// --image-dims-mode, --html, --markdown. --viewport is irrelevant (the bundle
    /// carries its own viewport). Exit codes: 0 = clean/below threshold; 1 = issues at
    /// or above --fail-on severity; 2 = tool/IO/schema error.
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
            let image_dims_mode =
                ImageDimensionsMode::parse(&cli.image_dims_mode).unwrap_or_else(|| {
                    eprintln!(
                        "error: unknown --image-dims-mode '{}'; expected 'strict' or 'responsive'",
                        cli.image_dims_mode
                    );
                    std::process::exit(2);
                });
            match run_analyze(
                &args.old_bundle,
                &args.new_bundle,
                &args.out,
                &cli.profile,
                cli.baseline.as_deref(),
                &cli.scope,
                cli.html,
                cli.markdown,
                image_dims_mode,
                &cli.fail_on,
            ) {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("error: {:#}", e);
                    2
                }
            }
        }
        None => {
            // Default: matchy run
            let image_dims_mode =
                ImageDimensionsMode::parse(&cli.image_dims_mode).unwrap_or_else(|| {
                    eprintln!(
                        "error: unknown --image-dims-mode '{}'; expected 'strict' or 'responsive'",
                        cli.image_dims_mode
                    );
                    std::process::exit(2);
                });
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
                        cli.baseline.as_deref(),
                        &cli.scope,
                        cli.html,
                        cli.markdown,
                        image_dims_mode,
                        cli.self_check,
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
    baseline_arg: Option<&str>,
    scope_args: &[String],
    html: bool,
    markdown: bool,
    image_dims_mode: ImageDimensionsMode,
    self_check: bool,
) -> anyhow::Result<i32> {
    let run_id = make_run_id();
    let out_path = PathBuf::from(out_dir);
    let capture_script =
        resolve_capture_script().context("capture.cjs not found — run `matchy doctor`")?;

    let profile = ParityProfile::parse(profile_str).unwrap_or(ParityProfile::ContentStructure);

    let viewports = parse_viewports(viewport_args);

    let baseline = match baseline_arg {
        Some(p) => matchy_analyze::baseline::load(std::path::Path::new(p))
            .context("failed to load --baseline")?,
        None => matchy_analyze::baseline::Baseline::default(),
    };

    let scope_opts = ScopeOptions {
        scope: scope_args.to_vec(),
    };

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
                    image_dims_mode,
                )?;
                viewport_analyses.push(vp_analysis);
            }
        }
    }

    // ------------------------------------------------------------------
    // --self-check: capture old URL a second time and diff old vs old-selfcheck.
    // ------------------------------------------------------------------
    let extra_warnings: Vec<matchy_analyze::contract::RunWarning> = if self_check {
        run_self_check(
            old_url,
            &out_path,
            &viewports,
            &capture_script,
            freeze_time,
            stub_random,
            hide,
            mask,
            click,
            &profile,
            image_dims_mode,
            &run_id,
        )?
    } else {
        vec![]
    };

    let result = assemble_diff_result(
        &run_id,
        old_url,
        new_url,
        &profile,
        viewport_analyses,
        &baseline,
        &scope_opts,
        extra_warnings,
    );
    write_diff_result(&result, &out_path)?;
    if html {
        matchy_analyze::report::html::write_html(&result, &out_path)?;
    }
    if markdown {
        matchy_analyze::report::markdown::write_markdown(&result, &out_path)?;
    }

    let exit_code = compute_exit_code(&result, fail_on);
    Ok(exit_code)
}

// ---------------------------------------------------------------------------
// --self-check implementation
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn run_self_check(
    old_url: &str,
    out_path: &Path,
    viewports: &[ViewportConfig],
    capture_script: &Path,
    freeze_time: bool,
    stub_random: bool,
    hide: &[String],
    mask: &[String],
    click: &[String],
    profile: &ParityProfile,
    image_dims_mode: ImageDimensionsMode,
    run_id: &str,
) -> anyhow::Result<Vec<matchy_analyze::contract::RunWarning>> {
    use matchy_analyze::contract::RunWarning;

    let mut sc_viewport_analyses: Vec<ViewportAnalysis> = Vec::new();

    for vp in viewports {
        let vp_dir = out_path.join(&vp.name);
        std::fs::create_dir_all(&vp_dir)?;

        // The first old capture already exists with prefix "old".
        // Capture a second time with prefix "old-selfcheck".
        let sc_config = build_capture_config(&matchy_analyze::orchestrate::CaptureConfigParams {
            url: old_url,
            prefix: "old-selfcheck",
            out_dir: out_path,
            viewport: vp,
            freeze_time,
            stub_random,
            hide_selectors: hide,
            mask_selectors: mask,
            click_selectors: click,
        });
        let sc_bundle_path = match run_capture(capture_script, &sc_config) {
            Ok(p) => p,
            Err(e) => {
                eprintln!(
                    "[self-check] second capture of old URL failed for viewport '{}': {}",
                    vp.name, e
                );
                continue;
            }
        };

        // The original old bundle path.
        let old_bundle_path = vp_dir.join("old.bundle.json");
        if !old_bundle_path.exists() {
            eprintln!(
                "[self-check] old bundle not found at {}; skipping viewport '{}'",
                old_bundle_path.display(),
                vp.name
            );
            continue;
        }

        match analyze_bundle_pair(
            &old_bundle_path,
            &sc_bundle_path,
            &vp_dir,
            &vp.name,
            profile,
            image_dims_mode,
        ) {
            Ok(vp_analysis) => sc_viewport_analyses.push(vp_analysis),
            Err(e) => {
                eprintln!(
                    "[self-check] analysis failed for viewport '{}': {}",
                    vp.name, e
                );
            }
        }
    }

    if sc_viewport_analyses.is_empty() {
        return Ok(vec![]);
    }

    let sc_result = assemble_diff_result(
        run_id,
        old_url,
        old_url,
        profile,
        sc_viewport_analyses,
        &matchy_analyze::baseline::Baseline::default(),
        &ScopeOptions::default(),
        vec![],
    );

    // Write self-check.json.
    let sc_path = out_path.join("self-check.json");
    if let Err(e) = std::fs::write(&sc_path, sc_result.to_json()?) {
        eprintln!("[self-check] failed to write self-check.json: {}", e);
    }

    let issue_count = sc_result.issues.len() as u32;
    if issue_count == 0 {
        return Ok(vec![]);
    }

    // Build byType BTreeMap (deterministic).
    let mut by_type: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for issue in &sc_result.issues {
        *by_type
            .entry(issue.issue_type.as_str().to_string())
            .or_insert(0) += 1;
    }

    let warning = RunWarning {
        code: "volatile_capture".to_string(),
        message: format!(
            "self-check: {} issue(s) appeared when diffing two captures of the old page against each other; treat similar issues in the main result with suspicion (capture volatility, e.g. rotating content)",
            issue_count
        ),
        context: Some(serde_json::json!({
            "issueCount": issue_count,
            "byType": by_type,
        })),
    };

    Ok(vec![warning])
}

// ---------------------------------------------------------------------------
// Run from existing bundles (matchy analyze subcommand)
// ---------------------------------------------------------------------------

fn run_analyze(
    old_bundle_arg: &str,
    new_bundle_arg: &str,
    out_dir: &str,
    profile_str: &str,
    baseline_arg: Option<&str>,
    scope_args: &[String],
    html: bool,
    markdown: bool,
    image_dims_mode: ImageDimensionsMode,
    fail_on: &str,
) -> anyhow::Result<i32> {
    let run_id = make_run_id();
    let out_path = PathBuf::from(out_dir);
    let old_bundle_path = PathBuf::from(old_bundle_arg);
    let new_bundle_path = PathBuf::from(new_bundle_arg);

    let profile = ParityProfile::parse(profile_str).unwrap_or(ParityProfile::ContentStructure);

    let baseline = match baseline_arg {
        Some(p) => matchy_analyze::baseline::load(std::path::Path::new(p))
            .context("failed to load --baseline")?,
        None => matchy_analyze::baseline::Baseline::default(),
    };

    let scope_opts = ScopeOptions {
        scope: scope_args.to_vec(),
    };

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
            image_dims_mode,
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

    let result = assemble_diff_result(
        &run_id,
        &old_url,
        &new_url,
        &profile,
        vec![vp_analysis],
        &baseline,
        &scope_opts,
        vec![],
    );
    write_diff_result(&result, &out_path)?;
    if html {
        matchy_analyze::report::html::write_html(&result, &out_path)?;
    }
    if markdown {
        matchy_analyze::report::markdown::write_markdown(&result, &out_path)?;
    }

    Ok(compute_exit_code(&result, fail_on))
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
    image_dims_mode: ImageDimensionsMode,
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
            image_dims_mode,
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
        integrity: None,
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

/// Compute the exit code based on the worst severity in kept issues and the fail-on threshold.
///
/// fail_on values: "never" → 0; "info"|"warning"|"error"|"critical" → rank-based;
/// unknown → defaults to "error" behaviour.
/// Exit codes: 0 = threshold not met; 1 = threshold met; 2 = tool error (set by caller).
fn compute_exit_code(result: &matchy_analyze::contract::DiffResult, fail_on: &str) -> i32 {
    if fail_on == "never" {
        return 0;
    }

    // Map threshold string to severity rank.
    let threshold_rank: i64 = match fail_on {
        "info" => 0,
        "warning" => 1,
        "error" => 2,
        "critical" => 3,
        _ => 2, // default to "error"
    };

    // Compute max severity rank over kept issues; -1 if no issues.
    let max_rank: i64 = result
        .issues
        .iter()
        .map(|i| i.severity.rank() as i64)
        .max()
        .unwrap_or(-1);

    if max_rank >= threshold_rank {
        1
    } else {
        0
    }
}
