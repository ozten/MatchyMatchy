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

    /// Emit the full legacy report dump instead of the default compact
    /// progressive-disclosure view (applies to --markdown and --html).
    #[arg(long, global = true, default_value_t = false)]
    full: bool,

    /// Path to baseline accept-list JSON (array of {"id": "..."}).
    #[arg(long, global = true)]
    baseline: Option<String>,

    /// Path to a severity-override JSON file:
    /// {"types": {"<issue type>": "info|warning|error|critical"},
    ///  "properties": {"<css property>": "info|warning|error|critical"}}.
    /// Overrides the built-in defaults and the profile's category mapping
    /// (property beats type). Cannot demote load_error, status_code_mismatch,
    /// or missing_form below critical — an attempted demotion is ignored and
    /// reported as a `severity_map_denied` warning. Unknown type/property
    /// keys or malformed JSON exit 2.
    #[arg(long, global = true)]
    severity_map: Option<String>,

    /// Restrict issues, scores and status to these landmark roles; out-of-scope issue ids are
    /// recorded in outOfScope. Page-level issues (no landmark) stay in scope.
    #[arg(long, global = true)]
    scope: Vec<String>,

    /// Capture the old page a second time and diff old-vs-old; writes self-check.json.
    /// Adds a volatile_capture warning if that probe finds drift, or self_check_failed
    /// if the probe itself fails; never affects the exit code. Run subcommand only.
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
    /// Hermetic computed-style / bbox triage probe over two frozen bundles.
    ///
    /// Locates a node by anchor string, node id, or CSS selector on each side
    /// independently and prints a per-side computed-style + bbox table highlighting
    /// differences.  No browser, network, or analysis engine involved — surfaces only
    /// data already in the bundles.
    ///
    /// Exit codes: 0 = node resolved on at least one side; 2 = node not found on
    /// either side, or bad locator syntax.
    Explain(ExplainArgs),
    /// Expand exactly one branch of an emitted diff-result.json to full detail.
    ///
    /// Hermetic and read-only: reads <out>/diff-result.json (no browser, network,
    /// capture bundles, or re-analysis). The branch handle is one of --region,
    /// --section (+ optional --heading), --cluster, or --issue. A --section without
    /// --heading expands the whole landmark (defined superset). Exit codes: 0 =
    /// branch resolved; 2 = handle unresolved, file missing/unreadable, or an
    /// unsupported (newer) schemaVersion.
    Show(ShowArgs),
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

#[derive(Args, Debug)]
#[group(required = true, multiple = false)]
struct ExplainLocator {
    /// Anchor locator: key=value where key ∈ {text, role, href, nearestHeading}.
    /// Substring match, case-sensitive.
    #[arg(long)]
    anchor: Option<String>,

    /// Node-id locator: exact match on SemanticNode.id (e.g. node_42).
    #[arg(long)]
    node: Option<String>,

    /// CSS-selector locator: exact match on SemanticNode.cssSelector, falling
    /// back to substring match when no exact match is found.
    #[arg(long)]
    selector: Option<String>,
}

#[derive(Args, Debug)]
struct ExplainArgs {
    /// Path to old CaptureBundle JSON
    #[arg(long)]
    old_bundle: String,

    /// Path to new CaptureBundle JSON
    #[arg(long)]
    new_bundle: String,

    #[command(flatten)]
    locator: ExplainLocator,

    /// Comma-separated list of CSS properties to show (e.g. color,font-family,gap).
    /// Default: show only properties that differ between the two sides (diff-only).
    #[arg(long)]
    props: Option<String>,
}

#[derive(Args, Debug)]
#[group(required = true, multiple = false)]
struct ShowHandle {
    /// Region landmark to expand (e.g. contentinfo).
    #[arg(long)]
    region: Option<String>,
    /// Section landmark to expand. Without --heading, expands the whole landmark (superset).
    #[arg(long)]
    section: Option<String>,
    /// Cluster id to expand (e.g. cluster_112233445566).
    #[arg(long)]
    cluster: Option<String>,
    /// Issue id to expand (e.g. issue_aabbccddeeff).
    #[arg(long)]
    issue: Option<String>,
}

