//! Visual diff between two full-page PNGs (spec §11, M1.md §5.2).
//!
//! DETERMINISM: single-threaded pixel pass; all operations are purely positional
//! (row-major); no HashMap or map iteration affecting output.

use std::path::Path;

use anyhow::{bail, Context};
use image::{ImageBuffer, Rgb, RgbImage};

use crate::config::{GRID_CELL, MIN_REGION_AREA, PIXELMATCH_MAX_DELTA, PIXEL_THRESHOLD};

/// A rectangle [x, y, w, h].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn area(&self) -> u64 {
        self.w as u64 * self.h as u64
    }
}

/// A clustered changed region.
#[derive(Debug, Clone)]
pub struct Region {
    /// Tight bounding box of changed pixels in this component.
    pub bbox: Rect,
    /// Number of changed pixels in this region.
    pub changed_pixels: u64,
}

/// Output from `diff_images`.
pub struct DiffOutput {
    pub width: u32,
    pub common_height: u32,
    pub old_height: u32,
    pub new_height: u32,
    /// Number of changed pixels within the common area.
    pub changed_pixels: u64,
    /// changed / (width * common_height)
    pub page_changed_ratio: f64,
    /// Clustered regions of change, sorted by (y, x).
    pub regions: Vec<Region>,
    /// The diff image (full canvas = max(old_height, new_height)).
    pub diff_image: RgbImage,
}

// ---------------------------------------------------------------------------
// YIQ perceptual delta (pixelmatch formula)
// ---------------------------------------------------------------------------

/// Convert an RGB pixel (already composited on white for alpha) to YIQ.
/// Returns (Y, I, Q) in the pixelmatch scaling.
#[inline]
fn rgb_to_yiq(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let y = r * 0.29889531 + g * 0.58662247 + b * 0.11448223;
    let i = r * 0.59597799 - g * 0.27417610 - b * 0.32180189;
    let q = r * 0.21147017 - g * 0.52261711 + b * 0.31114694;
    (y, i, q)
}

