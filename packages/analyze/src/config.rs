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
    /// Hygiene issues (HTTP metadata facts — no env/determinism multipliers).
    pub const HYGIENE: f64 = 0.98;
    /// Status code mismatch (highest confidence — clear HTTP fact).
    pub const STATUS_CODE_MISMATCH: f64 = 0.99;
    /// Page-level metadata facts (title, meta-description, h1 presence). M3.md §5.2.
    pub const PAGE_FACT: f64 = 0.97;
    /// Matched pair attribute diff — stage Identity. M3.md §5.3.
    pub const CONTENT_IDENTITY: f64 = 0.95;
    /// Matched pair attribute diff — stage Assignment. M3.md §5.3.
    pub const CONTENT_ASSIGNMENT: f64 = 0.90;
    /// broken_link (HTTP fact — no env/determinism multipliers). M3.md §5.4.
    pub const BROKEN_LINK: f64 = 0.98;
    /// broken_image (lazy-load dependent — env multipliers still apply). M3.md §5.3.
    pub const BROKEN_IMAGE: f64 = 0.95;
    /// Generic style_changed issue. M4.md §3.4.
    pub const STYLE_CHANGED: f64 = 0.9;
    /// Background gradient issue. M4.md §3.4.
    pub const GRADIENT: f64 = 0.95;
    /// Network request failure (new-only 4xx/5xx or failed flag). M7-introduced; not under the
    /// M6 freeze; calibratable at real-pair use.
    pub const NETWORK_ERROR: f64 = 0.95;
    /// New-only error-level console message. M7-introduced; not under the M6 freeze;
    /// calibratable at real-pair use.
    pub const CONSOLE_ERROR: f64 = 0.9;
    /// A11y rule set diff (axe-core violations). M7-introduced; not under the M6 freeze;
    /// calibratable at real-pair use.
    pub const A11Y: f64 = 0.95;
}

/// Minimum style similarity for ancestor pairing (M4.md §3.2).
pub const ANCESTOR_MIN_SIMILARITY: f64 = 0.6;

/// Diff property list: curated set MINUS the `background` shorthand (M4.md §3.1).
/// The `background` shorthand is excluded to avoid double-reporting with
/// `background-color` and `background-image`.
pub const STYLE_DIFF_PROPERTIES: &[&str] = &[
    "color",
    "background-color",
    "background-image",
    "border",
    "border-radius",
    "box-shadow",
    "font-family",
    "font-size",
    "font-weight",
    "line-height",
    "letter-spacing",
    "text-align",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "display",
    "position",
    "opacity",
    "flex-direction",
    "justify-content",
    "align-items",
    "gap",
    "grid-template-columns",
];

// ---------------------------------------------------------------------------
// M3 matcher thresholds (M3.md §3.6)
//
// FROZEN at M6 real-pair calibration (docs/calibration-note.md, 2026-06-11).
// The matching floors/margins/weights, tiebreak weights, and the visual
// thresholds above were validated against three real page pairs
// (archive-vs-live, live-vs-live, golden-vs-live): zero false positives at
// the live-vs-live noise floor, no unexplained missing/added on the drift
// pair, and ambiguous pairings landing in the uncertain band as designed.
// Changing any of them requires recalibration per spec §12 M6 and a
// golden-changelog entry.
// ---------------------------------------------------------------------------

/// Identity score floor: pairs ≥ this and mutual-unique → stage Identity. M3.md §3.6.
pub const IDENTITY_FLOOR: f64 = 0.85;

/// Both-sides gap required to declare a unique mutual best (stage 1). M3.md §3.6.
pub const TIE_MARGIN: f64 = 0.05;

/// Combined-score floor: pairs ≥ this after stage 2 → band Matched. M3.md §3.6.
pub const MATCH_FLOOR: f64 = 0.70;