#[derive(Args, Debug)]
struct ShowArgs {
    #[command(flatten)]
    handle: ShowHandle,
    /// Optional heading to scope a --section to one (landmark, heading) section.
    /// Pass shell-hazardous heading text (spaces, em-dashes) as a quoted value.
    #[arg(long)]
    heading: Option<String>,
    /// Directory containing diff-result.json (or a direct path to the JSON file).
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
            let mode = matchy_analyze::report::DisclosureMode::from_full_flag(cli.full);
            match run_analyze(
                &args.old_bundle,
                &args.new_bundle,
                &args.out,
                &cli.profile,
                cli.baseline.as_deref(),
                cli.severity_map.as_deref(),
                &cli.scope,
                cli.html,
                cli.markdown,
                image_dims_mode,
                &cli.fail_on,
                mode,
            ) {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("error: {:#}", e);
                    2
                }
            }
        }
        Some(CliCommand::Explain(args)) => match run_explain(args) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("error: {:#}", e);
                2
            }
        },
        Some(CliCommand::Show(args)) => match run_show(args) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("error: {:#}", e);
                2
            }
        },
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
                    let mode = matchy_analyze::report::DisclosureMode::from_full_flag(cli.full);
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
                        cli.severity_map.as_deref(),
                        &cli.scope,
                        cli.html,
                        cli.markdown,
                        image_dims_mode,
                        cli.self_check,
                        mode,
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
                    eprintln!("       matchy show --region LANDMARK --out DIR");
                    eprintln!(
                        "       matchy show --section LANDMARK [--heading HEADING] --out DIR"
                    );
                    eprintln!("       matchy show --cluster ID --out DIR");
                    eprintln!("       matchy show --issue ID --out DIR");
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
    severity_map_arg: Option<&str>,
    scope_args: &[String],
    html: bool,
    markdown: bool,
    image_dims_mode: ImageDimensionsMode,
    self_check: bool,
    mode: matchy_analyze::report::DisclosureMode,
) -> anyhow::Result<i32> {
    let run_id = make_run_id();
    let out_path = PathBuf::from(out_dir);
    let capture_script =
        resolve_capture_script().context("capture.cjs not found — run `matchy doctor`")?;

    let profile = ParityProfile::parse(profile_str).unwrap_or(ParityProfile::ContentStructure);
    let (severity_resolver, severity_map_echo, severity_map_warnings) =
        build_severity_resolver(profile.clone(), severity_map_arg)
            .context("failed to load --severity-map")?;

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

    // Viewports whose OLD-side capture
    // failed THIS run. A stale old.bundle.json from a previous run into the
    // same --out dir must never let the self-check probe diff against it as
    // if this run's old capture had succeeded — so we track it here and skip
    // the probe for these viewports (main already reports load_error for them).
    let mut old_capture_failed: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();

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
            (Err(old_err), Ok(_)) => {
                // OLD side failed, NEW side succeeded: emit load_error, and record
                // this viewport so run_self_check (which probes the OLD url) skips
                // it rather than diffing against a stale old.bundle.json.
                old_capture_failed.insert(vp.name.clone());
                let vp_analysis = make_load_error_analysis(
                    &vp.name,
                    &old_err.to_string(),
                    &vp_dir,
                    &severity_resolver,
                );
                viewport_analyses.push(vp_analysis);
            }
            (Ok(_), Err(new_err)) => {
                // NEW side failed only: the OLD capture this run is fine, so the
                // self-check probe (old-vs-old) is still valid for this viewport.
                let vp_analysis = make_load_error_analysis(
                    &vp.name,
                    &new_err.to_string(),
                    &vp_dir,
                    &severity_resolver,
                );
                viewport_analyses.push(vp_analysis);
            }
            (Ok(old_bundle_path), Ok(new_bundle_path)) => {
                let vp_analysis = analyze_bundle_pair(
                    &old_bundle_path,
                    &new_bundle_path,
                    &vp_dir,
                    &vp.name,
                    &severity_resolver,
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
            &severity_resolver,
            image_dims_mode,
            &run_id,
            &old_capture_failed,
        )?
    } else {
        vec![]
    };
    // severity_map_denied (if any) is CLI-input-driven and independent of
    // capture, so it goes first, ahead of self-check's capture-derived warnings.
    let mut extra_warnings = extra_warnings;
    let mut all_extra_warnings = severity_map_warnings;
    all_extra_warnings.append(&mut extra_warnings);
    let extra_warnings = all_extra_warnings;

    let result = assemble_diff_result(
        &run_id,
        old_url,
        new_url,
        &profile,
        viewport_analyses,
        &baseline,
        &scope_opts,
        extra_warnings,
        severity_map_echo,
    );
    write_diff_result(&result, &out_path)?;
    if html {
        matchy_analyze::report::html::write_html(&result, &out_path, mode)?;
    }
    if markdown {
        matchy_analyze::report::markdown::write_markdown(&result, &out_path, mode)?;
    }

    let exit_code = compute_exit_code(&result, fail_on);
    Ok(exit_code)
}

