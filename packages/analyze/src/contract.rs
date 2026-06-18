//! Serde structs mirroring the JSON contract.
//!
//! DETERMINISM: All maps use BTreeMap (never HashMap) so iteration order is stable.
//! Field order in structs matches the spec JSON ordering.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// CaptureBundle (consumed by analyze)
// ---------------------------------------------------------------------------

/// The seam between capture (TS) and analyze (Rust).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureBundle {
    pub schema_version: String,
    pub captured_at: String,
    pub viewport: ViewportConfig,
    pub environment: Environment,
    pub determinism: CaptureDeterminism,
    pub page: PageModel,
    /// M4: computed styles per node id. Empty in M1.
    pub computed_styles: BTreeMap<String, BTreeMap<String, String>>,
    pub screenshots: Screenshots,
    /// M4: ancestor chain metadata + chains map. Absent in pre-M4 bundles (defaults to empty).
    #[serde(default)]
    pub style_candidates: StyleCandidates,
}

// ---------------------------------------------------------------------------
// M4: StyleCandidates (ancestor chain metadata)
// ---------------------------------------------------------------------------

/// Ancestor-chain metadata emitted by the capture layer (M4 §2.4).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleCandidates {
    /// Ancestor descriptors sorted by id (document order).
    pub ancestors: Vec<AncestorDescriptor>,
    /// Map from node id → ancestor ids in nearest-first order.
    /// BTreeMap for deterministic iteration.
    pub chains: BTreeMap<String, Vec<String>>,
    /// Budget used during capture.
    pub budget: u32,
    /// True when the budget was exceeded and some ancestors were dropped.
    pub truncated: bool,
    /// Number of ancestor entries dropped due to budget overflow.
    pub dropped_count: u32,
}

/// A single ancestor element descriptor (M4 §2.3).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AncestorDescriptor {
    pub id: String,
    pub tag: String,
    /// [x, y, w, h] in CSS pixels.
    pub bbox: [i32; 4],
    /// Distance from document root.
    pub depth: u32,
    pub css_selector: Option<String>,
    pub anchors: Anchors,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewportConfig {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub dsf: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub os: String,
    pub chromium_build: String,
    pub playwright: String,
    pub dsf: f64,
}

/// Status of a single determinism step.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StepStatus {
    Ran,
    Failed,
    Skipped,
}

impl StepStatus {
    /// Returns a worst-case precedence value: failed > skipped > ran.
    pub fn precedence(&self) -> u8 {
        match self {
            StepStatus::Failed => 2,
            StepStatus::Skipped => 1,
            StepStatus::Ran => 0,
        }
    }

    /// Merge two statuses, returning the worse one.
    pub fn worst(a: &StepStatus, b: &StepStatus) -> StepStatus {
        if a.precedence() >= b.precedence() {
            a.clone()
        } else {
            b.clone()
        }
    }
}

/// Per-element-type counts at one point in time.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrityCounts {
    pub heading_count: u32,
    pub image_count: u32,
    pub landmark_count: u32,
}

/// Pre/post-stabilization element count snapshots.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrityInventory {
    pub pre: IntegrityCounts,
    pub post: IntegrityCounts,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDeterminism {
    pub animations_disabled: StepStatus,
    pub reduced_motion: StepStatus,
    pub time_frozen: StepStatus,
    pub random_stubbed: StepStatus,
    pub fonts_ready: StepStatus,
    pub images_decoded: StepStatus,
    pub lazy_load_pass: StepStatus,
    pub settled: StepStatus,
    pub clicked: Vec<String>,
    pub hidden: Vec<String>,
    pub masked: Vec<String>,
    pub retried_without_time_freeze: bool,
    /// Pre/post stabilization page inventory. None when the evaluate failed or was not taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrity: Option<IntegrityInventory>,
}