/// Compute YIQ perceptual delta squared (normalized to [0, PIXELMATCH_MAX_DELTA]).
#[inline]
fn color_delta(r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> f64 {
    let (y1, i1, q1) = rgb_to_yiq(r1 as f64, g1 as f64, b1 as f64);
    let (y2, i2, q2) = rgb_to_yiq(r2 as f64, g2 as f64, b2 as f64);
    let dy = y1 - y2;
    let di = i1 - i2;
    let dq = q1 - q2;
    0.5053 * dy * dy + 0.299 * di * di + 0.1957 * dq * dq
}

/// Returns true if the pixel delta exceeds the threshold.
/// threshold=0.1 corresponds to the standard pixelmatch semantics:
///   maxDelta = PIXELMATCH_MAX_DELTA * threshold^2
#[inline]
fn is_changed(r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8, threshold: f64) -> bool {
    let delta = color_delta(r1, g1, b1, r2, g2, b2);
    let max_delta = PIXELMATCH_MAX_DELTA * threshold * threshold;
    delta > max_delta
}

// ---------------------------------------------------------------------------
// Alpha compositing on white
// ---------------------------------------------------------------------------

/// Composite RGBA pixel on white background, return RGB u8.
#[inline]
fn composite_on_white(r: u8, g: u8, b: u8, a: u8) -> (u8, u8, u8) {
    if a == 255 {
        return (r, g, b);
    }
    let af = a as f64 / 255.0;
    let rf = r as f64 / 255.0;
    let gf = g as f64 / 255.0;
    let bf = b as f64 / 255.0;
    let ro = (rf * af + (1.0 - af)) * 255.0;
    let go = (gf * af + (1.0 - af)) * 255.0;
    let bo = (bf * af + (1.0 - af)) * 255.0;
    (ro as u8, go as u8, bo as u8)
}

// ---------------------------------------------------------------------------
// Load PNG as RGBA
// ---------------------------------------------------------------------------

fn load_rgba(path: &Path) -> anyhow::Result<image::DynamicImage> {
    image::open(path).with_context(|| format!("failed to open PNG: {}", path.display()))
}

// ---------------------------------------------------------------------------
// Union-Find for 8-connected grid-cell clustering
// ---------------------------------------------------------------------------

struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<u32>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]]; // path compression
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, x: usize, y: usize) {
        let rx = self.find(x);
        let ry = self.find(y);
        if rx == ry {
            return;
        }
        if self.rank[rx] < self.rank[ry] {
            self.parent[rx] = ry;
        } else if self.rank[rx] > self.rank[ry] {
            self.parent[ry] = rx;
        } else {
            self.parent[ry] = rx;
            self.rank[rx] += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Main diff function
// ---------------------------------------------------------------------------

/// Diff two full-page PNGs and return the visual diff output.
///
/// Widths must match (same viewport); mismatch returns Err.
/// Common height = min(h_old, h_new). Only common area is pixel-compared.
pub fn diff_images(old_path: &Path, new_path: &Path) -> anyhow::Result<DiffOutput> {
    let old_img = load_rgba(old_path)?;
    let new_img = load_rgba(new_path)?;

    let old_rgba = old_img.to_rgba8();
    let new_rgba = new_img.to_rgba8();

    let width = old_rgba.width();
    let old_height = old_rgba.height();
    let new_height = new_rgba.height();

    if new_rgba.width() != width {
        bail!("PNG width mismatch: old={} new={}", width, new_rgba.width());
    }

    let common_height = old_height.min(new_height);
    let max_height = old_height.max(new_height);

    // --- Step 1: build changed-pixel mask (single-threaded, row-major) ---
    let total_common_pixels = width as u64 * common_height as u64;
    // Flat boolean mask: changed[y * width + x]
    let mut changed_mask: Vec<bool> = vec![false; total_common_pixels as usize];
    let mut changed_pixels: u64 = 0;

    for y in 0..common_height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let [r1, g1, b1, a1] = old_rgba.get_pixel(x, y).0;
            let [r2, g2, b2, a2] = new_rgba.get_pixel(x, y).0;
            let (r1c, g1c, b1c) = composite_on_white(r1, g1, b1, a1);
            let (r2c, g2c, b2c) = composite_on_white(r2, g2, b2, a2);
            if is_changed(r1c, g1c, b1c, r2c, g2c, b2c, PIXEL_THRESHOLD) {
                changed_mask[idx] = true;
                changed_pixels += 1;
            }
        }
    }

    let page_changed_ratio = if total_common_pixels == 0 {
        0.0
    } else {
        changed_pixels as f64 / total_common_pixels as f64
    };

    // --- Step 2: 16px-cell grid + 8-connected union-find ---
    let grid_w = width.div_ceil(GRID_CELL);
    let grid_h = common_height.div_ceil(GRID_CELL);
    let num_cells = (grid_w * grid_h) as usize;

    // Mark which grid cells contain ≥1 changed pixel
    let mut cell_has_change: Vec<bool> = vec![false; num_cells];
    for y in 0..common_height {
        for x in 0..width {
            if changed_mask[(y * width + x) as usize] {
                let cx = x / GRID_CELL;
                let cy = y / GRID_CELL;
                cell_has_change[(cy * grid_w + cx) as usize] = true;
            }
        }
    }

    // 8-connected union-find over cells (row-major order)
    let mut uf = UnionFind::new(num_cells);
    for cy in 0..grid_h {
        for cx in 0..grid_w {
            let c = (cy * grid_w + cx) as usize;
            if !cell_has_change[c] {
                continue;
            }
            // Check 8 neighbors with (row, col) < current to avoid double-counting
            // Offsets: (-1,-1), (-1,0), (-1,+1), (0,-1)
            let neighbors: [(i32, i32); 4] = [(-1, -1), (-1, 0), (-1, 1), (0, -1)];
            for (dy, dx) in neighbors {
                let ny = cy as i32 + dy;
                let nx = cx as i32 + dx;
                if ny < 0 || nx < 0 || ny >= grid_h as i32 || nx >= grid_w as i32 {
                    continue;
                }
                let n = (ny as u32 * grid_w + nx as u32) as usize;
                if cell_has_change[n] {
                    uf.union(c, n);
                }
            }
        }
    }

    // Collect components: component_id -> (min_px_x, min_px_y, max_px_x, max_px_y, changed_px)
    // Use BTreeMap for determinism.
    use std::collections::BTreeMap;
    // key = canonical root of component
    let mut components: BTreeMap<usize, (u32, u32, u32, u32, u64)> = BTreeMap::new();

    // Process pixels within common area to compute tight bboxes
    for y in 0..common_height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            if !changed_mask[idx] {
                continue;
            }
            let cx = x / GRID_CELL;
            let cy = y / GRID_CELL;
            let cell_idx = (cy * grid_w + cx) as usize;
            let root = uf.find(cell_idx);
            let entry = components.entry(root).or_insert((x, y, x, y, 0));
            entry.0 = entry.0.min(x);
            entry.1 = entry.1.min(y);
            entry.2 = entry.2.max(x);
            entry.3 = entry.3.max(y);
            entry.4 += 1;
        }
    }

    // Build regions: filter by minRegionArea, sort by (y, x)
    let mut regions: Vec<Region> = components
        .into_values()
        .filter_map(|(min_x, min_y, max_x, max_y, cp)| {
            let w = max_x - min_x + 1;
            let h = max_y - min_y + 1;
            let area = w as u64 * h as u64;
            if area >= MIN_REGION_AREA {
                Some(Region {
                    bbox: Rect {
                        x: min_x,
                        y: min_y,
                        w,
                        h,
                    },
                    changed_pixels: cp,
                })
            } else {
                None
            }
        })
        .collect();

    // Sort regions by (y, x) for determinism
    regions.sort_by_key(|r| (r.bbox.y, r.bbox.x));

    // --- Step 3: Build diff image ---
    // Canvas height = max(old_height, new_height)
    let mut diff_img = ImageBuffer::<Rgb<u8>, _>::new(width, max_height);

    // Common area: grayscale-faded old as base
    for y in 0..common_height {
        for x in 0..width {
            let [r, g, b, a] = old_rgba.get_pixel(x, y).0;
            let (r, g, b) = composite_on_white(r, g, b, a);
            // Grayscale fade: luminance with reduced intensity
            let luma = (0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64) as u8;
            let faded = (luma as f64 * 0.5) as u8 + 40; // darken a bit
            diff_img.put_pixel(x, y, Rgb([faded, faded, faded]));
        }
    }

    // Overlay changed pixels in solid red
    for y in 0..common_height {
        for x in 0..width {
            if changed_mask[(y * width + x) as usize] {
                diff_img.put_pixel(x, y, Rgb([255, 0, 0]));
            }
        }
    }

    // Rows beyond common_height (taller side): neutral gray
    for y in common_height..max_height {
        for x in 0..width {
            diff_img.put_pixel(x, y, Rgb([160, 160, 160]));
        }
    }

    Ok(DiffOutput {
        width,
        common_height,
        old_height,
        new_height,
        changed_pixels,
        page_changed_ratio,
        regions,
        diff_image: diff_img,
    })
}

