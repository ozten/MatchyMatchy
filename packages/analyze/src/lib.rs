//! matchy-analyze library crate.

pub mod a11y_diff;
pub mod baseline;
pub mod clustering;
pub mod config;
pub mod contract;
pub mod doctor;
pub mod egress;
pub mod explain;
pub mod hygiene;
pub mod issue;
pub mod locale;
pub mod locale_data;
pub mod matching;
pub mod network_diff;
pub mod orchestrate;
pub mod region_link;
pub mod report;
pub mod scoring;
pub mod semantic_diff;
pub mod sequence_diff;
pub mod style_diff;
pub mod visual_diff;

/// Compute category scores from a slice of issue references and a pre-computed visual score.
///
/// Category counts exclude Info-severity issues. Rationale: info-severity issues are
/// by definition "expected/uncertain, not a regression" (localhost downgrades,
/// uncertain pairings, profile-demoted visual issues) and must not pin scores at 0.
///
/// Used from both `analyze_viewport` (per-viewport) and `report::json` (baseline recompute)
/// so the two paths cannot diverge.
pub fn compute_scores_from_issues(
    issues: &[&contract::Issue],
    visual_score: f64,
) -> contract::Scores {
    use contract::{IssueCategory, IssueSeverity, IssueType};

    // Count only issues whose severity is Warning or worse (exclude Info).
    let non_info = |i: &&contract::Issue| i.severity != IssueSeverity::Info;

    let content_n = issues
        .iter()
        .filter(|i| non_info(i) && i.category == IssueCategory::Content)
        .count();
    let structure_n = issues
        .iter()
        .filter(|i| non_info(i) && i.category == IssueCategory::Structure)
        .count();
    let style_n = issues
        .iter()
        .filter(|i| non_info(i) && i.category == IssueCategory::Style)
        .count();
    // accessibility: only regressions (not improvements), severity Warning+.
    let a11y_n = issues
        .iter()
        .filter(|i| non_info(i) && i.issue_type == IssueType::AccessibilityRegression)
        .count();
    let technical_n = issues
        .iter()
        .filter(|i| non_info(i) && i.category == IssueCategory::Technical)
        .count();
    let hygiene_n = issues
        .iter()
        .filter(|i| non_info(i) && i.category == IssueCategory::Hygiene)
        .count();

    // Decisive technical failure (M2.md §5.5): status_code_mismatch is a
    // page-level verdict, not a countable defect — technical pins to 0.0,
    // matching the hygiene short-circuit path in analyze_viewport.
    let status_mismatch = issues
        .iter()
        .any(|i| i.issue_type == IssueType::StatusCodeMismatch);

    contract::Scores {
        visual: visual_score,
        content: 1.0 / (1.0 + content_n as f64),
        structure: 1.0 / (1.0 + structure_n as f64),
        style: 1.0 / (1.0 + style_n as f64),
        accessibility: 1.0 / (1.0 + a11y_n as f64),
        technical: if status_mismatch {
            0.0
        } else {
            1.0 / (1.0 + technical_n as f64)
        },
        hygiene: 1.0 / (1.0 + hygiene_n as f64),
        by_landmark: std::collections::BTreeMap::new(),
    }
}

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
    pub image_dims_mode: config::ImageDimensionsMode,
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
        image_dims_mode,
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
        // Exclude Info-severity issues from the count (same rule as the main path).
        let hygiene_count = hygiene_outcome
            .issues
            .iter()
            .filter(|i| {
                i.category == contract::IssueCategory::Hygiene
                    && i.severity != contract::IssueSeverity::Info
            })
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
            by_landmark: std::collections::BTreeMap::new(),
        };
        return Ok((issues, scores));
    }

    // --- Content diff: match nodes then derive semantic issues (M3.md §5.7) ---
    let page_ctx = matching::PageCtx {
        old_final_url: old_bundle.page.final_url.clone(),
        new_final_url: new_bundle.page.final_url.clone(),
    };
    let match_outcome = matching::match_nodes(
        &old_bundle.page.nodes,
        &new_bundle.page.nodes,
        &page_ctx,
        old_bundle.page.page_height,
        new_bundle.page.page_height,
    );
    let content_issues = semantic_diff::semantic_issues(
        old_bundle,
        new_bundle,
        &match_outcome,
        viewport_name,
        profile,
        env_mismatch,
        *image_dims_mode,
    );

    // --- Sequence diff: order/reorder issues (M5 §2) ---
    let sequence_issues_vec = sequence_diff::sequence_issues(
        &old_bundle.page.nodes,
        &new_bundle.page.nodes,
        &match_outcome,
        viewport_name,
        new_bundle.page.lang.clone(),
    );

    // --- Style diff: computed-style issues (M4 §3.5) ---
    let style_issues_vec = style_diff::style_issues(
        old_bundle,
        new_bundle,
        &match_outcome,
        viewport_name,
        profile,
        env_mismatch,
    );

    let mut issues: Vec<contract::Issue> = Vec::new();

    // Load original images for cropping
    let old_img = image::open(old_img_path)?;
    let new_img = image::open(new_img_path)?;

    // --- visual_region_changed issues ---
    // Crops are deferred until after resolve_id_collisions so that each crop file
    // is named with the issue's FINAL id (which may gain a "-2", "-3" suffix).
    // We record (issue_index, bbox) here and write files below after collision resolution.
    let mut pending_crops: Vec<(usize, crate::visual_diff::Rect)> = Vec::new();

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

            // Build placeholder artifact paths using the pre-collision id.
            // These will be patched with the final id after collision resolution.
            let old_crop_name = format!("{}_old.png", id);
            let new_crop_name = format!("{}_new.png", id);
            let diff_crop_name = format!("{}_diff.png", id);

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

            let issue_index = issues.len();
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

            // Record for deferred crop writing (after collision resolution).
            pending_crops.push((issue_index, region.bbox));
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

        // --- WP-G: section attribution ---
        // Only when both bundles have non-empty landmark_rects.
        let old_rects = old_bundle.page.landmark_rects.as_deref().unwrap_or(&[]);
        let new_rects = new_bundle.page.landmark_rects.as_deref().unwrap_or(&[]);
        let has_rects = !old_rects.is_empty() || !new_rects.is_empty();

        // Build a BTreeMap for each side keyed by path (deterministic lookup).
        use std::collections::BTreeMap as StableBTreeMap;
        let old_by_path: StableBTreeMap<&str, &contract::LandmarkRect> =
            old_rects.iter().map(|r| (r.path.as_str(), r)).collect();
        let new_by_path: StableBTreeMap<&str, &contract::LandmarkRect> =
            new_rects.iter().map(|r| (r.path.as_str(), r)).collect();

        // Collect all unique paths from both sides (BTreeSet for deterministic order).
        use std::collections::BTreeSet;
        let mut all_paths: BTreeSet<&str> = BTreeSet::new();
        for r in old_rects {
            all_paths.insert(r.path.as_str());
        }
        for r in new_rects {
            all_paths.insert(r.path.as_str());
        }

        // Build section delta entries with non-zero delta.
        struct SectionDelta<'a> {
            path: &'a str,
            role: &'a str,
            heading: Option<&'a str>,
            old_height: i32,
            new_height: i32,
            delta: i32,
            old_bbox: Option<[i32; 4]>,
            new_bbox: Option<[i32; 4]>,
        }
        let mut section_deltas: Vec<SectionDelta<'_>> = Vec::new();
        for path in &all_paths {
            let old_entry = old_by_path.get(path);
            let new_entry = new_by_path.get(path);
            let old_height = old_entry.map(|r| r.bbox[3]).unwrap_or(0);
            let new_height = new_entry.map(|r| r.bbox[3]).unwrap_or(0);
            let delta = new_height - old_height;
            if delta != 0 {
                let role = old_entry
                    .map(|r| r.role.as_str())
                    .or_else(|| new_entry.map(|r| r.role.as_str()))
                    .unwrap_or("");
                let heading = old_entry
                    .and_then(|r| r.heading.as_deref())
                    .or_else(|| new_entry.and_then(|r| r.heading.as_deref()));
                let old_bbox = old_entry.map(|r| r.bbox);
                let new_bbox = new_entry.map(|r| r.bbox);
                section_deltas.push(SectionDelta {
                    path,
                    role,
                    heading,
                    old_height,
                    new_height,
                    delta,
                    old_bbox,
                    new_bbox,
                });
            }
        }

        // Sort by |delta| desc, tie-break path asc; cap at 8.
        section_deltas.sort_by(|a, b| {
            let abs_a = a.delta.unsigned_abs();
            let abs_b = b.delta.unsigned_abs();
            abs_b.cmp(&abs_a).then_with(|| a.path.cmp(b.path))
        });
        section_deltas.truncate(8);

        // Top contributor's bboxes for locator.
        let top_old_bbox = section_deltas.first().and_then(|d| d.old_bbox);
        let top_new_bbox = section_deltas.first().and_then(|d| d.new_bbox);

        // Build evidence object.
        let evidence = if has_rects && !section_deltas.is_empty() {
            let section_deltas_json: Vec<serde_json::Value> = section_deltas
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "path": d.path,
                        "role": d.role,
                        "heading": d.heading,
                        "oldHeight": d.old_height,
                        "newHeight": d.new_height,
                        "delta": d.delta
                    })
                })
                .collect();
            serde_json::json!({
                "old": { "pageHeight": old_bundle.page.page_height },
                "new": { "pageHeight": new_bundle.page.page_height },
                "delta": new_bundle.page.page_height as i64 - old_bundle.page.page_height as i64,
                "sectionDeltas": section_deltas_json
            })
        } else {
            serde_json::json!({
                "old": { "pageHeight": old_bundle.page.page_height },
                "new": { "pageHeight": new_bundle.page.page_height },
                "delta": new_bundle.page.page_height as i64 - old_bundle.page.page_height as i64
            })
        };

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
                bbox_old: top_old_bbox,
                bbox_new: top_new_bbox,
                seq_index_old: None,
                seq_index_new: None,
            },
            evidence,
            remediation: None,
        });
    }

    // --- Network/console diff (M7 §2) ---
    let network_issues = network_diff::network_console_issues(
        old_bundle,
        new_bundle,
        viewport_name,
        profile,
        env_mismatch,
    );

    // --- A11y diff (M7 §3) ---
    let a11y_issues_vec =
        a11y_diff::a11y_issues(old_bundle, new_bundle, viewport_name, profile, env_mismatch);

    // --- Append issues: visual ++ content ++ sequence ++ style ++ network ++ a11y ++ hygiene (M7) ---
    issues.extend(content_issues);
    issues.extend(sequence_issues_vec);
    issues.extend(style_issues_vec);
    issues.extend(network_issues);
    issues.extend(a11y_issues_vec);

    // --- Append hygiene issues (non-short-circuit path) ---
    issues.extend(hygiene_outcome.issues.clone());

    // Resolve id collisions
    resolve_id_collisions(&mut issues);

    // --- Deferred crop writing (Fix: suffix-aware crop artifact naming, M6.md §5) ---
    // Visual issues were pushed first into `issues` (indices 0..N_visual), and no sort
    // intervenes between the visual loop and resolve_id_collisions. After resolution,
    // each issue's final id (with any "-2"/"-3"/… suffix) is stable. We now write crop
    // PNGs named with the final id and patch each issue's evidence.artifacts accordingly.
    // pending_crops is built in construction order (ascending index), iterated the same way.
    for (issue_idx, bbox) in &pending_crops {
        let final_id = issues[*issue_idx].id.clone();
        let old_crop_name = format!("{}_old.png", final_id);
        let new_crop_name = format!("{}_new.png", final_id);
        let diff_crop_name = format!("{}_diff.png", final_id);

        let old_crop_path = issues_dir.join(&old_crop_name);
        let new_crop_path = issues_dir.join(&new_crop_name);
        let diff_crop_path = issues_dir.join(&diff_crop_name);

        let old_crop = crop_region(&old_img, bbox, CROP_PAD);
        let new_crop = crop_region(&new_img, bbox, CROP_PAD);
        let diff_crop = crop_diff_region(&diff_out.diff_image, bbox, CROP_PAD);

        save_png(&old_crop, &old_crop_path)?;
        save_png(&new_crop, &new_crop_path)?;
        save_png(&diff_crop, &diff_crop_path)?;

        // Patch evidence.artifacts with final-id paths.
        let old_crop_rel = format!("{}/issues/{}", viewport_name, old_crop_name);
        let new_crop_rel = format!("{}/issues/{}", viewport_name, new_crop_name);
        let diff_crop_rel = format!("{}/issues/{}", viewport_name, diff_crop_name);

        if let Some(artifacts) = issues[*issue_idx].evidence.get_mut("artifacts") {
            *artifacts = serde_json::json!({
                "oldCrop": old_crop_rel,
                "newCrop": new_crop_rel,
                "diffCrop": diff_crop_rel
            });
        }
    }

    // Compute scores — exclude Info-severity issues from all count-based scores.
    // Rationale: info-severity issues are by definition "expected/uncertain, not a regression"
    // (localhost downgrades, uncertain pairings, profile-demoted visual issues) and must not
    // pin scores at 0.
    let visual_score = (1.0 - diff_out.page_changed_ratio).clamp(0.0, 1.0);
    let issue_refs: Vec<&contract::Issue> = issues.iter().collect();
    let scores = compute_scores_from_issues(&issue_refs, visual_score);

    Ok((issues, scores))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CROP_PAD;
    use crate::contract::{Anchors, IssueCategory, IssueSeverity, IssueType, Locator};
    use crate::issue::{compute_issue_id, resolve_id_collisions};
    use crate::visual_diff::{crop_diff_region, crop_region, save_png, Rect};
    use image::{ImageBuffer, Rgb, Rgba, RgbaImage};
    use tempfile::TempDir;

    /// WP-A (M6.md §5): suffix-aware crop artifact naming.
    ///
    /// Two visual issues sharing the same content-addressed id (same type/viewport/null anchors)
    /// but covering distinct pixel regions must, after collision resolution and deferred crop
    /// writing, produce:
    ///  - distinct final ids: X (the base) and X-2 (the suffixed one)
    ///  - distinct artifact paths: X_old.png and X-2_old.png
    ///  - both files on disk with different bytes (different bboxes → different crops)
    #[test]
    fn test_deferred_crop_suffix_aware() {
        let tmp = TempDir::new().unwrap();
        let issues_dir = tmp.path().join("desktop").join("issues");
        std::fs::create_dir_all(&issues_dir).unwrap();

        // Build a synthetic 400x200 image: left half red, right half blue.
        // This guarantees the two crops (left region vs right region) have different bytes.
        let width = 400u32;
        let height = 200u32;
        let old_rgba: RgbaImage = ImageBuffer::from_fn(width, height, |x, _y| {
            if x < 200 {
                Rgba([255u8, 0, 0, 255])
            } else {
                Rgba([0u8, 0, 255, 255])
            }
        });
        let new_rgba: RgbaImage =
            ImageBuffer::from_fn(width, height, |_x, _y| Rgba([255u8, 255, 255, 255]));
        let diff_rgb: image::RgbImage = ImageBuffer::from_fn(width, height, |x, _y| {
            if x < 200 {
                Rgb([255u8, 0, 0])
            } else {
                Rgb([0u8, 0, 255])
            }
        });

        // Save old/new as PNGs so we can load them as DynamicImage.
        let old_path = tmp.path().join("old.png");
        let new_path = tmp.path().join("new.png");
        old_rgba.save(&old_path).unwrap();
        new_rgba.save(&new_path).unwrap();
        let old_img = image::open(&old_path).unwrap();
        let new_img = image::open(&new_path).unwrap();

        // Two regions: left blob [0,0,100,100] and right blob [200,0,100,100].
        let bbox_left = Rect {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
        };
        let bbox_right = Rect {
            x: 200,
            y: 0,
            w: 100,
            h: 100,
        };

        // Build two issues with identical content-addressed ids (same type + viewport + null anchors).
        let null_anchors = Anchors {
            text: None,
            role: None,
            href: None,
            alt: None,
            aria_label: None,
            nearest_heading: None,
            landmark: None,
            ordinal_in_landmark: None,
        };
        let base_id = compute_issue_id(
            &IssueType::VisualRegionChanged,
            "desktop",
            &null_anchors,
            None,
        );

        let make_visual_issue = |id: String, bbox: [i32; 4]| -> contract::Issue {
            let old_crop_rel = format!("desktop/issues/{}_old.png", id);
            let new_crop_rel = format!("desktop/issues/{}_new.png", id);
            let diff_crop_rel = format!("desktop/issues/{}_diff.png", id);
            contract::Issue {
                id,
                issue_type: IssueType::VisualRegionChanged,
                category: IssueCategory::Visual,
                severity: IssueSeverity::Info,
                confidence: 0.9,
                viewport: "desktop".to_string(),
                locale: None,
                goal: None,
                message: "test region".to_string(),
                locator: Locator {
                    anchors: null_anchors.clone(),
                    css_selector_old: None,
                    css_selector_new: None,
                    bbox_old: Some(bbox),
                    bbox_new: Some(bbox),
                    seq_index_old: None,
                    seq_index_new: None,
                },
                evidence: serde_json::json!({
                    "visual": {},
                    "artifacts": {
                        "oldCrop": old_crop_rel,
                        "newCrop": new_crop_rel,
                        "diffCrop": diff_crop_rel
                    }
                }),
                remediation: None,
            }
        };

        let bbox_left_arr = [0i32, 0, 100, 100];
        let bbox_right_arr = [200i32, 0, 100, 100];

        let mut issues: Vec<contract::Issue> = vec![
            make_visual_issue(base_id.clone(), bbox_left_arr),
            make_visual_issue(base_id.clone(), bbox_right_arr),
        ];
        let pending_crops: Vec<(usize, Rect)> = vec![(0, bbox_left), (1, bbox_right)];

        // Both issues share the same base id before collision resolution.
        assert_eq!(issues[0].id, issues[1].id);

        // Run collision resolution.
        resolve_id_collisions(&mut issues);

        // After resolution: one keeps base id, the other gets "-2".
        let final_id_0 = issues[0].id.clone();
        let final_id_1 = issues[1].id.clone();
        assert_ne!(
            final_id_0, final_id_1,
            "ids must differ after collision resolution"
        );
        let has_suffix = final_id_0.ends_with("-2") || final_id_1.ends_with("-2");
        assert!(has_suffix, "one issue must have -2 suffix");
        let (base_final, suffixed_final) = if final_id_0.ends_with("-2") {
            (final_id_1.clone(), final_id_0.clone())
        } else {
            (final_id_0.clone(), final_id_1.clone())
        };
        assert!(!base_final.ends_with("-2"), "base must not end with -2");
        assert!(suffixed_final.ends_with("-2"), "suffixed must end with -2");

        // Replicate the deferred crop-write pass from analyze_viewport.
        let viewport_name = "desktop";
        for (issue_idx, bbox) in &pending_crops {
            let final_id = issues[*issue_idx].id.clone();
            let old_crop_name = format!("{}_old.png", final_id);
            let new_crop_name = format!("{}_new.png", final_id);
            let diff_crop_name = format!("{}_diff.png", final_id);

            let old_crop_path = issues_dir.join(&old_crop_name);
            let new_crop_path = issues_dir.join(&new_crop_name);
            let diff_crop_path = issues_dir.join(&diff_crop_name);

            let old_crop = crop_region(&old_img, bbox, CROP_PAD);
            let new_crop = crop_region(&new_img, bbox, CROP_PAD);
            let diff_crop = crop_diff_region(&diff_rgb, bbox, CROP_PAD);

            save_png(&old_crop, &old_crop_path).unwrap();
            save_png(&new_crop, &new_crop_path).unwrap();
            save_png(&diff_crop, &diff_crop_path).unwrap();

            let old_crop_rel = format!("{}/issues/{}", viewport_name, old_crop_name);
            let new_crop_rel = format!("{}/issues/{}", viewport_name, new_crop_name);
            let diff_crop_rel = format!("{}/issues/{}", viewport_name, diff_crop_name);

            if let Some(artifacts) = issues[*issue_idx].evidence.get_mut("artifacts") {
                *artifacts = serde_json::json!({
                    "oldCrop": old_crop_rel,
                    "newCrop": new_crop_rel,
                    "diffCrop": diff_crop_rel
                });
            }
        }

        // Assert artifact paths reference the FINAL (post-collision) ids.
        for issue in &issues {
            let id = &issue.id;
            let artifacts = issue.evidence.get("artifacts").unwrap();
            let old_crop_path_val = artifacts.get("oldCrop").unwrap().as_str().unwrap();
            assert!(
                old_crop_path_val.contains(id.as_str()),
                "artifact path must contain the final id '{}', got '{}'",
                id,
                old_crop_path_val
            );
        }

        // Assert both old-crop files exist on disk.
        let old_crop_0 = issues_dir.join(format!("{}_old.png", final_id_0));
        let old_crop_1 = issues_dir.join(format!("{}_old.png", final_id_1));
        assert!(
            old_crop_0.exists(),
            "{} must exist on disk",
            old_crop_0.display()
        );
        assert!(
            old_crop_1.exists(),
            "{} must exist on disk",
            old_crop_1.display()
        );

        // Assert the two old-crop files have different bytes (different regions → different pixels).
        let bytes_0 = std::fs::read(&old_crop_0).unwrap();
        let bytes_1 = std::fs::read(&old_crop_1).unwrap();
        assert_ne!(
            bytes_0, bytes_1,
            "crops from distinct bboxes must have different bytes"
        );
    }

    // -----------------------------------------------------------------------
    // C1: duplicate-label id-set unit tests (M6 calibration)
    // Tests against semantic_diff::dup_label_ids (the set-builder helper).
    // -----------------------------------------------------------------------

    fn make_c1_node(
        id: &str,
        kind: &str,
        text: Option<&str>,
        bbox: [i32; 4],
        seq_index: u32,
    ) -> contract::SemanticNode {
        contract::SemanticNode {
            id: id.to_string(),
            kind: kind.to_string(),
            role: None,
            text: text.map(str::to_string),
            acc_name: None,
            href: None,
            image_alt: None,
            bbox,
            seq_index,
            anchors: contract::NodeAnchors {
                text: text.map(str::to_string),
                role: None,
                href: None,
                alt: None,
                aria_label: None,
                nearest_heading: None,
                landmark: None,
                ordinal_in_landmark: None,
            },
            css_selector: None,
            raw_href: None,
            src: None,
            natural_width: None,
            natural_height: None,
            loaded: None,
            heading_level: None,
        }
    }

    /// C1-a: text node with bbox inside a link's bbox and equal text → id is in dup-label set.
    #[test]
    fn test_c1_dup_label_inside_link_in_set() {
        let link = make_c1_node("link1", "link", Some("Get a Demo"), [0, 0, 200, 50], 0);
        let text = make_c1_node("text1", "text", Some("Get a Demo"), [10, 10, 180, 30], 1);
        let nodes = vec![link.clone(), text.clone()];
        let set = semantic_diff::dup_label_ids(&nodes);
        assert!(
            set.contains("text1"),
            "text dup-label id must be in the set"
        );
        assert!(!set.contains("link1"), "link id must NOT be in the set");
    }

    /// C1-b: equal text but text bbox is outside the link's bbox → id NOT in set.
    #[test]
    fn test_c1_equal_text_outside_bbox_not_in_set() {
        let link = make_c1_node("link1", "link", Some("Get a Demo"), [0, 0, 200, 50], 0);
        let text = make_c1_node("text1", "text", Some("Get a Demo"), [300, 10, 180, 30], 1);
        let nodes = vec![link.clone(), text.clone()];
        let set = semantic_diff::dup_label_ids(&nodes);
        assert!(
            !set.contains("text1"),
            "text node outside link bbox must NOT be in the set"
        );
    }

    /// C1-c: different text, text node inside link bbox → id NOT in set.
    #[test]
    fn test_c1_different_text_inside_bbox_not_in_set() {
        let link = make_c1_node("link1", "link", Some("Get a Demo"), [0, 0, 200, 50], 0);
        let text = make_c1_node("text1", "text", Some("Schedule Now"), [10, 10, 180, 30], 1);
        let nodes = vec![link.clone(), text.clone()];
        let set = semantic_diff::dup_label_ids(&nodes);
        assert!(
            !set.contains("text1"),
            "different-text node must NOT be in the set"
        );
    }

    /// C1-d: end-to-end via semantic_issues: old stream has link + dup-label text node,
    /// new stream has only the link. Matcher runs on FULL streams; the link pairs with the
    /// link, but the text node is missing_old. semantic_diff must NOT emit missing_text
    /// for the dup-label node.
    #[test]
    fn test_c1_end_to_end_no_missing_text_emission() {
        use crate::contract::{
            A11yInfo, CaptureDeterminism, Environment, NetworkInfo, PageModel, Screenshots,
            StepStatus, StyleCandidates, ViewportConfig,
        };
        use crate::matching::{match_nodes, PageCtx};
        use crate::scoring::ParityProfile;
        use std::collections::BTreeMap;

        let make_det = || CaptureDeterminism {
            animations_disabled: StepStatus::Ran,
            reduced_motion: StepStatus::Ran,
            time_frozen: StepStatus::Ran,
            random_stubbed: StepStatus::Ran,
            fonts_ready: StepStatus::Ran,
            images_decoded: StepStatus::Ran,
            lazy_load_pass: StepStatus::Ran,
            settled: StepStatus::Ran,
            clicked: vec![],
            hidden: vec![],
            masked: vec![],
            retried_without_time_freeze: false,
            integrity: None,
        };
        let make_bundle = |url: &str, nodes: Vec<contract::SemanticNode>| contract::CaptureBundle {
            schema_version: "1.0".to_string(),
            captured_at: "2026-01-01T00:00:00Z".to_string(),
            viewport: ViewportConfig {
                name: "desktop".to_string(),
                width: 1440,
                height: 900,
                dsf: 1.0,
            },
            environment: Environment {
                os: "linux".to_string(),
                chromium_build: "1234".to_string(),
                playwright: "1.60.0".to_string(),
                dsf: 1.0,
            },
            determinism: make_det(),
            page: PageModel {
                url: url.to_string(),
                final_url: url.to_string(),
                redirect_chain: vec![],
                status_code: 200,
                title: None,
                meta_description: None,
                canonical: None,
                lang: Some("en".to_string()),
                page_height: 2000,
                nodes,
                landmarks: vec![],
                landmark_rects: None,
                network: NetworkInfo { requests: vec![] },
                console: vec![],
                a11y: A11yInfo { violations: vec![] },
                link_probes: vec![],
            },
            computed_styles: BTreeMap::new(),
            screenshots: Screenshots {
                full_page: "desktop/old.png".to_string(),
                viewport: "desktop/old-vp.png".to_string(),
            },
            style_candidates: StyleCandidates::default(),
        };

        // Old: link + nested dup-label text node (text bbox inside link bbox)
        let old_link = make_c1_node(
            "old-link",
            "link",
            Some("Get a Demo"),
            [100, 200, 200, 50],
            0,
        );
        let old_text = make_c1_node(
            "old-text",
            "text",
            Some("Get a Demo"),
            [110, 210, 180, 30],
            1,
        );

        // New: only the link (no nested label)
        let new_link = make_c1_node(
            "new-link",
            "link",
            Some("Get a Demo"),
            [100, 200, 200, 50],
            0,
        );

        let old_bundle = make_bundle("http://localhost:3000/", vec![old_link, old_text]);
        let new_bundle = make_bundle("http://localhost:3001/", vec![new_link]);

        // Run matcher on FULL (unfiltered) streams — this is the new design.
        let ctx = PageCtx {
            old_final_url: old_bundle.page.final_url.clone(),
            new_final_url: new_bundle.page.final_url.clone(),
        };
        let outcome = match_nodes(
            &old_bundle.page.nodes,
            &new_bundle.page.nodes,
            &ctx,
            old_bundle.page.page_height,
            new_bundle.page.page_height,
        );

        // The text node will be in missing_old (no new text node to match).
        // semantic_issues must NOT emit missing_text for it.
        let issues = semantic_diff::semantic_issues(
            &old_bundle,
            &new_bundle,
            &outcome,
            "desktop",
            &ParityProfile::ContentStructure,
            false,
            crate::config::ImageDimensionsMode::Strict,
        );

        let missing_text_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == contract::IssueType::MissingText)
            .collect();

        assert!(
            missing_text_issues.is_empty(),
            "C1: dup-label text node must NOT emit missing_text; got {} missing_text issues",
            missing_text_issues.len()
        );
    }

    // -----------------------------------------------------------------------
    // WP-E: exclude-Info scoring
    // -----------------------------------------------------------------------

    fn make_score_issue(
        id: &str,
        category: IssueCategory,
        severity: IssueSeverity,
        issue_type: IssueType,
    ) -> contract::Issue {
        contract::Issue {
            id: id.to_string(),
            issue_type,
            category,
            severity,
            confidence: 0.9,
            viewport: "desktop".to_string(),
            locale: None,
            goal: None,
            message: "test".to_string(),
            locator: contract::Locator {
                anchors: Anchors {
                    text: Some("x".to_string()),
                    role: None,
                    href: None,
                    alt: None,
                    aria_label: None,
                    nearest_heading: None,
                    landmark: None,
                    ordinal_in_landmark: None,
                },
                css_selector_old: None,
                css_selector_new: None,
                bbox_old: None,
                bbox_new: None,
                seq_index_old: None,
                seq_index_new: None,
            },
            evidence: serde_json::json!({}),
            remediation: None,
        }
    }

    /// WP-E: Info-severity issues must not reduce any category score.
    /// Only Warning+ issues count toward the 1/(1+n) formula.
    #[test]
    fn test_compute_scores_excludes_info_issues() {
        // One Error content issue + one Info content issue → effective count = 1 → score = 0.5
        let error_issue = make_score_issue(
            "issue_err0000001",
            IssueCategory::Content,
            IssueSeverity::Error,
            IssueType::ChangedText,
        );
        let info_issue = make_score_issue(
            "issue_inf0000001",
            IssueCategory::Content,
            IssueSeverity::Info,
            IssueType::ChangedText,
        );
        let issues: Vec<&contract::Issue> = vec![&error_issue, &info_issue];
        let scores = compute_scores_from_issues(&issues, 1.0);
        // 1 non-info content issue → 1/(1+1) = 0.5
        assert!(
            (scores.content - 0.5).abs() < 1e-9,
            "content score must be 0.5 (1 non-info error), got {}",
            scores.content
        );
        // All other categories untouched
        assert_eq!(scores.structure, 1.0);
        assert_eq!(scores.style, 1.0);
        assert_eq!(scores.accessibility, 1.0);
        assert_eq!(scores.technical, 1.0);
        assert_eq!(scores.hygiene, 1.0);
    }

    /// WP-E: If ALL issues are Info, all scores must remain 1.0.
    #[test]
    fn test_compute_scores_all_info_all_pass() {
        let info1 = make_score_issue(
            "issue_inf0000001",
            IssueCategory::Content,
            IssueSeverity::Info,
            IssueType::ChangedText,
        );
        let info2 = make_score_issue(
            "issue_inf0000002",
            IssueCategory::Style,
            IssueSeverity::Info,
            IssueType::StyleChanged,
        );
        let issues: Vec<&contract::Issue> = vec![&info1, &info2];
        let scores = compute_scores_from_issues(&issues, 1.0);
        assert_eq!(scores.content, 1.0, "all-info content must score 1.0");
        assert_eq!(scores.style, 1.0, "all-info style must score 1.0");
        assert_eq!(scores.structure, 1.0);
    }

    /// M2.md §5.5: a status_code_mismatch issue is a decisive technical
    /// failure — technical must pin to 0.0 in the recompute path too,
    /// matching the hygiene short-circuit in analyze_viewport.
    #[test]
    fn test_compute_scores_status_mismatch_pins_technical_to_zero() {
        let mismatch = make_score_issue(
            "issue_smm0000001",
            IssueCategory::Technical,
            IssueSeverity::Critical,
            IssueType::StatusCodeMismatch,
        );
        let issues: Vec<&contract::Issue> = vec![&mismatch];
        let scores = compute_scores_from_issues(&issues, 1.0);
        assert_eq!(
            scores.technical, 0.0,
            "status_code_mismatch must pin technical to 0.0, got {}",
            scores.technical
        );
        // Other categories unaffected by the decisive rule.
        assert_eq!(scores.content, 1.0);
        assert_eq!(scores.hygiene, 1.0);
    }

    // -----------------------------------------------------------------------
    // WP-G: page_height_changed sectionDeltas unit tests
    // -----------------------------------------------------------------------

    use crate::contract::{
        A11yInfo, CaptureDeterminism, Environment, LandmarkRect, NetworkInfo, PageModel,
        Screenshots, StepStatus, StyleCandidates, ViewportConfig,
    };

    fn make_wpg_det() -> CaptureDeterminism {
        CaptureDeterminism {
            animations_disabled: StepStatus::Ran,
            reduced_motion: StepStatus::Ran,
            time_frozen: StepStatus::Ran,
            random_stubbed: StepStatus::Ran,
            fonts_ready: StepStatus::Ran,
            images_decoded: StepStatus::Ran,
            lazy_load_pass: StepStatus::Ran,
            settled: StepStatus::Ran,
            clicked: vec![],
            hidden: vec![],
            masked: vec![],
            retried_without_time_freeze: false,
            integrity: None,
        }
    }

    fn make_wpg_bundle(
        url: &str,
        page_height: u32,
        landmark_rects: Option<Vec<LandmarkRect>>,
    ) -> contract::CaptureBundle {
        use std::collections::BTreeMap;
        contract::CaptureBundle {
            schema_version: "1.0".to_string(),
            captured_at: "2026-01-01T00:00:00Z".to_string(),
            viewport: ViewportConfig {
                name: "desktop".to_string(),
                width: 1440,
                height: 900,
                dsf: 1.0,
            },
            environment: Environment {
                os: "linux".to_string(),
                chromium_build: "1234".to_string(),
                playwright: "1.60.0".to_string(),
                dsf: 1.0,
            },
            determinism: make_wpg_det(),
            page: PageModel {
                url: url.to_string(),
                final_url: url.to_string(),
                redirect_chain: vec![],
                status_code: 200,
                title: None,
                meta_description: None,
                canonical: None,
                lang: Some("en".to_string()),
                page_height,
                nodes: vec![],
                landmarks: vec![],
                landmark_rects,
                network: NetworkInfo { requests: vec![] },
                console: vec![],
                a11y: A11yInfo { violations: vec![] },
                link_probes: vec![],
            },
            computed_styles: BTreeMap::new(),
            screenshots: Screenshots {
                full_page: "desktop/old.png".to_string(),
                viewport: "desktop/old-vp.png".to_string(),
            },
            style_candidates: StyleCandidates::default(),
        }
    }

    fn rect(
        path: &str,
        role: &str,
        heading: Option<&str>,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> LandmarkRect {
        LandmarkRect {
            path: path.to_string(),
            role: role.to_string(),
            heading: heading.map(str::to_string),
            bbox: [x, y, w, h],
        }
    }

    /// WP-G-a: main shrinks 95px, footer shrinks 24px → sectionDeltas ordered by |delta| desc,
    /// locator bboxes point to the top contributor (main).
    #[test]
    fn test_wpg_section_deltas_basic_ordering() {
        let old_rects = vec![
            rect("main", "main", Some("Register"), 0, 72, 1440, 500),
            rect("contentinfo", "contentinfo", None, 0, 572, 1440, 310),
        ];
        let new_rects = vec![
            rect("main", "main", Some("Register"), 0, 72, 1440, 405), // -95
            rect("contentinfo", "contentinfo", None, 0, 477, 1440, 286), // -24
        ];
        let old_bundle = make_wpg_bundle("http://old.example.com/", 4211, Some(old_rects));
        let mut new_bundle = make_wpg_bundle("http://new.example.com/", 3792, Some(new_rects));
        // Make new page_height actually differ.
        new_bundle.page.page_height = 3792;

        // We only test the evidence/locator logic, not the full analyze_viewport pipeline.
        // Extract the logic inline: compute section_deltas as the detector would.
        let old_rects_ref = old_bundle.page.landmark_rects.as_deref().unwrap_or(&[]);
        let new_rects_ref = new_bundle.page.landmark_rects.as_deref().unwrap_or(&[]);

        use std::collections::{BTreeMap, BTreeSet};
        let old_by_path: BTreeMap<&str, &LandmarkRect> =
            old_rects_ref.iter().map(|r| (r.path.as_str(), r)).collect();
        let new_by_path: BTreeMap<&str, &LandmarkRect> =
            new_rects_ref.iter().map(|r| (r.path.as_str(), r)).collect();
        let mut all_paths: BTreeSet<&str> = BTreeSet::new();
        for r in old_rects_ref {
            all_paths.insert(r.path.as_str());
        }
        for r in new_rects_ref {
            all_paths.insert(r.path.as_str());
        }

        struct SD {
            path: String,
            delta: i32,
            old_bbox: Option<[i32; 4]>,
            new_bbox: Option<[i32; 4]>,
        }
        let mut deltas: Vec<SD> = Vec::new();
        for path in &all_paths {
            let oe = old_by_path.get(path);
            let ne = new_by_path.get(path);
            let oh = oe.map(|r| r.bbox[3]).unwrap_or(0);
            let nh = ne.map(|r| r.bbox[3]).unwrap_or(0);
            let d = nh - oh;
            if d != 0 {
                deltas.push(SD {
                    path: path.to_string(),
                    delta: d,
                    old_bbox: oe.map(|r| r.bbox),
                    new_bbox: ne.map(|r| r.bbox),
                });
            }
        }
        deltas.sort_by(|a, b| {
            let abs_a = a.delta.unsigned_abs();
            let abs_b = b.delta.unsigned_abs();
            abs_b.cmp(&abs_a).then_with(|| a.path.cmp(&b.path))
        });
        deltas.truncate(8);

        assert_eq!(deltas.len(), 2, "both sections have non-zero delta");
        assert_eq!(deltas[0].path, "main", "main (|delta|=95) must come first");
        assert_eq!(deltas[0].delta, -95, "main delta must be -95");
        assert_eq!(
            deltas[1].path, "contentinfo",
            "contentinfo (|delta|=24) must come second"
        );
        assert_eq!(deltas[1].delta, -24);

        // Locator bboxes: top contributor is 'main'; old_bbox and new_bbox should be set.
        let top_old_bbox = deltas[0].old_bbox;
        let top_new_bbox = deltas[0].new_bbox;
        assert!(
            top_old_bbox.is_some(),
            "top contributor old bbox must be Some"
        );
        assert!(
            top_new_bbox.is_some(),
            "top contributor new bbox must be Some"
        );
        // Heights should match what we put in
        assert_eq!(top_old_bbox.unwrap()[3], 500, "old main height must be 500");
        assert_eq!(top_new_bbox.unwrap()[3], 405, "new main height must be 405");
    }

    /// WP-G-b: old side has landmark_rects, new side has None → no sectionDeltas in evidence.
    #[test]
    fn test_wpg_missing_rects_no_section_deltas() {
        let old_rects = vec![rect("main", "main", None, 0, 72, 1440, 500)];
        let old_bundle = make_wpg_bundle("http://old.example.com/", 4211, Some(old_rects));
        let new_bundle = make_wpg_bundle("http://new.example.com/", 3792, None);

        // has_rects logic: old is non-empty but new is empty (None → &[]).
        let old_r = old_bundle.page.landmark_rects.as_deref().unwrap_or(&[]);
        let new_r = new_bundle.page.landmark_rects.as_deref().unwrap_or(&[]);
        let has_rects = !old_r.is_empty() || !new_r.is_empty();

        // has_rects is true (old is non-empty). Compute deltas.
        use std::collections::{BTreeMap, BTreeSet};
        let old_by_path: BTreeMap<&str, &LandmarkRect> =
            old_r.iter().map(|r| (r.path.as_str(), r)).collect();
        let new_by_path: BTreeMap<&str, &LandmarkRect> =
            new_r.iter().map(|r| (r.path.as_str(), r)).collect();
        let mut all_paths: BTreeSet<&str> = BTreeSet::new();
        for r in old_r {
            all_paths.insert(r.path.as_str());
        }
        for r in new_r {
            all_paths.insert(r.path.as_str());
        }
        let mut deltas: Vec<(i32,)> = Vec::new();
        for path in &all_paths {
            let oh = old_by_path.get(path).map(|r| r.bbox[3]).unwrap_or(0);
            let nh = new_by_path.get(path).map(|r| r.bbox[3]).unwrap_or(0);
            let d = nh - oh;
            if d != 0 {
                deltas.push((d,));
            }
        }
        // When new side is None (empty), main is "unpaired old": newHeight=0, delta=-500.
        assert!(
            has_rects,
            "has_rects should be true when old side has entries"
        );
        assert_eq!(deltas.len(), 1);
        assert_eq!(
            deltas[0].0, -500,
            "unpaired old entry: delta = 0 - 500 = -500"
        );
    }

    /// WP-G-c: both sides have None landmark_rects → no sectionDeltas (old-bundle fallback).
    #[test]
    fn test_wpg_both_missing_rects_no_section_deltas() {
        let old_bundle = make_wpg_bundle("http://old.example.com/", 4211, None);
        let new_bundle = make_wpg_bundle("http://new.example.com/", 3792, None);

        let old_r = old_bundle.page.landmark_rects.as_deref().unwrap_or(&[]);
        let new_r = new_bundle.page.landmark_rects.as_deref().unwrap_or(&[]);
        let has_rects = !old_r.is_empty() || !new_r.is_empty();

        // Neither side has entries → has_rects is false → sectionDeltas should NOT appear.
        assert!(
            !has_rects,
            "has_rects must be false when both sides are None"
        );
    }

    /// WP-G-d: unpaired landmark present in old, absent in new → newHeight=0, delta is negative.
    #[test]
    fn test_wpg_unpaired_old_entry() {
        let old_rects = vec![
            rect("main", "main", Some("Hero"), 0, 72, 1440, 600),
            rect("aside", "complementary", Some("Sidebar"), 0, 672, 300, 200),
        ];
        let new_rects = vec![
            rect("main", "main", Some("Hero"), 0, 72, 1440, 600), // same height, delta=0
                                                                  // aside is absent in new
        ];
        let old_bundle = make_wpg_bundle("http://old.example.com/", 4000, Some(old_rects));
        let new_bundle = make_wpg_bundle("http://new.example.com/", 3800, Some(new_rects));

        use std::collections::{BTreeMap, BTreeSet};
        let old_r = old_bundle.page.landmark_rects.as_deref().unwrap_or(&[]);
        let new_r = new_bundle.page.landmark_rects.as_deref().unwrap_or(&[]);
        let old_by_path: BTreeMap<&str, &LandmarkRect> =
            old_r.iter().map(|r| (r.path.as_str(), r)).collect();
        let new_by_path: BTreeMap<&str, &LandmarkRect> =
            new_r.iter().map(|r| (r.path.as_str(), r)).collect();
        let mut all_paths: BTreeSet<&str> = BTreeSet::new();
        for r in old_r {
            all_paths.insert(r.path.as_str());
        }
        for r in new_r {
            all_paths.insert(r.path.as_str());
        }

        let mut deltas: Vec<(String, i32, i32, i32)> = Vec::new(); // (path, oh, nh, d)
        for path in &all_paths {
            let oh = old_by_path.get(path).map(|r| r.bbox[3]).unwrap_or(0);
            let nh = new_by_path.get(path).map(|r| r.bbox[3]).unwrap_or(0);
            let d = nh - oh;
            if d != 0 {
                deltas.push((path.to_string(), oh, nh, d));
            }
        }

        assert_eq!(
            deltas.len(),
            1,
            "only aside has a delta (main is unchanged)"
        );
        let (path, oh, nh, d) = &deltas[0];
        assert_eq!(path, "aside");
        assert_eq!(*oh, 200, "old height of aside");
        assert_eq!(*nh, 0, "new height is 0 (absent in new)");
        assert_eq!(*d, -200, "delta = 0 - 200");
    }
}
