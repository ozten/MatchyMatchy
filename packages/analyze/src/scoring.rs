//! Scoring and severity mapping (M1.md §3.2, §5.4).
//!
//! DETERMINISM: fix_value is computed per-issue; sort uses total order ending in id.

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
        // load_error and status_code_mismatch are always critical regardless of profile
        if *issue_type == IssueType::LoadError || *issue_type == IssueType::StatusCodeMismatch {
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
            clicked: vec![],
            hidden: vec![],
            masked: vec![],
            retried_without_time_freeze: false,
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
            clicked: vec![],
            hidden: vec![],
            masked: vec![],
            retried_without_time_freeze: false,
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
            clicked: vec![],
            hidden: vec![],
            masked: vec![],
            retried_without_time_freeze: false,
        };

        // 0.9 * 0.8 = 0.72
        let conf = compute_confidence(0.9, false, &det_bad, &det_ok);
        assert_eq!(conf, 0.72);
    }
}
