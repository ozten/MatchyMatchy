//! Constants for the analyze layer (M1.md §5.6).
//! A config-file layer is deferred; these are the defaults.

/// Pixel-level change threshold for YIQ perceptual delta (0–1).
/// Delta > pixelThreshold => pixel is "changed".
pub const PIXEL_THRESHOLD: f64 = 0.1;

/// Minimum region area in px² to emit a visual_region_changed issue.
pub const MIN_REGION_AREA: u64 = 2500;

/// Minimum page-level changed-pixel ratio to emit visual_region_changed issues.
pub const VISUAL_THRESHOLD: f64 = 0.005;

/// Grid cell size in pixels for region clustering.
pub const GRID_CELL: u32 = 16;

/// Minimum overlap fraction (node bbox ∩ region / node bbox area) for anchor linking.
pub const REGION_NODE_OVERLAP: f64 = 0.3;

/// Padding in pixels when cropping region screenshots.
pub const CROP_PAD: u32 = 8;

/// Severity weights for fix-value calculation.
pub mod severity_weights {
    pub const INFO: f64 = 1.0;
    pub const WARNING: f64 = 2.0;
    pub const ERROR: f64 = 4.0;
    pub const CRITICAL: f64 = 8.0;
}

/// Locality bonus values based on anchor strength.
pub mod locality_bonus {
    pub const HIGH: f64 = 1.0;
    pub const MEDIUM: f64 = 0.7;
    pub const LOW: f64 = 0.4;
}

/// Base confidence values per issue type.
pub mod base_confidence {
    pub const VISUAL_REGION_CHANGED: f64 = 0.9;
    pub const PAGE_HEIGHT_CHANGED: f64 = 0.95;
    pub const LOAD_ERROR: f64 = 0.99;
}

/// The maxDelta constant from pixelmatch: 35215 is the maximum possible YIQ delta squared sum.
pub const PIXELMATCH_MAX_DELTA: f64 = 35215.0;