// ---------------------------------------------------------------------------
// --self-check implementation
// ---------------------------------------------------------------------------

/// Run the `--self-check` probe: a second capture of the OLD url, diffed
/// old-vs-old, surfaced only as `warnings[]` on the MAIN result (never the
/// exit code). Every fallible step is degraded to a per-viewport/whole-probe
/// failure recorded in `failed`/`write_failed` rather than propagated via `?`
/// — this function must always return `Ok(..)` (see the exit-code promise in
/// `run_full`'s docs). The `Result` signature is kept for call-site compatibility.
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
    severity_resolver: &matchy_analyze::scoring::SeverityResolver,
    image_dims_mode: ImageDimensionsMode,
    run_id: &str,
    old_capture_failed: &std::collections::BTreeSet<String>,
) -> anyhow::Result<Vec<matchy_analyze::contract::RunWarning>> {
    use matchy_analyze::contract::RunWarning;
    use std::collections::BTreeMap;

    // Best-effort cleanup of stale probe
    // state from a previous run into a REUSED --out dir. Without this, a stale
    // self-check.json (or stale <vp>/self-check/ artifacts) from a prior run
    // could survive a run where the probe now fails entirely, silently
    // contradicting this run's warnings[]. Best-effort: failure to remove is
    // not itself a probe failure.
    let _ = std::fs::remove_file(out_path.join("self-check.json"));

    // Per-viewport failures, keyed by viewport name, valued by a closed-vocabulary
    // stage: "capture" | "missing_old_bundle" | "analysis". Raw error text (anyhow
    // chains, paths, exit statuses) stays on stderr only — never folded into the
    // warning, to keep the warning byte-deterministic across machines/runs.
    let mut failed: BTreeMap<String, &'static str> = BTreeMap::new();

    let mut sc_viewport_analyses: Vec<ViewportAnalysis> = Vec::new();

    for vp in viewports {
        let vp_dir = out_path.join(&vp.name);

        // If THIS run's old-side capture already failed for this
        // viewport, old.bundle.json is stale-or-absent — main already emits
        // load_error for it. Diffing the probe against a stale old.bundle.json
        // would silently validate the wrong capture, so skip before even the
        // stale-state cleanup below.
        if old_capture_failed.contains(&vp.name) {
            eprintln!(
                "[self-check] skipping viewport '{}': this run's old-side capture failed",
                vp.name
            );
            failed.insert(vp.name.clone(), "missing_old_bundle");
            continue;
        }

        // Stale per-viewport probe artifacts (diff.png /
        // issues/) from a previous run into this same --out dir.
        let _ = std::fs::remove_dir_all(vp_dir.join("self-check"));

        // Degrade instead of `?` — a failure to create the viewport dir
        // must not escape run_self_check.
        if let Err(e) = std::fs::create_dir_all(&vp_dir) {
            eprintln!(
                "[self-check] failed to create viewport dir {} for '{}': {}",
                vp_dir.display(),
                vp.name,
                e
            );
            failed.insert(vp.name.clone(), "capture");
            continue;
        }

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
                failed.insert(vp.name.clone(), "capture");
                continue;
            }
        };

        // The original old bundle path.
        let old_bundle_path = vp_dir.join("old.bundle.json");
        if !old_bundle_path.exists() {
            // Backstop: old_capture_failed should already have caught
            // this, but keep the direct existence check too.
            eprintln!(
                "[self-check] old bundle not found at {}; skipping viewport '{}'",
                old_bundle_path.display(),
                vp.name
            );
            failed.insert(vp.name.clone(), "missing_old_bundle");
            continue;
        }

        // Probe-private artifact dir. analyze_bundle_pair writes
        // <dir>/diff.png and <dir>/issues/* unconditionally, and previously
        // received the SAME vp_dir the main run used — so a successful probe
        // silently overwrote the main run's visual artifacts. Isolating the
        // probe under <vp_dir>/self-check/ means the main run's diff.png /
        // issue crops (referenced by diff-result.json) are never touched.
        // self-check.json's embedded viewport-relative crop paths are
        // diagnostic only — its consumed surface is warnings[].
        let sc_artifact_dir = vp_dir.join("self-check");
        if let Err(e) = std::fs::create_dir_all(&sc_artifact_dir) {
            eprintln!(
                "[self-check] failed to create probe artifact dir {} for '{}': {}",
                sc_artifact_dir.display(),
                vp.name,
                e
            );
            failed.insert(vp.name.clone(), "analysis");
            continue;
        }

        match analyze_bundle_pair(
            &old_bundle_path,
            &sc_bundle_path,
            &sc_artifact_dir,
            &vp.name,
            severity_resolver,
            image_dims_mode,
        ) {
            Ok(vp_analysis) => sc_viewport_analyses.push(vp_analysis),
            Err(e) => {
                eprintln!(
                    "[self-check] analysis failed for viewport '{}': {}",
                    vp.name, e
                );
                failed.insert(vp.name.clone(), "analysis");
            }
        }
    }

    if sc_viewport_analyses.is_empty() {
        // Every viewport failed: no self-check.json can be assembled/written.
        // Report a warning instead of the previous silent Ok(vec![]).
        return Ok(
            build_self_check_failed_warning(&failed, false, viewports.len())
                .into_iter()
                .collect(),
        );
    }

    let sc_result = assemble_diff_result(
        run_id,
        old_url,
        old_url,
        severity_resolver.profile(),
        sc_viewport_analyses,
        &matchy_analyze::baseline::Baseline::default(),
        &ScopeOptions::default(),
        vec![],
        // self-check.json is an internal probe artifact, not the primary
        // contract deliverable — it never echoes the severity map.
        None,
    );

    // Degrade `to_json()?` — serialization failure must not escape.
    // volatile_capture is still built from sc_result.issues (available
    // in-memory regardless of serialization outcome); write_failed folds into
    // the self_check_failed warning.
    let sc_json = match sc_result.to_json() {
        Ok(j) => Some(j),
        Err(e) => {
            eprintln!("[self-check] failed to serialize self-check.json: {}", e);
            None
        }
    };

    let sc_path = out_path.join("self-check.json");
    let mut write_failed = false;
    match sc_json {
        Some(json) => {
            if let Err(e) = std::fs::write(&sc_path, json) {
                eprintln!("[self-check] failed to write self-check.json: {}", e);
                write_failed = true;
            }
        }
        None => write_failed = true,
    }

    let mut warnings: Vec<RunWarning> = Vec::new();

    if let Some(w) = build_volatile_capture_warning(&sc_result.issues) {
        warnings.push(w);
    }

    if let Some(w) = build_self_check_failed_warning(&failed, write_failed, viewports.len()) {
        warnings.push(w);
    }

    Ok(warnings)
}

