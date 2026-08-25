//! Scoring and severity mapping (M1.md §3.2, §5.4).
//!
//! DETERMINISM: fix_value is computed per-issue; sort uses total order ending in id.

use std::collections::BTreeMap;

use crate::config;
use crate::contract::{
    AnchorStrength, CaptureDeterminism, IssueCategory, IssueSeverity, IssueType, Status,
};

/// Parity profile: controls how visual category maps to severity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParityProfile {
    ContentStructure,
    StrictVisual,
}

impl ParityProfile {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "content-structure" => Some(ParityProfile::ContentStructure),
            "strict-visual" => Some(ParityProfile::StrictVisual),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ParityProfile::ContentStructure => "content-structure",
            ParityProfile::StrictVisual => "strict-visual",
        }
    }

    /// Map an issue type + category to severity under this profile.
    pub fn severity_for(&self, issue_type: &IssueType, category: &IssueCategory) -> IssueSeverity {
        // M7 per-type overrides (before the category match)
        if *issue_type == IssueType::AccessibilityImproved {
            return IssueSeverity::Info;
        }
        if *issue_type == IssueType::ConsoleError {
            return IssueSeverity::Warning;
        }
        // load_error and status_code_mismatch are always critical regardless of profile
        if *issue_type == IssueType::LoadError || *issue_type == IssueType::StatusCodeMismatch {
            return IssueSeverity::Critical;
        }
        // missing_form is always critical: spec §13.1 fixture 5
        if *issue_type == IssueType::MissingForm {
            return IssueSeverity::Critical;
        }
        match category {
            IssueCategory::Visual => match self {
                ParityProfile::ContentStructure => IssueSeverity::Info,
                ParityProfile::StrictVisual => IssueSeverity::Error,
            },
            IssueCategory::Technical => IssueSeverity::Error,
            IssueCategory::Content => IssueSeverity::Error,
            IssueCategory::Structure => IssueSeverity::Error,
            // Spec §9 profile table: style is "warn" under content-structure,
            // "fail" under strict-visual; hygiene is "fail" under both.
            IssueCategory::Style => match self {
                ParityProfile::ContentStructure => IssueSeverity::Warning,
                ParityProfile::StrictVisual => IssueSeverity::Error,
            },
            IssueCategory::Accessibility => IssueSeverity::Warning,
            IssueCategory::Hygiene => IssueSeverity::Error,
        }
    }
}

// ---------------------------------------------------------------------------
// Severity resolution (port-parity U3)
// ---------------------------------------------------------------------------

/// Resolves the effective severity for an issue.
///
/// Resolution order, most-general first (design brief "Severity resolution
/// design"):
///   1. `ParityProfile::severity_for` category default (incl. its 4 pre-existing
///      hard per-type overrides — unchanged, embedded there).
///   2. Built-in per-type / per-property overrides
///      (`config::BUILTIN_TYPE_SEVERITY` / `config::BUILTIN_PROPERTY_SEVERITY`)
///      — property beats type within this layer.
///   3. User `--severity-map` per-type / per-property overrides — property
///      beats type within this layer; overrides both 1 and 2 when present.
///   4. Hard-Critical deny-list (`config::HARD_CRITICAL_TYPES`): enforced at
///      construction time in `with_user_map` — a "types" entry that would
///      demote a hard-Critical type below Critical is stripped from the
///      accepted map before the resolver is built, so `resolve` never has to
///      special-case it (there is simply no entry to find).
///
/// The uncertain-pairing forced-Info demotion in `style_diff.rs` runs AFTER
/// this resolver returns, unchanged (takes the min with Info).
pub struct SeverityResolver {
    profile: ParityProfile,
    user_types: BTreeMap<String, IssueSeverity>,
    user_properties: BTreeMap<String, IssueSeverity>,
}

impl SeverityResolver {
    /// No user overrides — profile + built-ins only. This is the
    /// `--severity-map`-absent case (the default).
    pub fn from_profile(profile: ParityProfile) -> Self {
        SeverityResolver {
            profile,
            user_types: BTreeMap::new(),
            user_properties: BTreeMap::new(),
        }
    }

