//! Constants for the analyze layer (M1.md §5.6).
//! A config-file layer is deferred; these are the defaults.

use crate::contract::IssueSeverity;

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
    /// `clickable_area_regressed` (port-parity U7): per-point hit-test parity check.
    /// The detector already gates on a hard occlusion threshold before ever emitting
    /// (see `CLICKABLE_OLD_FLOOR` / `CLICKABLE_DELTA`), so a surviving true positive
    /// starts at high base confidence, same tier as `STYLE_CHANGED`.
    pub const CLICKABLE_AREA_REGRESSED: f64 = 0.9;
}

/// Minimum group size to emit a cluster (spec §7.4 clusterMin default).
pub const CLUSTER_MIN: usize = 3;

/// Minimum style similarity for ancestor pairing (M4.md §3.2).
pub const ANCESTOR_MIN_SIMILARITY: f64 = 0.6;

/// Diff property list: curated set MINUS the `background` shorthand (M4.md §3.1).
/// The `background` shorthand is excluded to avoid double-reporting with
/// `background-color` and `background-image`.
///
/// 32 properties total (33 captured minus `background`). Issue #4 / R4b added
/// `text-decoration-line` (NOT the `text-decoration` shorthand, which embeds
/// color and would be noise), `z-index`, `max-width`, `pointer-events` — same
/// fixed slice-order position as the newcomers in
/// `packages/capture/src/extract/page-model.ts` / `computed-style.ts`.
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
    "text-decoration-line",
    "z-index",
    "max-width",
    "pointer-events",
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

// ---------------------------------------------------------------------------
// Severity mapping: built-in overrides (port-parity U3, design brief §"Severity
// resolution design").
//
// Resolution order (most-general first), implemented by `scoring::SeverityResolver`:
//   1. `ParityProfile::severity_for` category default (incl. its 4 pre-existing
//      hard per-type overrides: accessibility_improved, console_error,
//      load_error/status_code_mismatch, missing_form — those stay embedded
//      there, unchanged).
//   2. The two tables below (property beats type within this layer).
//   3. An optional user `--severity-map` file (property beats type within this
//      layer; overrides both 1 and 2).
//   4. HARD_CRITICAL_TYPES deny-list, enforced at `SeverityResolver`
//      construction (a user-map demotion below Critical is stripped before
//      the resolver is built, never reaches resolution).
// ---------------------------------------------------------------------------

/// Built-in per-type severity overrides. Keys are `IssueType::as_str()` wire
/// names. Applied after the profile default, before any user map entry.
///
/// `clickable_area_regressed` carries `category: Visual` (spec §9's profile
/// table would otherwise map that to Info under content-structure) — but the
/// detector (U7's `hit_test_diff.rs`) already gates on a hard occlusion
/// threshold (old fraction >= 0.9 AND old-new delta > 0.1) before ever
/// emitting the issue, so a surviving true positive must never be silently
/// demoted to Info by the profile. Forced to Error regardless of profile.
pub const BUILTIN_TYPE_SEVERITY: &[(&str, IssueSeverity)] =
    &[("clickable_area_regressed", IssueSeverity::Error)];

/// Built-in per-property severity overrides, applied to property-carrying
/// style-channel issues (`style_changed` + the gradient types, on the leaf,
/// ancestor, and future pseudo channels), keyed on the issue's CSS property
/// (`remediation.property`). More specific than `BUILTIN_TYPE_SEVERITY`
/// (property beats type within this layer) and than the profile default.
///
/// `letter-spacing` and `line-height` are cascade-tail properties that fire at
/// high volume with low defect signal — the dominant contributors to issue
/// #4's 2,500-issue `style_changed` flood (docs/calibration-note.md). Demoted
/// to Info by default so a port-parity gate isn't drowned by sub-pixel
/// kerning/leading noise; `color`, `font-size`, `text-align`, and
/// `background-color` are deliberately NOT in this table and stay at profile
/// severity.
pub const BUILTIN_PROPERTY_SEVERITY: &[(&str, IssueSeverity)] = &[
    ("letter-spacing", IssueSeverity::Info),
    ("line-height", IssueSeverity::Info),
];

/// Hard-Critical issue types (wire names) that a user `--severity-map` can
/// never demote below Critical (gate-integrity deny-list; port-parity U3).
/// An attempted demotion is stripped from the accepted map at
/// `SeverityResolver` construction and surfaced as a `severity_map_denied`
/// run warning — never silently honored, never silently dropped without a
/// trace. Mirrors the existing hard overrides embedded in
/// `ParityProfile::severity_for` (layer 1), which already keep these types at
/// Critical with no user map present at all.
pub const HARD_CRITICAL_TYPES: &[&str] = &["load_error", "status_code_mismatch", "missing_form"];