/// Save a diff image as PNG.
pub fn save_png(img: &RgbImage, path: &Path) -> anyhow::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create dir: {}", parent.display()))?;
    img.save(path)
        .with_context(|| format!("failed to save PNG: {}", path.display()))
}

/// Crop a region from a full-page PNG (with padding, clamped to image bounds).
pub fn crop_region(
    img: &image::DynamicImage,
    bbox: &Rect,
    pad: u32,
) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let rgba = img.to_rgba8();
    let iw = rgba.width();
    let ih = rgba.height();

    let x0 = bbox.x.saturating_sub(pad).min(iw.saturating_sub(1));
    let y0 = bbox.y.saturating_sub(pad).min(ih.saturating_sub(1));
    let x1 = (bbox.x + bbox.w + pad).min(iw);
    let y1 = (bbox.y + bbox.h + pad).min(ih);

    let cw = x1.saturating_sub(x0).max(1);
    let ch = y1.saturating_sub(y0).max(1);

    let mut crop = ImageBuffer::<Rgb<u8>, Vec<u8>>::new(cw, ch);
    for cy in 0..ch {
        for cx in 0..cw {
            let px = x0 + cx;
            let py = y0 + cy;
            if px < iw && py < ih {
                let [r, g, b, a] = rgba.get_pixel(px, py).0;
                let (r, g, b) = composite_on_white(r, g, b, a);
                crop.put_pixel(cx, cy, Rgb([r, g, b]));
            } else {
                crop.put_pixel(cx, cy, Rgb([255, 255, 255]));
            }
        }
    }
    crop
}