    /// Build a resolver from a validated user map, enforcing the hard-Critical
    /// deny-list (layer 4). Returns the resolver plus the DENIED entries (type
    /// wire-name -> attempted severity) so the caller can surface a
    /// `severity_map_denied` run warning; denied entries are excluded from
    /// the resolver (and therefore from the DiffResult echo — "the ACCEPTED
    /// overrides").
    ///
    /// `user_types` / `user_properties` are assumed already validated (wire
    /// names / CSS properties both real) by `load_user_severity_map` — this
    /// constructor only enforces the deny-list.
    pub fn with_user_map(
        profile: ParityProfile,
        user_types: BTreeMap<String, IssueSeverity>,
        user_properties: BTreeMap<String, IssueSeverity>,
    ) -> (Self, BTreeMap<String, IssueSeverity>) {
        let mut accepted_types = BTreeMap::new();
        let mut denied: BTreeMap<String, IssueSeverity> = BTreeMap::new();
        for (type_key, sev) in user_types {
            let is_hard_critical = config::HARD_CRITICAL_TYPES.contains(&type_key.as_str());
            if is_hard_critical && sev.rank() < IssueSeverity::Critical.rank() {
                denied.insert(type_key, sev);
            } else {
                accepted_types.insert(type_key, sev);
            }
        }
        (
            SeverityResolver {
                profile,
                user_types: accepted_types,
                user_properties,
            },
            denied,
        )
    }

    /// The underlying parity profile (layer 1) — needed by callers that also
    /// serialize `DiffResult.parity_profile` from the same value.
    pub fn profile(&self) -> &ParityProfile {
        &self.profile
    }

    /// Accepted user-map type overrides currently in effect (post deny-list).
    /// For the DiffResult `severity_map` echo.
    pub fn accepted_types(&self) -> &BTreeMap<String, IssueSeverity> {
        &self.user_types
    }

    /// Accepted user-map property overrides currently in effect. For the
    /// DiffResult `severity_map` echo.
    pub fn accepted_properties(&self) -> &BTreeMap<String, IssueSeverity> {
        &self.user_properties
    }

    /// Resolve severity for a property-less issue (type + category only).
    pub fn severity_for(&self, issue_type: &IssueType, category: &IssueCategory) -> IssueSeverity {
        self.resolve(issue_type, category, None)
    }

    /// Resolve severity for a property-carrying issue: `style_changed` and the
    /// gradient types, on any style channel (leaf, ancestor, or the future
    /// pseudo channel), keyed on the issue's CSS property
    /// (`remediation.property`).
    pub fn severity_for_property(
        &self,
        issue_type: &IssueType,
        category: &IssueCategory,
        property: &str,
    ) -> IssueSeverity {
        self.resolve(issue_type, category, Some(property))
    }

    fn resolve(
        &self,
        issue_type: &IssueType,
        category: &IssueCategory,
        property: Option<&str>,
    ) -> IssueSeverity {
        // 3. User map — property beats type.
        if let Some(p) = property {
            if let Some(s) = self.user_properties.get(p) {
                return s.clone();
            }
        }
        if let Some(s) = self.user_types.get(issue_type.as_str()) {
            return s.clone();
        }
        // 2. Built-ins — property beats type.
        if let Some(p) = property {
            if let Some((_, s)) = config::BUILTIN_PROPERTY_SEVERITY
                .iter()
                .find(|(k, _)| *k == p)
            {
                return s.clone();
            }
        }
        if let Some((_, s)) = config::BUILTIN_TYPE_SEVERITY
            .iter()
            .find(|(k, _)| *k == issue_type.as_str())
        {
            return s.clone();
        }
        // 1. Profile category default (incl. its 4 embedded hard overrides).
        self.profile.severity_for(issue_type, category)
    }
}

/// Raw (string-valued) shape of a `--severity-map` JSON file, before
/// key/value validation.
#[derive(Debug, serde::Deserialize)]
struct SeverityMapFile {
    #[serde(default)]
    types: BTreeMap<String, String>,
    #[serde(default)]
    properties: BTreeMap<String, String>,
}