/// Build the `volatile_capture` `RunWarning` (if any) from the self-check diff's
/// issues. Pure and deterministic: `byType` is a `BTreeMap` keyed by issue type, so
/// the same issue list always serializes identically regardless of iteration order.
///
/// Returns `None` when there is nothing to report (no issues found).
fn build_volatile_capture_warning(
    issues: &[matchy_analyze::contract::Issue],
) -> Option<matchy_analyze::contract::RunWarning> {
    use matchy_analyze::contract::RunWarning;
    use std::collections::BTreeMap;

    let issue_count = issues.len() as u32;
    if issue_count == 0 {
        return None;
    }

    // Build byType BTreeMap (deterministic).
    let mut by_type: BTreeMap<String, u32> = BTreeMap::new();
    for issue in issues {
        *by_type
            .entry(issue.issue_type.as_str().to_string())
            .or_insert(0) += 1;
    }

    Some(RunWarning {
        code: "volatile_capture".to_string(),
        message: format!(
            "self-check: {} issue(s) appeared when diffing two captures of the old page against each other; treat similar issues in the main result with suspicion (capture volatility, e.g. rotating content)",
            issue_count
        ),
        context: Some(serde_json::json!({
            "issueCount": issue_count,
            "byType": by_type,
        })),
    })
}