/// Minimum per-axis intrinsic dimension ratio to suppress changed_image_dimensions. M3.md §5.3.
pub const IMAGE_DIM_RATIO_FLOOR: f64 = 0.9;

/// Maximum fractional difference in aspect ratio before two images are considered
/// aspect-changed in responsive mode (docs/bugs/p2-07). 0.02 = 2%.
pub const ASPECT_RATIO_TOLERANCE: f64 = 0.02;

/// Image-dimension comparison mode (docs/bugs/p2-07).
///
/// `Strict`     — flag any naturalWidth/Height change (default). Goldens recorded in this mode.
/// `Responsive` — pass aspect-preserving downscales that still cover the rendered box;
///               flag upscales, aspect changes, and undersized images.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageDimensionsMode {
    Strict,
    Responsive,
}

impl ImageDimensionsMode {
    /// Parse from CLI string. Returns `None` on unrecognised value.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "strict" => Some(Self::Strict),
            "responsive" => Some(Self::Responsive),
            _ => None,
        }
    }

    /// Canonical string representation (round-trips with `parse`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Responsive => "responsive",
        }
    }
}

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

// ---------------------------------------------------------------------------
// Progressive-disclosure budget (feat: agent-first progressive disclosure).
//
// FROZEN at U6 calibration (docs/calibration-note.md §7, 2026-06-18) against the
// frozen p01 bundle via `matchy analyze` replay (NEVER a live capture — p1-03:
// captures are not byte-stable; analyze of a frozen bundle is). The compact
// report inlines section branches in fix-value order until the cumulative
// rendered-size (character) proxy would exceed DISCLOSURE_BUDGET; the rest
// collapse to one-line drill pointers. Bands, not a knife-edge: saturated
// regions and any section larger than DISCLOSURE_SECTION_CEILING always collapse
// (high watermark); a section set whose total fits the budget inlines wholesale
// (low watermark, R4). Byte-identical DiffResult -> byte-identical projection
// (R3/AE2).
//
// Calibration evidence (p01 desktop, 272 issues; per-section compact inline
// sizes in chars): content/defect sections = {385, 539, 539, 927, 937, 1048,
// 1137}; flood sections (18/27/30-issue style+visual dumps) = {1965, 2107, 3071}.
// A wide natural gap separates the two clusters (1137 -> 1965, 828 chars).
//   * CEILING = 1500 sits mid-gap: the broken_link section (1137) inlines with
//     363 chars (24%) headroom; the three flood sections always collapse with
//     >=465 chars (31%) margin. (The earlier 1200 left only a fragile 69-char
//     margin on the defect section — message-length variance could have flipped
//     it and buried the broken_link, violating R13.)
//   * BUDGET = 3000 inlines the top-3 content sections (cumulative 2570, 430
//     headroom); the next section by fix-value (937) would reach 3507, a 507-char
//     overshoot. The inline/collapse boundary therefore sits inside a 937-char
//     gap, so a +-1-char jitter cannot flip a branch (R3).
// On p01 this yields: contentinfo region collapsed to one pointer, the standalone
// broken_link surfaced as the first inlined item (R13), 3 sections inlined / 7
// collapsed, report.md 6.9 KB vs 19.9 KB full (~65% smaller). Second-page budget
// generalization (home / branded-call) is deferred validation, not a pre-freeze
// blocker (mirrors how 0.6/10 were frozen on p01).
// ---------------------------------------------------------------------------

/// Cumulative rendered-size budget (chars) for inlined compact-report section detail.
pub const DISCLOSURE_BUDGET: usize = 3000;

/// Per-section rendered-size ceiling (chars): a single section larger than this
/// always collapses to a pointer even if the cumulative budget is not yet spent.
pub const DISCLOSURE_SECTION_CEILING: usize = 1500;

/// Minimum pair score required to emit style issues at Warning/Error severity.
///
/// Bug p1-04: uncertain pairings (band != Matched OR score < this threshold)
/// produce style issues at Info severity only, which are excluded from the
/// style category score. This prevents 1592-issue saturation from uncertain
/// cross-engine pairings dominating the style score.
pub const MIN_PAIRING_SCORE_FOR_STYLE: f64 = 0.75;

// ---------------------------------------------------------------------------
// Clickable-area hit-test thresholds (port-parity U7, design brief "Detector").
//
// Issue #4's suggested thresholds, frozen here (recalibration requires a
// golden-changelog entry, same discipline as the M3 matcher constants above).
// ---------------------------------------------------------------------------