/// Load, parse, and validate a `--severity-map` JSON file (port-parity U3).
///
/// Validation (all are hard errors, matching the design brief):
///   - an unknown "types" key (not an `IssueType::parse`-able wire name), or
///   - an unknown "properties" key (not in `config::STYLE_DIFF_PROPERTIES`)
///   - any severity value not one of info|warning|error|critical
///   - malformed JSON
///
/// Each produces a descriptive `Err` for the CLI to report and exit 2 on —
/// this function never panics and never silently drops an unrecognised key.
///
/// The hard-Critical deny-list is NOT enforced here — that's
/// `SeverityResolver::with_user_map`'s job (it needs the accepted/denied
/// split, this function just validates and parses).
pub fn load_user_severity_map(
    path: &std::path::Path,
) -> anyhow::Result<(
    BTreeMap<String, IssueSeverity>,
    BTreeMap<String, IssueSeverity>,
)> {
    use anyhow::Context;

    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read severity-map file '{}'", path.display()))?;
    let raw: SeverityMapFile = serde_json::from_str(&text)
        .with_context(|| format!("malformed JSON in severity-map file '{}'", path.display()))?;

    let mut types = BTreeMap::new();
    for (key, value) in raw.types {
        if IssueType::parse(&key).is_none() {
            anyhow::bail!(
                "severity-map: unknown issue type '{}' in \"types\" (not a recognised IssueType)",
                key
            );
        }
        let sev = IssueSeverity::parse(&value).ok_or_else(|| {
            anyhow::anyhow!(
                "severity-map: unknown severity '{}' for type '{}' (expected info|warning|error|critical)",
                value,
                key
            )
        })?;
        types.insert(key, sev);
    }

    let mut properties = BTreeMap::new();
    for (key, value) in raw.properties {
        if !config::STYLE_DIFF_PROPERTIES.contains(&key.as_str()) {
            anyhow::bail!(
                "severity-map: unknown CSS property '{}' in \"properties\" (not in the captured style-diff property list)",
                key
            );
        }
        let sev = IssueSeverity::parse(&value).ok_or_else(|| {
            anyhow::anyhow!(
                "severity-map: unknown severity '{}' for property '{}' (expected info|warning|error|critical)",
                value,
                key
            )
        })?;
        properties.insert(key, sev);
    }

    Ok((types, properties))
}

/// Compute the fix value for sorting.
///
/// fix_value = severity_weight * confidence * locality_bonus
/// Ordered descending; tie-break ascending id.
pub fn fix_value(
    severity: &IssueSeverity,
    confidence: f64,
    anchor_strength: &AnchorStrength,
) -> f64 {
    severity.weight() * confidence * anchor_strength.bonus()
}

/// Compute confidence from base value and bundle determinism states.
///
/// base: 0.9 visual_region_changed, 0.95 page_height_changed, 0.99 load_error
///
/// * 0.5 if environment fingerprints differ
/// * 0.8 (once) if timeFrozen/lazyLoadPass/fontsReady is failed/skipped on either bundle
///
/// Round to 4 decimals.
pub fn compute_confidence(
    base: f64,
    env_mismatch: bool,
    old_det: &CaptureDeterminism,
    new_det: &CaptureDeterminism,
) -> f64 {
    let mut conf = base;
    if env_mismatch {
        conf *= 0.5;
    }
    // Apply *0.8 once if any confidence-penalty step failed/skipped on either bundle.
    if old_det.has_confidence_penalty() || new_det.has_confidence_penalty() {
        conf *= 0.8;
    }
    // Round to 4 decimals
    (conf * 10000.0).round() / 10000.0
}

/// Determine overall run status from a list of post-profile severities.
pub fn compute_status(severities: &[IssueSeverity]) -> Status {
    let mut status = Status::Pass;
    for sev in severities {
        let candidate = match sev {
            IssueSeverity::Critical | IssueSeverity::Error => Status::Fail,
            IssueSeverity::Warning => Status::Warn,
            IssueSeverity::Info => Status::Pass,
        };
        if candidate.is_worse_than(&status) {
            status = candidate;
        }
    }
    status
}