/// Build the `self_check_failed` `RunWarning` (if any) from the per-viewport failure
/// map and the self-check.json write outcome. Pure and deterministic: the message and
/// context are built ONLY from viewport names (already-owned `String`s) and the closed
/// failure-stage vocabulary — never from raw error text — so two calls with the same
/// inputs always serialize identically. `failed` iteration order is the `BTreeMap`'s
/// sorted order.
///
/// Returns `None` when there is nothing to report (no failed viewports and the write,
/// if attempted, succeeded).
fn build_self_check_failed_warning(
    failed: &std::collections::BTreeMap<String, &'static str>,
    write_failed: bool,
    total_viewports: usize,
) -> Option<matchy_analyze::contract::RunWarning> {
    use matchy_analyze::contract::RunWarning;

    if failed.is_empty() && !write_failed {
        return None;
    }

    let message = if failed.is_empty() {
        // All viewports succeeded; only the self-check.json write failed.
        "self-check probe ran but self-check.json could not be written".to_string()
    } else {
        let details: Vec<String> = failed
            .iter()
            .map(|(name, stage)| format!("{} ({})", name, stage))
            .collect();
        let mut msg = format!(
            "self-check probe failed for {} of {} viewport(s): {}",
            failed.len(),
            total_viewports,
            details.join(", ")
        );
        if write_failed {
            msg.push_str("; failed to write self-check.json");
        }
        msg
    };

    Some(RunWarning {
        code: "self_check_failed".to_string(),
        message,
        context: Some(serde_json::json!({
            "failedViewports": failed,
            "selfCheckJsonWriteFailed": write_failed,
        })),
    })
}

// ---------------------------------------------------------------------------
// matchy explain subcommand handler
// ---------------------------------------------------------------------------

fn run_explain(args: &ExplainArgs) -> anyhow::Result<i32> {
    use matchy_analyze::explain::{explain, format_report, Locator, ResolutionStatus};

    // Parse the locator from the exactly-one required flag group.
    let (locator, locator_str) = if let Some(anchor) = &args.locator.anchor {
        let loc = Locator::parse_anchor(anchor).map_err(|e| anyhow::anyhow!("{}", e))?;
        (loc, format!("--anchor \"{}\"", anchor))
    } else if let Some(node_id) = &args.locator.node {
        (
            Locator::NodeId(node_id.clone()),
            format!("--node {}", node_id),
        )
    } else if let Some(sel) = &args.locator.selector {
        (
            Locator::Selector(sel.clone()),
            format!("--selector \"{}\"", sel),
        )
    } else {
        // Clap enforces the required group, so this branch is unreachable.
        eprintln!("error: exactly one of --anchor, --node, or --selector is required");
        return Ok(2);
    };

    // Load bundles.
    let old_bundle_path = PathBuf::from(&args.old_bundle);
    let new_bundle_path = PathBuf::from(&args.new_bundle);
    let old_bundle = load_bundle(&old_bundle_path)
        .with_context(|| format!("failed to load old bundle: {}", args.old_bundle))?;
    let new_bundle = load_bundle(&new_bundle_path)
        .with_context(|| format!("failed to load new bundle: {}", args.new_bundle))?;

    // Parse --props if given.
    let props_vec: Option<Vec<String>> = args.props.as_ref().map(|p| {
        p.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });
    let props_slice = props_vec.as_deref();

    // Run the pure explain function.
    let report = explain(&old_bundle, &new_bundle, &locator, props_slice);

    // Check resolution status.
    let both_not_found = report.old.status == ResolutionStatus::NotFound
        && report.new.status == ResolutionStatus::NotFound;

    if both_not_found {
        eprintln!(
            "error: node not found for locator {} in either bundle",
            locator_str
        );
        return Ok(2);
    }

    // Print the formatted table.
    let output = format_report(&report, &locator_str);
    print!("{}", output);

    // Exit 0 on single-side match too (a legitimate triage finding).
    Ok(0)
}

// ---------------------------------------------------------------------------
// matchy show subcommand handler
// ---------------------------------------------------------------------------