/// Combined-score ceiling: pairs below this are forbidden (never assigned). M3.md §3.6.
pub const NO_MATCH_CEIL: f64 = 0.45;

/// Maximum block side-count for Hungarian solver; above this → greedy. M3.md §3.6.
pub const HUNGARIAN_MAX: usize = 128;

/// Confidence multiplier applied to all issues emitted from Uncertain-band pairs. M3.md §3.6.
pub const UNCERTAIN_MULTIPLIER: f64 = 0.6;

/// Confidence penalty for issues whose anchor landmark is browser chrome (banner/nav/footer). M3.md §5.3 D9.
pub const CHROME_PENALTY: f64 = 0.85;

// ---------------------------------------------------------------------------
// M3 stage-2 combination weights (M3.md §3.5)
// ---------------------------------------------------------------------------

/// Weight of identity score in combined score: combined = STAGE2_IDENTITY_WEIGHT·id + STAGE2_TIEBREAK_WEIGHT·tb. M3.md §3.5.
pub const STAGE2_IDENTITY_WEIGHT: f64 = 0.7;

/// Weight of tiebreak score in combined score. M3.md §3.5.
pub const STAGE2_TIEBREAK_WEIGHT: f64 = 0.3;

/// Tiebreak sub-weight: position (normalised page-y ratio). M3.md §3.5.
pub const TIEBREAK_POS: f64 = 0.5;

/// Tiebreak sub-weight: size (bbox area ratio). M3.md §3.5.
pub const TIEBREAK_SIZE: f64 = 0.3;

/// Tiebreak sub-weight: nearby context (nearestHeading + landmark). M3.md §3.5.
pub const TIEBREAK_NEARBY: f64 = 0.2;

/// Minimum per-axis intrinsic dimension ratio to suppress changed_image_dimensions. M3.md §5.3.
pub const IMAGE_DIM_RATIO_FLOOR: f64 = 0.9;

/// Minimum displacement constant for sequence-diff reorder emission.
/// A `component_reordered` is emitted only when block displacement (in eligible-pair
/// rank units) **strictly exceeds** this value; displacement == SEQ_MIN_DISPLACEMENT is
/// suppressed as extraction jitter or a knock-on shift from a nearby removal.
/// Swaps (exchanges) are never thresholded.
pub const SEQ_MIN_DISPLACEMENT: u32 = 2;

/// Trailing-slash policy (M2.md §5.2 item 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrailingSlashPolicy {
    /// Paths must NOT have a trailing slash (except root "/").
    Never,
    /// Paths must have a trailing slash (except root "/").
    Always,
    /// New page slash state must match old page slash state.
    Preserve,
}

/// Default trailing-slash policy (Never per M2.md §5.2).
pub const DEFAULT_TRAILING_SLASH_POLICY: TrailingSlashPolicy = TrailingSlashPolicy::Never;

/// The maxDelta constant from pixelmatch: 35215 is the maximum possible YIQ delta squared sum.
pub const PIXELMATCH_MAX_DELTA: f64 = 35215.0;

// ---------------------------------------------------------------------------
// M6 calibration constants
// ---------------------------------------------------------------------------

/// Bbox containment tolerance (px) for the duplicate-label text-node filter.
///
/// When a text node's bbox is contained within a link/button node's bbox up to
/// this tolerance, the text node is treated as a nested label duplicate and
/// suppressed from matcher input. Value chosen from M6 real-pair calibration.
pub const DUP_LABEL_BBOX_TOLERANCE_PX: f64 = 2.0;

/// Numeric epsilon for style-value sub-pixel jitter suppression.
///
/// Two numeric tokens with the same trailing unit that differ by less than
/// this amount are treated as equal during style comparison. Calibrated from
/// M6 live-page observations (e.g. "19.5776px" vs "19.6px").
/// Note: 13px vs 14px (diff 1.0) is still reported.
pub const STYLE_NUMERIC_EPSILON: f64 = 0.1;