/// Crop a region from the diff image (already RGB, no alpha compositing needed).
pub fn crop_diff_region(diff: &RgbImage, bbox: &Rect, pad: u32) -> RgbImage {
    let iw = diff.width();
    let ih = diff.height();

    let x0 = bbox.x.saturating_sub(pad).min(iw.saturating_sub(1));
    let y0 = bbox.y.saturating_sub(pad).min(ih.saturating_sub(1));
    let x1 = (bbox.x + bbox.w + pad).min(iw);
    let y1 = (bbox.y + bbox.h + pad).min(ih);

    let cw = x1.saturating_sub(x0).max(1);
    let ch = y1.saturating_sub(y0).max(1);

    let mut crop = RgbImage::new(cw, ch);
    for cy in 0..ch {
        for cx in 0..cw {
            let px = x0 + cx;
            let py = y0 + cy;
            if px < iw && py < ih {
                crop.put_pixel(cx, cy, *diff.get_pixel(px, py));
            } else {
                crop.put_pixel(cx, cy, Rgb([160, 160, 160]));
            }
        }
    }
    crop
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn solid_rgba(w: u32, h: u32, r: u8, g: u8, b: u8) -> RgbaImage {
        RgbaImage::from_fn(w, h, |_, _| Rgba([r, g, b, 255]))
    }

    fn save_rgba_png(img: &RgbaImage, dir: &TempDir, name: &str) -> PathBuf {
        let p = dir.path().join(name);
        img.save(&p).unwrap();
        p
    }

    #[test]
    fn test_yiq_threshold_sanity() {
        // White vs black — maximum delta, should be changed at 0.1
        assert!(is_changed(0, 0, 0, 255, 255, 255, 0.1));
        // Identical pixels — delta = 0, never changed
        assert!(!is_changed(128, 128, 128, 128, 128, 128, 0.1));
        // Very similar colors — should not be changed at default threshold
        assert!(!is_changed(100, 100, 100, 101, 101, 101, 0.1));
    }

    #[test]
    fn test_common_height_no_false_flood() {
        // Synthetic: old=100x200 white, new=100x300 white.
        // First 200 rows identical -> zero changed pixels; one page_height_changed.
        let tmp = TempDir::new().unwrap();
        let old_img = solid_rgba(100, 200, 255, 255, 255);
        let new_img = solid_rgba(100, 300, 255, 255, 255);
        let old_path = save_rgba_png(&old_img, &tmp, "old.png");
        let new_path = save_rgba_png(&new_img, &tmp, "new.png");

        let out = diff_images(&old_path, &new_path).unwrap();
        assert_eq!(out.common_height, 200);
        assert_eq!(out.changed_pixels, 0, "no changed pixels in common area");
        assert_eq!(out.page_changed_ratio, 0.0);
        assert!(out.regions.is_empty(), "no regions from identical overlap");
        // Height mismatch is detected at the orchestration layer.
        assert_ne!(out.old_height, out.new_height);
    }

    #[test]
    fn test_width_mismatch_error() {
        let tmp = TempDir::new().unwrap();
        let old_img = solid_rgba(100, 200, 255, 255, 255);
        let new_img = solid_rgba(200, 200, 255, 255, 255);
        let old_path = save_rgba_png(&old_img, &tmp, "old.png");
        let new_path = save_rgba_png(&new_img, &tmp, "new.png");

        assert!(diff_images(&old_path, &new_path).is_err());
    }

    #[test]
    fn test_region_clustering_two_blobs() {
        // 200x200 white image. Two distinct black blobs far apart -> two regions.
        // Blob 1: [0..60, 0..60] (3600 px²) - above MIN_REGION_AREA=2500
        // Blob 2: [140..200, 140..200] (3600 px²) - above MIN_REGION_AREA=2500
        // Gap of 80px between them (> GRID_CELL=16), so two separate clusters.
        let tmp = TempDir::new().unwrap();

        let mut old_img = solid_rgba(200, 200, 255, 255, 255);
        let new_img = solid_rgba(200, 200, 255, 255, 255);
        // Mark region 1 as black in old (so it differs from white new)
        for y in 0..60u32 {
            for x in 0..60u32 {
                old_img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        // Mark region 2 as black in old
        for y in 140..200u32 {
            for x in 140..200u32 {
                old_img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }

        let old_path = save_rgba_png(&old_img, &tmp, "old.png");
        let new_path = save_rgba_png(&new_img, &tmp, "new.png");

        let out = diff_images(&old_path, &new_path).unwrap();
        assert_eq!(out.regions.len(), 2, "should have two separate regions");
        // Regions sorted by (y, x)
        assert!(out.regions[0].bbox.y <= out.regions[1].bbox.y);
    }

    #[test]
    fn test_region_clustering_adjacent_cells_merge() {
        // Two blobs within one GRID_CELL gap -> should merge into one region.
        // Blob 1: [0..55, 0..55] and Blob 2: [60..115, 60..115].
        // Gap of 5px < GRID_CELL=16 -> same grid cell region -> merged.
        let tmp = TempDir::new().unwrap();

        let mut old_img = solid_rgba(200, 200, 255, 255, 255);
        let new_img = solid_rgba(200, 200, 255, 255, 255);
        for y in 0..55u32 {
            for x in 0..55u32 {
                old_img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        for y in 60..115u32 {
            for x in 60..115u32 {
                old_img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }

        let old_path = save_rgba_png(&old_img, &tmp, "old.png");
        let new_path = save_rgba_png(&new_img, &tmp, "new.png");

        let out = diff_images(&old_path, &new_path).unwrap();
        assert_eq!(
            out.regions.len(),
            1,
            "adjacent blobs should merge into one region"
        );
    }

    #[test]
    fn test_region_min_area_filter() {
        // A tiny 10x10 changed area (100 px²) should not emit a region (MIN=2500).
        let tmp = TempDir::new().unwrap();

        let mut old_img = solid_rgba(200, 200, 255, 255, 255);
        let new_img = solid_rgba(200, 200, 255, 255, 255);
        // Tiny change: 10x10 = 100 px² (below 2500 threshold)
        for y in 90..100u32 {
            for x in 90..100u32 {
                old_img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }

        let old_path = save_rgba_png(&old_img, &tmp, "old.png");
        let new_path = save_rgba_png(&new_img, &tmp, "new.png");

        let out = diff_images(&old_path, &new_path).unwrap();
        assert!(
            out.regions.is_empty(),
            "tiny region should be filtered out (area {} < {})",
            100u32,
            MIN_REGION_AREA
        );
    }
}