/// Count fixable issues: severity >= warning AND anchor strength >= medium AND remediation non-null.
pub fn count_fixable_now(issues: &[crate::contract::Issue]) -> u32 {
    issues
        .iter()
        .filter(|i| {
            i.severity.rank() >= IssueSeverity::Warning.rank()
                && i.locator.anchors.strength() != AnchorStrength::Low
                && i.remediation.is_some()
        })
        .count() as u32
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{AnchorStrength, IssueSeverity, Status};

    #[test]
    fn test_m7_severity_overrides() {
        let cs = ParityProfile::ContentStructure;
        let sv = ParityProfile::StrictVisual;

        // accessibility_improved → Info regardless of profile
        assert_eq!(
            cs.severity_for(
                &IssueType::AccessibilityImproved,
                &IssueCategory::Accessibility
            ),
            IssueSeverity::Info
        );
        assert_eq!(
            sv.severity_for(
                &IssueType::AccessibilityImproved,
                &IssueCategory::Accessibility
            ),
            IssueSeverity::Info
        );

        // console_error → Warning regardless of profile
        assert_eq!(
            cs.severity_for(&IssueType::ConsoleError, &IssueCategory::Technical),
            IssueSeverity::Warning
        );
        assert_eq!(
            sv.severity_for(&IssueType::ConsoleError, &IssueCategory::Technical),
            IssueSeverity::Warning
        );
    }

    #[test]
    fn test_status_profile_mapping() {
        let profile = ParityProfile::ContentStructure;

        // Visual -> info under content-structure
        assert_eq!(
            profile.severity_for(&IssueType::VisualRegionChanged, &IssueCategory::Visual),
            IssueSeverity::Info
        );

        // Visual -> error under strict-visual
        let strict = ParityProfile::StrictVisual;
        assert_eq!(
            strict.severity_for(&IssueType::VisualRegionChanged, &IssueCategory::Visual),
            IssueSeverity::Error
        );

        // load_error always critical
        assert_eq!(
            profile.severity_for(&IssueType::LoadError, &IssueCategory::Technical),
            IssueSeverity::Critical
        );
    }

    #[test]
    fn test_compute_status() {
        assert_eq!(compute_status(&[]), Status::Pass);
        assert_eq!(compute_status(&[IssueSeverity::Info]), Status::Pass);
        assert_eq!(compute_status(&[IssueSeverity::Warning]), Status::Warn);
        assert_eq!(compute_status(&[IssueSeverity::Error]), Status::Fail);
        assert_eq!(compute_status(&[IssueSeverity::Critical]), Status::Fail);
        assert_eq!(
            compute_status(&[IssueSeverity::Info, IssueSeverity::Warning]),
            Status::Warn
        );
        assert_eq!(
            compute_status(&[IssueSeverity::Warning, IssueSeverity::Error]),
            Status::Fail
        );
    }

    #[test]
    fn test_fix_value_ordering_total_order() {
        // Critical + high anchor > warning + low anchor
        let fv_critical_high = fix_value(&IssueSeverity::Critical, 0.99, &AnchorStrength::High);
        let fv_warning_low = fix_value(&IssueSeverity::Warning, 0.5, &AnchorStrength::Low);
        assert!(fv_critical_high > fv_warning_low);

        // Equal fix values: sorting must use id as tiebreaker (tested in sort logic, not here)
        let fv1 = fix_value(&IssueSeverity::Info, 1.0, &AnchorStrength::High);
        let fv2 = fix_value(&IssueSeverity::Info, 1.0, &AnchorStrength::High);
        assert!((fv1 - fv2).abs() < 1e-10);
    }

    #[test]
    fn test_confidence_rounding() {
        use crate::contract::CaptureDeterminism;
        use crate::contract::StepStatus;

        let det_ok = CaptureDeterminism {
            animations_disabled: StepStatus::Ran,
            reduced_motion: StepStatus::Ran,
            time_frozen: StepStatus::Ran,
            random_stubbed: StepStatus::Ran,
            fonts_ready: StepStatus::Ran,
            images_decoded: StepStatus::Ran,
            lazy_load_pass: StepStatus::Ran,
            settled: StepStatus::Ran,
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
        };

        let conf = compute_confidence(0.9, false, &det_ok, &det_ok);
        assert_eq!(conf, 0.9);

        // With env mismatch: 0.9 * 0.5 = 0.45
        let conf2 = compute_confidence(0.9, true, &det_ok, &det_ok);
        assert_eq!(conf2, 0.45);
    }

    #[test]
    fn test_confidence_with_penalty() {
        use crate::contract::CaptureDeterminism;
        use crate::contract::StepStatus;

        let det_bad = CaptureDeterminism {
            animations_disabled: StepStatus::Ran,
            reduced_motion: StepStatus::Ran,
            time_frozen: StepStatus::Skipped, // penalty
            random_stubbed: StepStatus::Ran,
            fonts_ready: StepStatus::Ran,
            images_decoded: StepStatus::Ran,
            lazy_load_pass: StepStatus::Ran,
            settled: StepStatus::Ran,
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
        };
        let det_ok = CaptureDeterminism {
            animations_disabled: StepStatus::Ran,
            reduced_motion: StepStatus::Ran,
            time_frozen: StepStatus::Ran,
            random_stubbed: StepStatus::Ran,
            fonts_ready: StepStatus::Ran,
            images_decoded: StepStatus::Ran,
            lazy_load_pass: StepStatus::Ran,
            settled: StepStatus::Ran,
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
        };

        // 0.9 * 0.8 = 0.72
        let conf = compute_confidence(0.9, false, &det_bad, &det_ok);
        assert_eq!(conf, 0.72);
    }

    // -----------------------------------------------------------------------
    // SeverityResolver (port-parity U3)
    // -----------------------------------------------------------------------

    fn map(pairs: &[(&str, IssueSeverity)]) -> BTreeMap<String, IssueSeverity> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    /// Default run (no user map): a letter-spacing style_changed demotes to
    /// Info via the built-in property table, while a color style_changed
    /// stays at profile severity (Warning under content-structure).
    #[test]
    fn test_builtin_property_demotes_letter_spacing_not_color() {
        let resolver = SeverityResolver::from_profile(ParityProfile::ContentStructure);

        let letter_spacing_sev = resolver.severity_for_property(
            &IssueType::StyleChanged,
            &IssueCategory::Style,
            "letter-spacing",
        );
        assert_eq!(letter_spacing_sev, IssueSeverity::Info);

        let color_sev = resolver.severity_for_property(
            &IssueType::StyleChanged,
            &IssueCategory::Style,
            "color",
        );
        assert_eq!(
            color_sev,
            ParityProfile::ContentStructure
                .severity_for(&IssueType::StyleChanged, &IssueCategory::Style)
        );
        assert_eq!(color_sev, IssueSeverity::Warning);
    }

    /// `clickable_area_regressed` resolves to Error under BOTH profiles with
    /// no user map (category Visual would otherwise give Info under
    /// content-structure).
    #[test]
    fn test_clickable_area_regressed_error_both_profiles() {
        for profile in [ParityProfile::ContentStructure, ParityProfile::StrictVisual] {
            let resolver = SeverityResolver::from_profile(profile);
            let sev =
                resolver.severity_for(&IssueType::ClickableAreaRegressed, &IssueCategory::Visual);
            assert_eq!(sev, IssueSeverity::Error);
        }
    }

    /// A user map promoting `pseudo_element_missing` to error under
    /// content-structure is honored (and would be echoed by the caller from
    /// `accepted_types()`).
    #[test]
    fn test_user_map_promotes_pseudo_element_missing() {
        let user_types = map(&[("pseudo_element_missing", IssueSeverity::Error)]);
        let (resolver, denied) = SeverityResolver::with_user_map(
            ParityProfile::ContentStructure,
            user_types,
            BTreeMap::new(),
        );
        assert!(denied.is_empty());
        let sev = resolver.severity_for(&IssueType::PseudoElementMissing, &IssueCategory::Style);
        assert_eq!(sev, IssueSeverity::Error);
        assert_eq!(
            resolver.accepted_types().get("pseudo_element_missing"),
            Some(&IssueSeverity::Error)
        );
    }

    /// A user map demoting `status_code_mismatch` to info is denied: the
    /// resolved severity stays Critical, and the denial is reported back to
    /// the caller (never silently dropped, never silently honored).
    #[test]
    fn test_user_map_demotion_of_hard_critical_is_denied() {
        let user_types = map(&[("status_code_mismatch", IssueSeverity::Info)]);
        let (resolver, denied) = SeverityResolver::with_user_map(
            ParityProfile::ContentStructure,
            user_types,
            BTreeMap::new(),
        );
        assert_eq!(
            denied.get("status_code_mismatch"),
            Some(&IssueSeverity::Info)
        );
        assert!(resolver.accepted_types().is_empty());
        let sev = resolver.severity_for(&IssueType::StatusCodeMismatch, &IssueCategory::Technical);
        assert_eq!(sev, IssueSeverity::Critical);
    }

    /// A user map that promotes a hard-Critical type is NOT denied (only
    /// demotions below Critical are).
    #[test]
    fn test_user_map_hard_critical_promotion_to_critical_not_denied() {
        let user_types = map(&[("load_error", IssueSeverity::Critical)]);
        let (_resolver, denied) = SeverityResolver::with_user_map(
            ParityProfile::ContentStructure,
            user_types,
            BTreeMap::new(),
        );
        assert!(denied.is_empty());
    }

    /// Property override beats type override: a map with BOTH
    /// {"types": {"style_changed": "error"}} and
    /// {"properties": {"letter-spacing": "info"}} resolves a letter-spacing
    /// style_changed to Info (property wins).
    #[test]
    fn test_property_override_beats_type_override() {
        let user_types = map(&[("style_changed", IssueSeverity::Error)]);
        let user_properties = map(&[("letter-spacing", IssueSeverity::Info)]);
        let (resolver, denied) = SeverityResolver::with_user_map(
            ParityProfile::ContentStructure,
            user_types,
            user_properties,
        );
        assert!(denied.is_empty());

        let letter_spacing_sev = resolver.severity_for_property(
            &IssueType::StyleChanged,
            &IssueCategory::Style,
            "letter-spacing",
        );
        assert_eq!(letter_spacing_sev, IssueSeverity::Info);

        // A DIFFERENT property on the same type still gets the type override.
        let other_sev = resolver.severity_for_property(
            &IssueType::StyleChanged,
            &IssueCategory::Style,
            "color",
        );
        assert_eq!(other_sev, IssueSeverity::Error);
    }

    /// `load_user_severity_map` parses a well-formed file into validated
    /// (type, property) severity maps.
    #[test]
    fn test_load_user_severity_map_happy_path() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "matchy_severity_map_test_{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"{"types": {"pseudo_element_missing": "error"}, "properties": {"letter-spacing": "info"}}"#,
        )
        .unwrap();

        let (types, properties) = load_user_severity_map(&path).expect("should parse");
        std::fs::remove_file(&path).ok();

        assert_eq!(
            types.get("pseudo_element_missing"),
            Some(&IssueSeverity::Error)
        );
        assert_eq!(properties.get("letter-spacing"), Some(&IssueSeverity::Info));
    }

    /// An unknown "types" key is a hard error (the CLI turns this into exit 2).
    #[test]
    fn test_load_user_severity_map_unknown_type_key_errors() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "matchy_severity_map_bad_type_{}.json",
            std::process::id()
        ));
        std::fs::write(&path, r#"{"types": {"not_a_real_type": "error"}}"#).unwrap();

        let result = load_user_severity_map(&path);
        std::fs::remove_file(&path).ok();
        assert!(result.is_err());
    }

    /// An unknown "properties" key is a hard error.
    #[test]
    fn test_load_user_severity_map_unknown_property_key_errors() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "matchy_severity_map_bad_prop_{}.json",
            std::process::id()
        ));
        std::fs::write(&path, r#"{"properties": {"not-a-real-property": "info"}}"#).unwrap();

        let result = load_user_severity_map(&path);
        std::fs::remove_file(&path).ok();
        assert!(result.is_err());
    }

    /// Malformed JSON is a hard error.
    #[test]
    fn test_load_user_severity_map_malformed_json_errors() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "matchy_severity_map_malformed_{}.json",
            std::process::id()
        ));
        std::fs::write(&path, "{not valid json").unwrap();

        let result = load_user_severity_map(&path);
        std::fs::remove_file(&path).ok();
        assert!(result.is_err());
    }
}