fn run_show(args: &ShowArgs) -> anyhow::Result<i32> {
    use matchy_analyze::contract::DiffResult;
    use matchy_analyze::report::outline::{render_branch_detail, resolve_handle, BranchHandle};

    // 1. Build the handle from the exactly-one required flag.
    let (handle, handle_str) = if let Some(lm) = &args.handle.region {
        (
            BranchHandle::Region {
                landmark: lm.clone(),
            },
            format!("--region {}", lm),
        )
    } else if let Some(lm) = &args.handle.section {
        (
            BranchHandle::Section {
                landmark: lm.clone(),
                heading: args.heading.clone(),
            },
            match &args.heading {
                Some(h) => format!("--section {} --heading {}", lm, h),
                None => format!("--section {}", lm),
            },
        )
    } else if let Some(id) = &args.handle.cluster {
        (
            BranchHandle::Cluster { id: id.clone() },
            format!("--cluster {}", id),
        )
    } else if let Some(id) = &args.handle.issue {
        (
            BranchHandle::Issue { id: id.clone() },
            format!("--issue {}", id),
        )
    } else {
        eprintln!("error: exactly one of --region, --section, --cluster, --issue is required");
        return Ok(2);
    };

    // 2. Locate + read diff-result.json (dir or direct file path).
    let p = PathBuf::from(&args.out);
    let result_path = if p.is_file() {
        p
    } else {
        p.join("diff-result.json")
    };
    let raw = std::fs::read_to_string(&result_path)
        .with_context(|| format!("failed to read {}", result_path.display()))?;

    // 3. Parse.
    let result = DiffResult::from_json(&raw).with_context(|| {
        format!(
            "failed to parse {} (is it a diff-result.json?)",
            result_path.display()
        )
    })?;

    // 4. schemaVersion guard — refuse a newer major than this binary understands.
    const SUPPORTED_SCHEMA_MAJOR: u32 = 1;
    let major = result
        .schema_version
        .split('.')
        .next()
        .and_then(|s| s.parse::<u32>().ok());
    match major {
        Some(m) if m <= SUPPORTED_SCHEMA_MAJOR => { /* supported — fall through */ }
        Some(m) => {
            eprintln!(
                "error: {} has schemaVersion '{}' (major {}), newer than this matchy understands (supports {}.x) — upgrade matchy",
                result_path.display(), result.schema_version, m, SUPPORTED_SCHEMA_MAJOR
            );
            return Ok(2);
        }
        None => {
            eprintln!(
                "error: {} has an unrecognized schemaVersion '{}' (no parseable major version) — is it a valid diff-result.json?",
                result_path.display(), result.schema_version
            );
            return Ok(2);
        }
    }

    // 5. Resolve + print.
    let members = resolve_handle(&result, &handle);
    if members.is_empty() {
        eprintln!(
            "error: branch {} resolved to no issues in {}",
            handle_str,
            result_path.display()
        );
        return Ok(2);
    }
    print!("{}", render_branch_detail(&handle, &members));
    Ok(0)
}