impl CaptureDeterminism {
    /// Merge two determinism reports, taking worst per step.
    pub fn merge_worst(a: &CaptureDeterminism, b: &CaptureDeterminism) -> CaptureDeterminism {
        CaptureDeterminism {
            animations_disabled: StepStatus::worst(&a.animations_disabled, &b.animations_disabled),
            reduced_motion: StepStatus::worst(&a.reduced_motion, &b.reduced_motion),
            time_frozen: StepStatus::worst(&a.time_frozen, &b.time_frozen),
            random_stubbed: StepStatus::worst(&a.random_stubbed, &b.random_stubbed),
            fonts_ready: StepStatus::worst(&a.fonts_ready, &b.fonts_ready),
            images_decoded: StepStatus::worst(&a.images_decoded, &b.images_decoded),
            lazy_load_pass: StepStatus::worst(&a.lazy_load_pass, &b.lazy_load_pass),
            settled: StepStatus::worst(&a.settled, &b.settled),
            // Clicked/hidden/masked: union (sorted for determinism)
            clicked: {
                let mut v: Vec<String> =
                    a.clicked.iter().chain(b.clicked.iter()).cloned().collect();
                v.sort();
                v.dedup();
                v
            },
            hidden: {
                let mut v: Vec<String> = a.hidden.iter().chain(b.hidden.iter()).cloned().collect();
                v.sort();
                v.dedup();
                v
            },
            masked: {
                let mut v: Vec<String> = a.masked.iter().chain(b.masked.iter()).cloned().collect();
                v.sort();
                v.dedup();
                v
            },
            retried_without_time_freeze: a.retried_without_time_freeze
                || b.retried_without_time_freeze,
            // integrity: prefer the first Some value (a's, then b's).
            // Rationale: merge_worst is used across viewports for the same capture side;
            // if the first viewport has inventory data we keep it as representative.
            integrity: a.integrity.clone().or_else(|| b.integrity.clone()),
        }
    }

    /// Returns true if any of the confidence-relevant steps failed or were skipped.
    pub fn has_confidence_penalty(&self) -> bool {
        self.time_frozen != StepStatus::Ran
            || self.lazy_load_pass != StepStatus::Ran
            || self.fonts_ready != StepStatus::Ran
    }
}

/// A single link probe result recorded by the capture layer.
/// Shape emitted by capture (camelCase JSON):
/// { url, redirectChain, finalUrl, status, skipped, error }
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkProbe {
    /// Absolute, fragment-stripped, redacted URL that was probed.
    pub url: String,
    /// One entry per hop (pre-redirect URLs); empty when no redirect occurred.
    pub redirect_chain: Vec<String>,
    /// Final response URL after all redirects; null when skipped or errored before any response.
    pub final_url: Option<String>,
    /// Final response status code; null when skipped/errored.
    pub status: Option<i32>,
    /// Why the probe was skipped; null when not skipped.
    pub skipped: Option<String>,
    /// Error message; null when no error.
    pub error: Option<String>,
}