/// Minimum surviving denominator (after excluding clipped/offViewport points on
/// either side and both-side misses) required to evaluate the clickable-area
/// parity ratio. Below this, the sample is too small to trust — guards
/// degenerate geometry (tiny/odd-shaped interactive elements) per plan U7.
/// The grid is 25 points (5x5); 9 is roughly a third of the grid.
pub const MIN_HIT_DENOMINATOR: usize = 9;

/// Minimum adjusted old-side hit fraction required before a clickable-area
/// regression can fire. Below this, the old side was already partly occluded
/// and a further drop isn't a clean "it used to work, now it doesn't" signal.
pub const CLICKABLE_OLD_FLOOR: f64 = 0.9;

/// Minimum (adjusted old − adjusted new) hit-fraction drop required to fire
/// `clickable_area_regressed`.
pub const CLICKABLE_DELTA: f64 = 0.1;

/// Confidence multiplier applied to `clickable_area_regressed` when either
/// bundle's determinism shows the settle stage did not cleanly reach
/// quiescence (`quiescence == timeout`) or the settle step itself
/// failed/was skipped. Absent settle fields (pre-settle bundles) mean NO
/// demotion — settle simply never ran, which is not itself a red flag.
pub const CLICKABLE_SETTLE_DEMOTION: f64 = 0.7;

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// WP-I: ImageDimensionsMode::parse round-trips with as_str.
    #[test]
    fn test_image_dims_mode_parse_roundtrip() {
        assert_eq!(
            ImageDimensionsMode::parse("strict"),
            Some(ImageDimensionsMode::Strict)
        );
        assert_eq!(
            ImageDimensionsMode::parse("responsive"),
            Some(ImageDimensionsMode::Responsive)
        );
        assert_eq!(
            ImageDimensionsMode::Strict.as_str(),
            "strict",
            "Strict.as_str() must be 'strict'"
        );
        assert_eq!(
            ImageDimensionsMode::Responsive.as_str(),
            "responsive",
            "Responsive.as_str() must be 'responsive'"
        );
    }

    /// WP-I: Unrecognised value returns None.
    #[test]
    fn test_image_dims_mode_parse_bad_value() {
        assert_eq!(ImageDimensionsMode::parse("unknown"), None);
        assert_eq!(ImageDimensionsMode::parse(""), None);
        assert_eq!(ImageDimensionsMode::parse("Strict"), None);
    }

    /// port-parity U3: the deny-list is exactly the three hard-Critical types
    /// named in the design brief — pins it against accidental drift.
    #[test]
    fn test_hard_critical_types_frozen_set() {
        assert_eq!(
            HARD_CRITICAL_TYPES,
            &["load_error", "status_code_mismatch", "missing_form"]
        );
    }

    /// port-parity U3: built-in per-type override table pins `clickable_area_regressed`
    /// -> Error and nothing else (adding entries here is a deliberate, evidence-backed
    /// change, not an accident).
    #[test]
    fn test_builtin_type_severity_frozen() {
        assert_eq!(BUILTIN_TYPE_SEVERITY.len(), 1);
        assert_eq!(BUILTIN_TYPE_SEVERITY[0].0, "clickable_area_regressed");
        assert_eq!(BUILTIN_TYPE_SEVERITY[0].1, IssueSeverity::Error);
    }

    /// port-parity U3: built-in per-property table pins the two cascade-tail
    /// demotions and nothing else.
    #[test]
    fn test_builtin_property_severity_frozen() {
        assert_eq!(BUILTIN_PROPERTY_SEVERITY.len(), 2);
        let map: std::collections::BTreeMap<&str, IssueSeverity> =
            BUILTIN_PROPERTY_SEVERITY.iter().cloned().collect();
        assert_eq!(map.get("letter-spacing"), Some(&IssueSeverity::Info));
        assert_eq!(map.get("line-height"), Some(&IssueSeverity::Info));
    }

    /// port-parity U7: hit-test/clickable-area thresholds are frozen at the
    /// design-brief values (issue #4's suggested thresholds).
    #[test]
    fn test_clickable_area_thresholds_frozen() {
        assert_eq!(MIN_HIT_DENOMINATOR, 9);
        assert_eq!(CLICKABLE_OLD_FLOOR, 0.9);
        assert_eq!(CLICKABLE_DELTA, 0.1);
        assert_eq!(CLICKABLE_SETTLE_DEMOTION, 0.7);
        assert_eq!(base_confidence::CLICKABLE_AREA_REGRESSED, 0.9);
    }
}