// ---------------------------------------------------------------------------
// Run from existing bundles (matchy analyze subcommand)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn run_analyze(
    old_bundle_arg: &str,
    new_bundle_arg: &str,
    out_dir: &str,
    profile_str: &str,
    baseline_arg: Option<&str>,
    severity_map_arg: Option<&str>,
    scope_args: &[String],
    html: bool,
    markdown: bool,
    image_dims_mode: ImageDimensionsMode,
    fail_on: &str,
    mode: matchy_analyze::report::DisclosureMode,
) -> anyhow::Result<i32> {
    let run_id = make_run_id();
    let out_path = PathBuf::from(out_dir);
    let old_bundle_path = PathBuf::from(old_bundle_arg);
    let new_bundle_path = PathBuf::from(new_bundle_arg);

    let profile = ParityProfile::parse(profile_str).unwrap_or(ParityProfile::ContentStructure);
    let (severity_resolver, severity_map_echo, severity_map_warnings) =
        build_severity_resolver(profile.clone(), severity_map_arg)
            .context("failed to load --severity-map")?;

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

    let (issues, scores, old_landmark_node_counts) =
        matchy_analyze::analyze_viewport(&matchy_analyze::ViewportAnalysisParams {
            old_bundle: &old_bundle,
            new_bundle: &new_bundle,
            old_img_path: &old_img,
            new_img_path: &new_img,
            diff_img_path: &diff_img_path,
            issues_dir: &issues_dir,
            viewport_name: &viewport_name,
            profile: &severity_resolver,
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
        old_landmark_node_counts,
    };

    let result = assemble_diff_result(
        &run_id,
        &old_url,
        &new_url,
        &profile,
        vec![vp_analysis],
        &baseline,
        &scope_opts,
        severity_map_warnings,
        severity_map_echo,
    );
    write_diff_result(&result, &out_path)?;
    if html {
        matchy_analyze::report::html::write_html(&result, &out_path, mode)?;
    }
    if markdown {
        matchy_analyze::report::markdown::write_markdown(&result, &out_path, mode)?;
    }

    Ok(compute_exit_code(&result, fail_on))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build the severity resolver for a run from the parsed `--profile` plus an
/// optional `--severity-map PATH` (port-parity U3).
///
/// Returns `(resolver, severity_map_echo, extra_warnings)`:
///   - `resolver`: threaded to every differ in place of the bare `ParityProfile`.
///   - `severity_map_echo`: `Some(..)` iff `--severity-map` was supplied (even
///     if its overrides ended up empty or fully denied) — populates
///     `DiffResult.severity_map` so two runs with different maps are never
///     silently incomparable.
///   - `extra_warnings`: a single `severity_map_denied` warning when the map
///     attempted to demote a hard-Critical type below critical; empty
///     otherwise. Deterministic (BTreeMap-keyed context).
///
/// Malformed JSON / unknown type-or-property keys surface as an `Err` here —
/// the caller wraps it with `.context("failed to load --severity-map")?`,
/// which the top-level `Err(e) => { eprintln!(...); 2 }` arms turn into exit 2.
fn build_severity_resolver(
    profile: ParityProfile,
    severity_map_arg: Option<&str>,
) -> anyhow::Result<(
    matchy_analyze::scoring::SeverityResolver,
    Option<matchy_analyze::contract::SeverityMapEcho>,
    Vec<matchy_analyze::contract::RunWarning>,
)> {
    use matchy_analyze::contract::{RunWarning, SeverityMapEcho, SeverityOverrides};
    use matchy_analyze::scoring::SeverityResolver;

    let severity_map_path = match severity_map_arg {
        None => return Ok((SeverityResolver::from_profile(profile), None, vec![])),
        Some(p) => p,
    };

    let (user_types, user_properties) =
        matchy_analyze::scoring::load_user_severity_map(Path::new(severity_map_path))?;
    let (resolver, denied) = SeverityResolver::with_user_map(profile, user_types, user_properties);

    let mut warnings = Vec::new();
    if !denied.is_empty() {
        // BTreeMap<String, IssueSeverity> context, deterministic; wire severity
        // values as strings for a stable, human-readable warning payload.
        let denied_context: std::collections::BTreeMap<String, String> = denied
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().to_string()))
            .collect();
        warnings.push(RunWarning {
            code: "severity_map_denied".to_string(),
            message: format!(
                "--severity-map attempted to demote {} hard-Critical type(s) below critical; ignored (load_error, status_code_mismatch, missing_form can never be demoted below critical)",
                denied.len()
            ),
            context: Some(serde_json::json!({ "denied": denied_context })),
        });
    }

    let echo = Some(SeverityMapEcho {
        source: "file".to_string(),
        overrides: SeverityOverrides {
            types: resolver.accepted_types().clone(),
            properties: resolver.accepted_properties().clone(),
        },
    });

    Ok((resolver, echo, warnings))
}