/// WP-G: geometry for a single landmark element or a direct section-child of main.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LandmarkRect {
    /// Role-based path: e.g. "main", "contentinfo", "banner[2]", "main › section[1]".
    pub path: String,
    /// ARIA landmark role.
    pub role: String,
    /// Text of first h1-h3 inside, capped at 80 chars. Null if absent.
    pub heading: Option<String>,
    /// [x, y, w, h] in CSS pixels (page coordinates, scroll-offset-adjusted).
    pub bbox: [i32; 4],
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageModel {
    pub url: String,
    pub final_url: String,
    pub redirect_chain: Vec<String>,
    pub status_code: u32,
    pub title: Option<String>,
    pub meta_description: Option<String>,
    pub canonical: Option<String>,
    pub lang: Option<String>,
    pub page_height: u32,
    pub nodes: Vec<SemanticNode>,
    pub landmarks: Vec<String>,
    /// WP-G: landmark geometry. Absent in pre-WP-G bundles; defaults to None.
    #[serde(default)]
    pub landmark_rects: Option<Vec<LandmarkRect>>,
    pub network: NetworkInfo,
    pub console: Vec<ConsoleEntry>,
    pub a11y: A11yInfo,
    /// Link probes recorded by the capture layer (M2).
    /// Present when CaptureConfig.probeLinks was true; otherwise empty.
    #[serde(default)]
    pub link_probes: Vec<LinkProbe>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticNode {
    pub id: String,
    pub kind: String,
    pub role: Option<String>,
    pub text: Option<String>,
    pub acc_name: Option<String>,
    pub href: Option<String>,
    pub image_alt: Option<String>,
    /// [x, y, w, h] in CSS pixels (page coordinates)
    pub bbox: [i32; 4],
    pub seq_index: u32,
    pub anchors: NodeAnchors,
    pub css_selector: Option<String>,
    // --- M3 fields (all default to None so pre-M3 bundles still parse) ---
    /// Links: href attribute exactly as authored in HTML (un-resolved). M3 §2.
    #[serde(default)]
    pub raw_href: Option<String>,
    /// Images: currentSrc||src resolved to absolute URL. M3 §2.
    #[serde(default)]
    pub src: Option<String>,
    /// Images: intrinsic pixel width. M3 §2.
    #[serde(default)]
    pub natural_width: Option<u32>,
    /// Images: intrinsic pixel height. M3 §2.
    #[serde(default)]
    pub natural_height: Option<u32>,
    /// Images: complete && naturalWidth > 0. M3 §2.
    #[serde(default)]
    pub loaded: Option<bool>,
    /// Headings: level 1–6 parsed from tag name. M3 §2.
    #[serde(default)]
    pub heading_level: Option<u8>,
}

impl SemanticNode {
    /// Return the node's distinctive anchor text for region linking.
    /// Priority: text > href > alt > aria_label
    pub fn distinctive_anchor(&self) -> Option<&str> {
        if self
            .anchors
            .text
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
        {
            return self.anchors.text.as_deref();
        }
        if self.href.as_deref().map(|s| !s.is_empty()).unwrap_or(false) {
            return self.href.as_deref();
        }
        if self
            .image_alt
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
        {
            return self.image_alt.as_deref();
        }
        if self
            .anchors
            .aria_label
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
        {
            return self.anchors.aria_label.as_deref();
        }
        None
    }

    /// Returns true if this node has a distinctive anchor for region linking.
    pub fn has_distinctive_anchor(&self) -> bool {
        self.distinctive_anchor().is_some()
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeAnchors {
    pub text: Option<String>,
    pub role: Option<String>,
    pub href: Option<String>,
    pub alt: Option<String>,
    pub aria_label: Option<String>,
    pub nearest_heading: Option<String>,
    pub landmark: Option<String>,
    pub ordinal_in_landmark: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInfo {
    pub requests: Vec<NetworkRequest>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkRequest {
    pub url: String,
    pub status: Option<u32>,
    #[serde(rename = "type")]
    pub request_type: Option<String>,
    pub failed: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleEntry {
    pub level: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct A11yInfo {
    pub violations: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Screenshots {
    pub full_page: String,
    pub viewport: String,
}

// ---------------------------------------------------------------------------
// DiffResult (produced by analyze)
// ---------------------------------------------------------------------------

/// A run-level warning surfacing capture-integrity or baseline staleness conditions.
///
/// `context` is always serialized (null when absent) to keep golden key-sets stable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunWarning {
    pub code: String,
    pub message: String,
    pub context: Option<serde_json::Value>,
}

/// Out-of-scope issues: those whose landmark was excluded by --scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutOfScope {
    pub count: u32,
    pub ids: Vec<String>,
}

/// Per-landmark aggregated scores (no `visual` — visual is page-global pixel data).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LandmarkScores {
    pub content: f64,
    pub structure: f64,
    pub style: f64,
    pub accessibility: f64,
    pub technical: f64,
    pub hygiene: f64,
}

/// The primary deliverable of the matchy tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffResult {
    pub schema_version: String,
    pub tool_version: String,
    pub run_id: String,
    pub old_url: String,
    pub new_url: String,
    pub parity_profile: String,
    pub status: Status,
    pub agent_summary: AgentSummary,
    pub scores: Scores,
    pub viewports: Vec<ViewportResult>,
    pub issues: Vec<Issue>,
    pub clusters: Vec<Cluster>,
    pub regions: Vec<Region>,
    pub suppressed: Suppressed,
    pub warnings: Vec<RunWarning>,
    pub scoped_to: Option<Vec<String>>,
    pub out_of_scope: OutOfScope,
    pub determinism: DeterminismSummary,
    pub artifacts: Artifacts,
}

impl DiffResult {
    /// Serialize to pretty JSON with trailing newline (deterministic output).
    pub fn to_json(&self) -> anyhow::Result<String> {
        let mut s = serde_json::to_string_pretty(self)?;
        s.push('\n');
        Ok(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Warn,
    Fail,
    Error,
}

impl Status {
    pub fn is_worse_than(&self, other: &Status) -> bool {
        self.rank() > other.rank()
    }

    pub fn rank(&self) -> u8 {
        match self {
            Status::Pass => 0,
            Status::Warn => 1,
            Status::Fail => 2,
            Status::Error => 3,
        }
    }

    pub fn worst(a: Status, b: Status) -> Status {
        if a.rank() >= b.rank() {
            a
        } else {
            b
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSummary {
    pub fixable_now: u32,
    /// BTreeMap for deterministic serialization order.
    pub by_type: BTreeMap<String, u32>,
    pub cluster_count: u32,
    pub region_count: u32,
    pub top_fixes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scores {
    pub visual: f64,
    pub content: f64,
    pub structure: f64,
    pub style: f64,
    pub accessibility: f64,
    pub technical: f64,
    pub hygiene: f64,
    /// Per-landmark aggregated scores (BTreeMap for deterministic order).
    /// Empty map `{}` at per-viewport level when not populated; filled at top level.
    pub by_landmark: BTreeMap<String, LandmarkScores>,
}

impl Scores {
    pub fn all_pass() -> Self {
        Scores {
            visual: 1.0,
            content: 1.0,
            structure: 1.0,
            style: 1.0,
            accessibility: 1.0,
            technical: 1.0,
            hygiene: 1.0,
            by_landmark: BTreeMap::new(),
        }
    }

    pub fn min_per_category(scores: &[Scores]) -> Scores {
        if scores.is_empty() {
            return Scores::all_pass();
        }
        // Fixed order reduction for determinism.
        let visual = scores
            .iter()
            .map(|s| s.visual)
            .fold(f64::INFINITY, f64::min);
        let content = scores
            .iter()
            .map(|s| s.content)
            .fold(f64::INFINITY, f64::min);
        let structure = scores
            .iter()
            .map(|s| s.structure)
            .fold(f64::INFINITY, f64::min);
        let style = scores.iter().map(|s| s.style).fold(f64::INFINITY, f64::min);
        let accessibility = scores
            .iter()
            .map(|s| s.accessibility)
            .fold(f64::INFINITY, f64::min);
        let technical = scores
            .iter()
            .map(|s| s.technical)
            .fold(f64::INFINITY, f64::min);
        let hygiene = scores
            .iter()
            .map(|s| s.hygiene)
            .fold(f64::INFINITY, f64::min);
        Scores {
            visual,
            content,
            structure,
            style,
            accessibility,
            technical,
            hygiene,
            // by_landmark is not aggregated via min — caller sets it separately.
            by_landmark: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewportResult {
    pub name: String,
    pub status: Status,
    pub issues: Vec<String>,
    pub artifacts: Artifacts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifacts {
    pub old: String,
    pub new: String,
    pub diff: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suppressed {
    pub count: u32,
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeterminismSummary {
    pub old: CaptureDeterminism,
    pub new: CaptureDeterminism,
}

// ---------------------------------------------------------------------------
// Issue
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub id: String,
    #[serde(rename = "type")]
    pub issue_type: IssueType,
    pub category: IssueCategory,
    pub severity: IssueSeverity,
    pub confidence: f64,
    pub viewport: String,
    pub locale: Option<String>,
    pub goal: Option<String>,
    pub message: String,
    pub locator: Locator,
    /// Open-schema evidence; shape varies by issue type.
    pub evidence: serde_json::Value,
    pub remediation: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueType {
    VisualRegionChanged,
    PageHeightChanged,
    MissingTitle,
    ChangedTitle,
    MissingMetaDescription,
    ChangedMetaDescription,
    MissingH1,
    ChangedH1,
    HeadingStructureChanged,
    MissingText,
    ChangedText,
    DuplicateText,
    MissingLink,
    ChangedLinkTarget,
    BrokenLink,
    ChangedLinkText,
    MissingImage,
    BrokenImage,
    ChangedAltText,
    MissingAltText,
    ChangedImageDimensions,
    MissingForm,
    ChangedForm,
    MissingFormField,
    ChangedRequiredField,
    MissingSubmit,
    ChangedCta,
    MissingButton,
    ComponentReordered,
    ComponentSwapped,
    StyleChanged,
    BackgroundGradientLost,
    BackgroundGradientChanged,
    AccessibilityRegression,
    AccessibilityImproved,
    StatusCodeMismatch,
    NetworkError,
    ConsoleError,
    LoadError,
    UrlTrailingSlash,
    UrlRedirectChain,
    UrlProtocolDowngrade,
    CanonicalMismatch,
    LocaleCaseInvalid,
    LocaleSeparatorInvalid,
    LocaleUnknown,
}

impl IssueType {
    pub fn as_str(&self) -> &'static str {
        match self {
            IssueType::VisualRegionChanged => "visual_region_changed",
            IssueType::PageHeightChanged => "page_height_changed",
            IssueType::MissingTitle => "missing_title",
            IssueType::ChangedTitle => "changed_title",
            IssueType::MissingMetaDescription => "missing_meta_description",
            IssueType::ChangedMetaDescription => "changed_meta_description",
            IssueType::MissingH1 => "missing_h1",
            IssueType::ChangedH1 => "changed_h1",
            IssueType::HeadingStructureChanged => "heading_structure_changed",
            IssueType::MissingText => "missing_text",
            IssueType::ChangedText => "changed_text",
            IssueType::DuplicateText => "duplicate_text",
            IssueType::MissingLink => "missing_link",
            IssueType::ChangedLinkTarget => "changed_link_target",
            IssueType::BrokenLink => "broken_link",
            IssueType::ChangedLinkText => "changed_link_text",
            IssueType::MissingImage => "missing_image",
            IssueType::BrokenImage => "broken_image",
            IssueType::ChangedAltText => "changed_alt_text",
            IssueType::MissingAltText => "missing_alt_text",
            IssueType::ChangedImageDimensions => "changed_image_dimensions",
            IssueType::MissingForm => "missing_form",
            IssueType::ChangedForm => "changed_form",
            IssueType::MissingFormField => "missing_form_field",
            IssueType::ChangedRequiredField => "changed_required_field",
            IssueType::MissingSubmit => "missing_submit",
            IssueType::ChangedCta => "changed_cta",
            IssueType::MissingButton => "missing_button",
            IssueType::ComponentReordered => "component_reordered",
            IssueType::ComponentSwapped => "component_swapped",
            IssueType::StyleChanged => "style_changed",
            IssueType::BackgroundGradientLost => "background_gradient_lost",
            IssueType::BackgroundGradientChanged => "background_gradient_changed",
            IssueType::AccessibilityRegression => "accessibility_regression",
            IssueType::AccessibilityImproved => "accessibility_improved",
            IssueType::StatusCodeMismatch => "status_code_mismatch",
            IssueType::NetworkError => "network_error",
            IssueType::ConsoleError => "console_error",
            IssueType::LoadError => "load_error",
            IssueType::UrlTrailingSlash => "url_trailing_slash",
            IssueType::UrlRedirectChain => "url_redirect_chain",
            IssueType::UrlProtocolDowngrade => "url_protocol_downgrade",
            IssueType::CanonicalMismatch => "canonical_mismatch",
            IssueType::LocaleCaseInvalid => "locale_case_invalid",
            IssueType::LocaleSeparatorInvalid => "locale_separator_invalid",
            IssueType::LocaleUnknown => "locale_unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueCategory {
    Visual,
    Content,
    Structure,
    Style,
    Accessibility,
    Technical,
    Hygiene,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

impl IssueSeverity {
    pub fn rank(&self) -> u8 {
        match self {
            IssueSeverity::Info => 0,
            IssueSeverity::Warning => 1,
            IssueSeverity::Error => 2,
            IssueSeverity::Critical => 3,
        }
    }

    pub fn weight(&self) -> f64 {
        match self {
            IssueSeverity::Info => 1.0,
            IssueSeverity::Warning => 2.0,
            IssueSeverity::Error => 4.0,
            IssueSeverity::Critical => 8.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Locator {
    pub anchors: Anchors,
    pub css_selector_old: Option<String>,
    pub css_selector_new: Option<String>,
    pub bbox_old: Option<[i32; 4]>,
    pub bbox_new: Option<[i32; 4]>,
    pub seq_index_old: Option<u32>,
    pub seq_index_new: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Anchors {
    pub text: Option<String>,
    pub role: Option<String>,
    pub href: Option<String>,
    pub alt: Option<String>,
    pub aria_label: Option<String>,
    pub nearest_heading: Option<String>,
    pub landmark: Option<String>,
    pub ordinal_in_landmark: Option<u32>,
}

impl Anchors {
    pub fn null() -> Self {
        Anchors {
            text: None,
            role: None,
            href: None,
            alt: None,
            aria_label: None,
            nearest_heading: None,
            landmark: None,
            ordinal_in_landmark: None,
        }
    }

    /// Anchor strength per spec §5.
    pub fn strength(&self) -> AnchorStrength {
        let has_high = self.text.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
            || self.href.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
            || self.alt.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
            || self
                .aria_label
                .as_deref()
                .map(|s| !s.is_empty())
                .unwrap_or(false);
        if has_high {
            return AnchorStrength::High;
        }
        let has_medium = self
            .nearest_heading
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
            || self
                .landmark
                .as_deref()
                .map(|s| !s.is_empty())
                .unwrap_or(false);
        if has_medium {
            return AnchorStrength::Medium;
        }
        AnchorStrength::Low
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorStrength {
    High,
    Medium,
    Low,
}

impl AnchorStrength {
    pub fn bonus(&self) -> f64 {
        match self {
            AnchorStrength::High => crate::config::locality_bonus::HIGH,
            AnchorStrength::Medium => crate::config::locality_bonus::MEDIUM,
            AnchorStrength::Low => crate::config::locality_bonus::LOW,
        }
    }
}

// ---------------------------------------------------------------------------
// Cluster
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Cluster {
    pub id: String,
    pub issue_ids: Vec<String>,
    pub shared_property: Option<String>,
    pub shared_landmark: Option<String>,
    pub summary: Option<String>,
}

// ---------------------------------------------------------------------------
// Region
// ---------------------------------------------------------------------------

/// A saturated ARIA-landmark region rollup: one work item representing all
/// issues anchored to a landmark whose structural damage crosses the saturation
/// threshold.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub id: String,
    pub landmark: String,
    pub saturation: f64,
    pub structural_count: u32,
    pub old_node_count: u32,
    pub member_issue_ids: Vec<String>,
    pub severity: IssueSeverity,
    pub summary: String,
}

// ---------------------------------------------------------------------------
// Orchestration helpers
// ---------------------------------------------------------------------------

/// Response from capture.cjs on stdout.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all_fields = "camelCase")]
#[serde(untagged)]
pub enum CaptureResponse {
    Ok { ok: bool, bundle_path: String },
    Err { ok: bool, error: CaptureError },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureError {
    pub code: String,
    pub message: String,
}

/// Config sent to capture.cjs on stdin.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureConfig {
    pub mode: String,
    pub url: String,
    pub out_dir: String,
    pub prefix: String,
    pub viewport: ViewportConfig,
    pub stabilization: StabilizationConfig,
    pub hide_selectors: Vec<String>,
    pub mask_selectors: Vec<String>,
    pub click_before_capture: Vec<String>,
    pub max_text_length: u32,
    pub redact_params: Vec<String>,
    /// Whether to probe same-site links for redirect chains (M2).
    /// Set to true only for the "new" side.
    pub probe_links: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StabilizationConfig {
    pub freeze_time: bool,
    pub fixed_time: String,
    pub stub_random: bool,
    pub random_seed: u64,
    pub network_idle_timeout_ms: u64,
    pub settle_ms: u64,
    pub lazy_scroll_step_px: u32,
}

impl Default for StabilizationConfig {
    fn default() -> Self {
        StabilizationConfig {
            freeze_time: true,
            fixed_time: "2026-01-01T00:00:00.000Z".to_string(),
            stub_random: true,
            random_seed: 1337,
            network_idle_timeout_ms: 15000,
            settle_ms: 1000,
            lazy_scroll_step_px: 800,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal CaptureDeterminism for use in test fixtures.
    fn make_det() -> CaptureDeterminism {
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

    /// Build a minimal DiffResult with empty regions and region_count = 0.
    fn make_minimal_diff_result(regions: Vec<Region>, region_count: u32) -> DiffResult {
        DiffResult {
            schema_version: "1.2".to_string(),
            tool_version: "0.0.0".to_string(),
            run_id: "2026-01-01T00-00-00Z".to_string(),
            old_url: "https://example.com/old".to_string(),
            new_url: "https://example.com/new".to_string(),
            parity_profile: "content-structure".to_string(),
            status: Status::Pass,
            agent_summary: AgentSummary {
                fixable_now: 0,
                by_type: BTreeMap::new(),
                cluster_count: 0,
                region_count,
                top_fixes: vec![],
            },
            scores: Scores::all_pass(),
            viewports: vec![],
            issues: vec![],
            clusters: vec![],
            regions,
            suppressed: Suppressed {
                count: 0,
                ids: vec![],
            },
            warnings: vec![],
            scoped_to: None,
            out_of_scope: OutOfScope {
                count: 0,
                ids: vec![],
            },
            determinism: DeterminismSummary {
                old: make_det(),
                new: make_det(),
            },
            artifacts: Artifacts {
                old: "desktop/old.png".to_string(),
                new: "desktop/new.png".to_string(),
                diff: "desktop/diff.png".to_string(),
            },
        }
    }

    /// Serde round-trip: DiffResult with empty regions serializes with
    /// `"regions": []` and `"regionCount": 0`, then deserializes equal.
    #[test]
    fn test_diff_result_empty_regions_round_trip() {
        let original = make_minimal_diff_result(vec![], 0);
        let json = original.to_json().expect("should serialize");
        let parsed: DiffResult = serde_json::from_str(&json).expect("should deserialize");

        // Key presence checks on the JSON value
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            val["regions"],
            serde_json::json!([]),
            "regions must serialize as empty array"
        );
        assert_eq!(
            val["agentSummary"]["regionCount"],
            serde_json::json!(0),
            "regionCount must serialize as 0"
        );

        // Round-trip equality (schema_version, regions, region_count)
        assert_eq!(parsed.schema_version, "1.2");
        assert_eq!(parsed.regions, vec![]);
        assert_eq!(parsed.agent_summary.region_count, 0);
    }

    /// Serde round-trip: DiffResult carrying one fully-populated Region
    /// round-trips equal and camelCase keys appear in the JSON.
    #[test]
    fn test_diff_result_one_region_round_trip() {
        let region = Region {
            id: "region_aabbccddeeff".to_string(),
            landmark: "contentinfo".to_string(),
            saturation: 0.86,
            structural_count: 44,
            old_node_count: 51,
            member_issue_ids: vec![
                "issue_000000000001".to_string(),
                "issue_000000000002".to_string(),
            ],
            severity: IssueSeverity::Error,
            summary: "contentinfo region: 44/51 structural nodes affected".to_string(),
        };

        let original = make_minimal_diff_result(vec![region.clone()], 1);
        let json = original.to_json().expect("should serialize");
        let parsed: DiffResult = serde_json::from_str(&json).expect("should deserialize");

        // Round-trip equality
        assert_eq!(parsed.regions.len(), 1);
        assert_eq!(parsed.regions[0], region);
        assert_eq!(parsed.agent_summary.region_count, 1);

        // camelCase key names in JSON
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        let r = &val["regions"][0];
        assert!(
            r.get("structuralCount").is_some(),
            "structuralCount key must exist"
        );
        assert!(
            r.get("oldNodeCount").is_some(),
            "oldNodeCount key must exist"
        );
        assert!(
            r.get("memberIssueIds").is_some(),
            "memberIssueIds key must exist"
        );
        assert_eq!(r["structuralCount"], serde_json::json!(44));
        assert_eq!(r["oldNodeCount"], serde_json::json!(51));
        assert_eq!(r["saturation"], serde_json::json!(0.86));
        assert_eq!(r["severity"], serde_json::json!("error"));
        assert_eq!(r["landmark"], serde_json::json!("contentinfo"));
    }
}
