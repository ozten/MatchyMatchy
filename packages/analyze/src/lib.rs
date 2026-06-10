//! matchy-analyze library crate.

pub mod config;
pub mod contract;
pub mod doctor;
pub mod egress;
pub mod hygiene;
pub mod issue;
pub mod locale;
pub mod locale_data;
pub mod orchestrate;
pub mod region_link;
pub mod report;
pub mod scoring;
pub mod visual_diff;

/// Parameters for a single-viewport analysis.
pub struct ViewportAnalysisParams<'a> {
    pub old_bundle: &'a contract::CaptureBundle,
    pub new_bundle: &'a contract::CaptureBundle,
    pub old_img_path: &'a std::path::Path,
    pub new_img_path: &'a std::path::Path,
    pub diff_img_path: &'a std::path::Path,
    pub issues_dir: &'a std::path::Path,
    pub viewport_name: &'a str,
    pub profile: &'a scoring::ParityProfile,
}

/// Core analysis: given old and new bundles + paths, produce per-viewport issues + scores.
///
/// Returns (issues, scores) for a single viewport.
pub fn analyze_viewport(
    params: &ViewportAnalysisParams<'_>,
) -> anyhow::Result<(Vec<contract::Issue>, contract::Scores)> {
    let ViewportAnalysisParams {
        old_bundle,
        new_bundle,
        old_img_path,
        new_img_path,
        diff_img_path,
        issues_dir,
        viewport_name,
        profile,
    } = params;
    use crate::config::{base_confidence, CROP_PAD, VISUAL_THRESHOLD};
    use crate::contract::{IssueCategory, IssueType, Locator};
    use crate::issue::{compute_issue_id, resolve_id_collisions};
    use crate::region_link::link_region;
    use crate::scoring::compute_confidence;
    use crate::visual_diff::{crop_diff_region, crop_region, diff_images, save_png};

    let env_mismatch = orchestrate::env_mismatch(old_bundle, new_bundle);

    // --- Run hygiene checks FIRST (M2.md §5.5) ---
    let hygiene_outcome = hygiene::hygiene_issues(old_bundle, new_bundle, viewport_name, profile);

    // --- Diff the images (always — artifacts stay valid even on short-circuit) ---
    let diff_out = diff_images(old_img_path, new_img_path)?;
    save_png(&diff_out.diff_image, diff_img_path)?;

    // If hygiene short-circuited: the viewport's issues are exactly the hygiene issues.
    // Visual score still reflects the diff (advisory); technical = 0.0.
    if hygiene_outcome.short_circuit {
        // The decisive issue is status_code_mismatch (category technical), so the
        // hygiene score counts hygiene-category issues only (M2.md §5.5).
        let hygiene_count = hygiene_outcome
            .issues
            .iter()
            .filter(|i| i.category == contract::IssueCategory::Hygiene)
            .count();
        let hygiene_score = 1.0 / (1.0 + hygiene_count as f64);
        let visual_score = (1.0 - diff_out.page_changed_ratio).clamp(0.0, 1.0);
        let mut issues = hygiene_outcome.issues;
        resolve_id_collisions(&mut issues);
        let scores = contract::Scores {
            visual: visual_score,
            content: 1.0,
            structure: 1.0,
            style: 1.0,
            accessibility: 1.0,
            technical: 0.0,
            hygiene: hygiene_score,
        };
        return Ok((issues, scores));
    }

    let mut issues: Vec<contract::Issue> = Vec::new();

    // Load original images for cropping
    let old_img = image::open(old_img_path)?;
    let new_img = image::open(new_img_path)?;

    // --- visual_region_changed issues ---
    if diff_out.page_changed_ratio >= VISUAL_THRESHOLD {
        for region in &diff_out.regions {
            // Link region to nodes
            let link = link_region(&region.bbox, &old_bundle.page.nodes, &new_bundle.page.nodes);

            let confidence = compute_confidence(
                base_confidence::VISUAL_REGION_CHANGED,
                env_mismatch,
                &old_bundle.determinism,
                &new_bundle.determinism,
            );

            let severity =
                profile.severity_for(&IssueType::VisualRegionChanged, &IssueCategory::Visual);

            let region_changed_ratio = if diff_out.common_height > 0 && diff_out.width > 0 {
                region.changed_pixels as f64
                    / (diff_out.width as f64 * diff_out.common_height as f64)
            } else {
                0.0
            };

            let id = compute_issue_id(
                &IssueType::VisualRegionChanged,
                viewport_name,
                &link.anchors,
                None,
            );

            // Save crop PNGs
            let old_crop_name = format!("{}_old.png", id);
            let new_crop_name = format!("{}_new.png", id);
            let diff_crop_name = format!("{}_diff.png", id);

            let old_crop_path = issues_dir.join(&old_crop_name);
            let new_crop_path = issues_dir.join(&new_crop_name);
            let diff_crop_path = issues_dir.join(&diff_crop_name);

            let old_crop = crop_region(&old_img, &region.bbox, CROP_PAD);
            let new_crop = crop_region(&new_img, &region.bbox, CROP_PAD);
            let diff_crop = crop_diff_region(&diff_out.diff_image, &region.bbox, CROP_PAD);

            save_png(&old_crop, &old_crop_path)?;
            save_png(&new_crop, &new_crop_path)?;
            save_png(&diff_crop, &diff_crop_path)?;

            // Build artifact paths relative to output dir
            // issues_dir is <viewport>/issues/, crop paths relative to out_dir parent
            let old_crop_rel = format!("{}/issues/{}", viewport_name, old_crop_name);
            let new_crop_rel = format!("{}/issues/{}", viewport_name, new_crop_name);
            let diff_crop_rel = format!("{}/issues/{}", viewport_name, diff_crop_name);

            let evidence = serde_json::json!({
                "visual": {
                    "regionBbox": [region.bbox.x, region.bbox.y, region.bbox.w, region.bbox.h],
                    "changedPixels": region.changed_pixels,
                    "regionChangedRatio": region_changed_ratio,
                    "pageChangedRatio": diff_out.page_changed_ratio
                },
                "artifacts": {
                    "oldCrop": old_crop_rel,
                    "newCrop": new_crop_rel,
                    "diffCrop": diff_crop_rel
                }
            });

            let bbox_arr = [
                region.bbox.x as i32,
                region.bbox.y as i32,
                region.bbox.w as i32,
                region.bbox.h as i32,
            ];

            issues.push(contract::Issue {
                id,
                issue_type: IssueType::VisualRegionChanged,
                category: IssueCategory::Visual,
                severity,
                confidence,
                viewport: viewport_name.to_string(),
                locale: old_bundle.page.lang.clone(),
                goal: None,
                message: format!(
                    "Visual region changed at ({}, {}) {}x{}",
                    region.bbox.x, region.bbox.y, region.bbox.w, region.bbox.h
                ),
                locator: Locator {
                    anchors: link.anchors,
                    css_selector_old: link.css_selector_old,
                    css_selector_new: link.css_selector_new,
                    bbox_old: Some(bbox_arr),
                    bbox_new: Some(bbox_arr),
                    seq_index_old: link.seq_index_old,
                    seq_index_new: link.seq_index_new,
                },
                evidence,
                remediation: None,
            });
        }
    }

    // --- page_height_changed issue ---
    if diff_out.old_height != diff_out.new_height {
        let confidence = compute_confidence(
            base_confidence::PAGE_HEIGHT_CHANGED,
            env_mismatch,
            &old_bundle.determinism,
            &new_bundle.determinism,
        );

        let severity = profile.severity_for(&IssueType::PageHeightChanged, &IssueCategory::Visual);

        let null_anchors = contract::Anchors::null();
        let id = compute_issue_id(
            &IssueType::PageHeightChanged,
            viewport_name,
            &null_anchors,
            None,
        );

        let evidence = serde_json::json!({
            "old": { "pageHeight": old_bundle.page.page_height },
            "new": { "pageHeight": new_bundle.page.page_height },
            "delta": new_bundle.page.page_height as i64 - old_bundle.page.page_height as i64
        });

        issues.push(contract::Issue {
            id,
            issue_type: IssueType::PageHeightChanged,
            category: IssueCategory::Visual,
            severity,
            confidence,
            viewport: viewport_name.to_string(),
            locale: old_bundle.page.lang.clone(),
            goal: None,
            message: format!(
                "Page height changed from {} to {} px",
                diff_out.old_height, diff_out.new_height
            ),
            locator: Locator {
                anchors: null_anchors,
                css_selector_old: None,
                css_selector_new: None,
                bbox_old: None,
                bbox_new: None,
                seq_index_old: None,
                seq_index_new: None,
            },
            evidence,
            remediation: None,
        });
    }

    // --- Append hygiene issues (non-short-circuit path) ---
    issues.extend(hygiene_outcome.issues.clone());

    // Resolve id collisions
    resolve_id_collisions(&mut issues);

    // Compute scores
    let hygiene_count = hygiene_outcome.issues.len();
    let hygiene_score = 1.0 / (1.0 + hygiene_count as f64);
    let visual_score = (1.0 - diff_out.page_changed_ratio).clamp(0.0, 1.0);
    let scores = contract::Scores {
        visual: visual_score,
        content: 1.0,
        structure: 1.0,
        style: 1.0,
        accessibility: 1.0,
        technical: 1.0,
        hygiene: hygiene_score,
    };

    Ok((issues, scores))
}