fn analyze_bundle_pair(
    old_bundle_path: &Path,
    new_bundle_path: &Path,
    vp_dir: &Path,
    viewport_name: &str,
    profile: &matchy_analyze::scoring::SeverityResolver,
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

    let (issues, scores, old_landmark_node_counts) =
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
        old_landmark_node_counts,
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
    profile: &matchy_analyze::scoring::SeverityResolver,
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
        old_landmark_node_counts: std::collections::BTreeMap::new(),
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
        settle: None,
        hit_test_probe: None,
        quiescence: None,
        settle_scroll_ineffective: None,
        settle_growth_capped: None,
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::collections::BTreeMap;

    /// --full parses as a global flag on the top-level command (matchy run).
    #[test]
    fn test_full_flag_parses_global() {
        // Default: no --full → full == false.
        let cli = Cli::try_parse_from(["matchy", "--old", "u", "--new", "v", "--out", "o"])
            .expect("parse without --full");
        assert!(!cli.full, "--full must default to false");

        // With --full → full == true.
        let cli_full =
            Cli::try_parse_from(["matchy", "--old", "u", "--new", "v", "--out", "o", "--full"])
                .expect("parse with --full on run");
        assert!(cli_full.full, "--full must be true when passed on run");

        // --full on the analyze subcommand.
        let cli_analyze = Cli::try_parse_from([
            "matchy",
            "analyze",
            "--old-bundle",
            "a",
            "--new-bundle",
            "b",
            "--out",
            "o",
            "--full",
        ])
        .expect("parse with --full on analyze");
        assert!(
            cli_analyze.full,
            "--full must be true when passed on analyze"
        );
    }

    // -------------------------------------------------------------------
    // build_self_check_failed_warning (U2)
    // -------------------------------------------------------------------

    #[test]
    fn test_self_check_failed_warning_none_when_all_ok() {
        let failed = BTreeMap::new();
        let warning = build_self_check_failed_warning(&failed, false, 2);
        assert!(
            warning.is_none(),
            "no failed viewports and a successful write must produce no warning"
        );
    }

    #[test]
    fn test_self_check_failed_warning_two_failed_viewports() {
        let mut failed = BTreeMap::new();
        failed.insert("mobile".to_string(), "analysis");
        failed.insert("desktop".to_string(), "capture");

        let warning = build_self_check_failed_warning(&failed, false, 2)
            .expect("two failed viewports must produce a warning");

        assert_eq!(warning.code, "self_check_failed");
        assert_eq!(
            warning.message,
            "self-check probe failed for 2 of 2 viewport(s): desktop (capture), mobile (analysis)"
        );

        let context = warning.context.expect("context must be present");
        let failed_viewports = context
            .get("failedViewports")
            .expect("failedViewports key must be present")
            .as_object()
            .expect("failedViewports must be an object");
        assert_eq!(failed_viewports.len(), 2);
        assert_eq!(
            failed_viewports.get("desktop").and_then(|v| v.as_str()),
            Some("capture")
        );
        assert_eq!(
            failed_viewports.get("mobile").and_then(|v| v.as_str()),
            Some("analysis")
        );
        assert_eq!(
            context
                .get("selfCheckJsonWriteFailed")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn test_self_check_failed_warning_write_only_failure() {
        let failed = BTreeMap::new();
        let warning = build_self_check_failed_warning(&failed, true, 2)
            .expect("write failure alone must still produce a warning");

        assert_eq!(warning.code, "self_check_failed");
        assert_eq!(
            warning.message,
            "self-check probe ran but self-check.json could not be written"
        );

        let context = warning.context.expect("context must be present");
        // Stable key-set: failedViewports present (empty object) even though nothing failed.
        let failed_viewports = context
            .get("failedViewports")
            .expect("failedViewports key must be present")
            .as_object()
            .expect("failedViewports must be an object");
        assert!(failed_viewports.is_empty());
        assert_eq!(
            context
                .get("selfCheckJsonWriteFailed")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn test_self_check_failed_warning_deterministic() {
        let mut failed = BTreeMap::new();
        failed.insert("desktop".to_string(), "missing_old_bundle");

        let a = build_self_check_failed_warning(&failed, true, 1).unwrap();
        let b = build_self_check_failed_warning(&failed, true, 1).unwrap();

        let a_json = serde_json::to_string(&a).unwrap();
        let b_json = serde_json::to_string(&b).unwrap();
        assert_eq!(
            a_json, b_json,
            "identical inputs must serialize identically"
        );
    }

    #[test]
    fn test_self_check_failed_warning_code_is_exact() {
        let mut failed = BTreeMap::new();
        failed.insert("desktop".to_string(), "capture");
        let warning = build_self_check_failed_warning(&failed, false, 1).unwrap();
        assert_eq!(warning.code, "self_check_failed");
    }
}
