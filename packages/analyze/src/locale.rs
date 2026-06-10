//! Locale segment parser for M2 hygiene checks (M2.md §5.2 item 6).
//!
//! Detects and validates BCP-47 locale segments in URL paths.
//! Hand-rolled byte-level parsing; no regex crate.
//!
//! DETERMINISM: pure functions, no I/O, no random state.

use crate::locale_data;

/// The locale shape detected in a path segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocaleShape {
    /// `lang-REGION` or `lang_REGION` style (2-3 alpha + sep + 2 alpha or 3 digits)
    Separated {
        language: String,
        separator: char,
        region: String,
    },
    /// 2-letter bare language code (no region)
    Bare { language: String },
}

/// The validation result for a locale segment.
#[derive(Debug, Clone, Default)]
pub struct LocaleValidation {
    /// Whether the separator is invalid (underscore instead of hyphen)
    pub separator_invalid: bool,
    /// Whether the case is invalid (lang not lowercase or alpha region not uppercase)
    pub case_invalid: bool,
    /// Whether the language or region is unknown (not in ISO 639-1 / 3166-1)
    pub unknown: bool,
    /// The raw segment as found in the path
    pub raw_segment: String,
    /// The corrected segment (after applying all fixes)
    pub corrected_segment: String,
    /// The subtag that is unknown (for `locale_unknown` evidence)
    pub unknown_subtag: Option<String>,
}

/// Check whether a byte is ASCII alpha.
#[inline]
fn is_ascii_alpha(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

/// Check whether a byte is ASCII digit.
#[inline]
fn is_ascii_digit(b: u8) -> bool {
    b.is_ascii_digit()
}

/// Try to parse a path segment as a locale shape.
/// Returns None if the segment does not look like a locale at all.
pub fn parse_locale_segment(segment: &str) -> Option<LocaleShape> {
    let bytes = segment.as_bytes();
    let len = bytes.len();

    // --- Bare shape: exactly 2 ASCII alpha chars ---
    if len == 2 && is_ascii_alpha(bytes[0]) && is_ascii_alpha(bytes[1]) {
        let lower = segment.to_ascii_lowercase();
        // Only treat as locale if the lowercase form is a known ISO 639-1 code
        if locale_data::ISO_639_1
            .binary_search(&lower.as_str())
            .is_ok()
        {
            return Some(LocaleShape::Bare {
                language: segment.to_string(),
            });
        }
        return None;
    }

    // --- Separated shape: 2-3 alpha + ('-' or '_') + (2 alpha or 3 digits) ---
    // Find separator position (must be '-' or '_')
    let sep_pos = bytes.iter().position(|&b| b == b'-' || b == b'_')?;
    let lang_part = &segment[..sep_pos];
    let sep_char = bytes[sep_pos] as char;
    let region_part = &segment[sep_pos + 1..];

    let lang_bytes = lang_part.as_bytes();
    let region_bytes = region_part.as_bytes();

    // Language: 2 or 3 ASCII alpha chars
    let lang_len = lang_bytes.len();
    if !(2..=3).contains(&lang_len) {
        return None;
    }
    if !lang_bytes.iter().all(|&b| is_ascii_alpha(b)) {
        return None;
    }

    // Region: exactly 2 ASCII alpha OR exactly 3 ASCII digits
    let region_len = region_bytes.len();
    let region_is_alpha = region_len == 2 && region_bytes.iter().all(|&b| is_ascii_alpha(b));
    let region_is_digit = region_len == 3 && region_bytes.iter().all(|&b| is_ascii_digit(b));

    if !region_is_alpha && !region_is_digit {
        return None;
    }

    Some(LocaleShape::Separated {
        language: lang_part.to_string(),
        separator: sep_char,
        region: region_part.to_string(),
    })
}

/// Validate a locale shape and produce a `LocaleValidation` result.
/// Each rule is checked independently per M2.md §5.2 item 6.
pub fn validate_locale(shape: &LocaleShape, raw_segment: &str) -> LocaleValidation {
    match shape {
        LocaleShape::Bare { language } => {
            // Bare 2-letter: only validate case (must be lowercase).
            // We already know the lowercase form is in ISO 639-1 (parse_locale_segment checked).
            let case_invalid = language.chars().any(|c| c.is_ascii_uppercase());
            let corrected = language.to_ascii_lowercase();
            LocaleValidation {
                separator_invalid: false,
                case_invalid,
                unknown: false,
                raw_segment: raw_segment.to_string(),
                corrected_segment: corrected,
                unknown_subtag: None,
            }
        }
        LocaleShape::Separated {
            language,
            separator,
            region,
        } => {
            let region_bytes = region.as_bytes();
            let region_is_digit =
                region_bytes.len() == 3 && region_bytes.iter().all(|&b| is_ascii_digit(b));

            // Rule 1: separator must be '-'
            let separator_invalid = *separator == '_';

            // Rule 2: language must be all-lowercase; alpha region must be all-uppercase
            let lang_case_bad = language.chars().any(|c| c.is_ascii_uppercase());
            let region_case_bad =
                !region_is_digit && region.chars().any(|c| c.is_ascii_lowercase());
            let case_invalid = lang_case_bad || region_case_bad;

            // Rule 3: check known codes (case-normalized)
            // 3-letter language subtags → always locale_unknown (only 639-1 embedded)
            let lang_lower = language.to_ascii_lowercase();
            let lang_known = language.len() == 2
                && locale_data::ISO_639_1
                    .binary_search(&lang_lower.as_str())
                    .is_ok();

            let region_upper = region.to_ascii_uppercase();
            // 3-digit M49 regions are accepted unvalidated
            let region_known = region_is_digit
                || locale_data::ISO_3166_1_ALPHA2
                    .binary_search(&region_upper.as_str())
                    .is_ok();

            let unknown = !lang_known || !region_known;

            // Determine which subtag is unknown (for evidence)
            let unknown_subtag = if unknown {
                // Report whichever is unknown; prefer language if both
                if !lang_known {
                    Some(lang_lower.clone())
                } else {
                    Some(region_upper.clone())
                }
            } else {
                None
            };

            // Corrected segment: lang lowercase, hyphen separator, alpha region uppercase
            let corrected = format!(
                "{}-{}",
                lang_lower,
                if region_is_digit {
                    region.clone()
                } else {
                    region_upper
                }
            );

            LocaleValidation {
                separator_invalid,
                case_invalid,
                unknown,
                raw_segment: raw_segment.to_string(),
                corrected_segment: corrected,
                unknown_subtag,
            }
        }
    }
}

/// Find the first locale-shaped path segment in a URL path.
///
/// Checks first segment, then second (first locale-shaped match wins).
/// Returns (segment_string, validation) or None if no locale detected.
pub fn detect_locale_in_path(path: &str) -> Option<(String, LocaleValidation)> {
    // Split path into segments, skipping leading '/'
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    // Check first segment, then second
    for seg in segments.iter().take(2) {
        if let Some(shape) = parse_locale_segment(seg) {
            let validation = validate_locale(&shape, seg);
            // Only emit if there are any issues OR it's a valid locale
            // (we always return Some here so the caller can check)
            return Some((seg.to_string(), validation));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_es_mx_ok() {
        let (seg, v) = detect_locale_in_path("/es-MX/products").unwrap();
        assert_eq!(seg, "es-MX");
        assert!(!v.separator_invalid);
        assert!(!v.case_invalid);
        assert!(!v.unknown);
    }

    #[test]
    fn test_es_mx_separator_only() {
        let (seg, v) = detect_locale_in_path("/es_MX/products").unwrap();
        assert_eq!(seg, "es_MX");
        assert!(v.separator_invalid, "should flag separator invalid");
        assert!(!v.case_invalid, "case is actually ok (es lower, MX upper)");
        assert!(!v.unknown, "es-MX is known");
    }

    #[test]
    fn test_es_mx_case_only() {
        let (seg, v) = detect_locale_in_path("/es-mx/products").unwrap();
        assert_eq!(seg, "es-mx");
        assert!(!v.separator_invalid);
        assert!(v.case_invalid, "mx should be uppercase");
        assert!(!v.unknown, "es-MX is known after normalization");
    }

    #[test]
    fn test_es_hyphen_mx_case_upper_lang() {
        // ES-mx — lang uppercase + region lowercase
        let (seg, v) = detect_locale_in_path("/ES-mx/products").unwrap();
        assert_eq!(seg, "ES-mx");
        assert!(!v.separator_invalid);
        assert!(
            v.case_invalid,
            "ES should be lowercase and mx should be uppercase"
        );
        assert!(!v.unknown);
    }

    #[test]
    fn test_es_underscore_mx_case_uppercase_lang() {
        // ES_mx — separator + case
        let (seg, v) = detect_locale_in_path("/ES_mx/products").unwrap();
        assert_eq!(seg, "ES_mx");
        assert!(v.separator_invalid);
        assert!(v.case_invalid);
        assert!(!v.unknown);
    }

    #[test]
    fn test_xx_mx_unknown() {
        let (seg, v) = detect_locale_in_path("/xx-MX/products").unwrap();
        assert_eq!(seg, "xx-MX");
        assert!(!v.separator_invalid);
        assert!(!v.case_invalid);
        assert!(v.unknown, "xx is not a known language");
    }

    #[test]
    fn test_es_bare_ok() {
        // bare "es" — known, lowercase
        let (seg, v) = detect_locale_in_path("/es/products").unwrap();
        assert_eq!(seg, "es");
        assert!(!v.separator_invalid);
        assert!(!v.case_invalid);
        assert!(!v.unknown);
    }

    #[test]
    fn test_api_not_detected() {
        assert!(detect_locale_in_path("/api/products").is_none());
    }

    #[test]
    fn test_img_not_detected() {
        assert!(detect_locale_in_path("/img/logo.png").is_none());
    }

    #[test]
    fn test_products_not_detected() {
        // "products" is > 3 letters, not a locale shape
        assert!(detect_locale_in_path("/products/connect").is_none());
    }

    #[test]
    fn test_fil_ph_unknown() {
        // "fil" is 3-letter, only 639-1 embedded, so always locale_unknown
        let (seg, v) = detect_locale_in_path("/fil-PH/about").unwrap();
        assert_eq!(seg, "fil-PH");
        assert!(!v.separator_invalid);
        assert!(!v.case_invalid, "fil is lowercase, PH is uppercase");
        assert!(v.unknown, "fil is 3-letter, only 639-1 supported");
    }

    #[test]
    fn test_second_segment_detection() {
        // /docs/es-MX/about — es-MX is the second segment
        let (seg, v) = detect_locale_in_path("/docs/es-MX/about").unwrap();
        assert_eq!(seg, "es-MX");
        assert!(!v.separator_invalid);
        assert!(!v.case_invalid);
        assert!(!v.unknown);
    }

    #[test]
    fn test_digit_region_accepted() {
        // es-419 — 3-digit M49 region, accepted unvalidated
        let (seg, v) = detect_locale_in_path("/es-419/products").unwrap();
        assert_eq!(seg, "es-419");
        assert!(!v.separator_invalid);
        assert!(!v.case_invalid, "digits have no case");
        assert!(!v.unknown, "digit regions are accepted unvalidated");
    }

    #[test]
    fn test_locale_data_iso_639_1_sorted_and_deduped() {
        // Assert via windows(2) comparison — do NOT print contents
        let arr = crate::locale_data::ISO_639_1;
        for w in arr.windows(2) {
            assert!(
                w[0] < w[1],
                "ISO_639_1 must be sorted ascending and deduped: {} >= {}",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn test_locale_data_iso_3166_1_alpha2_sorted_and_deduped() {
        // Assert via windows(2) comparison — do NOT print contents
        let arr = crate::locale_data::ISO_3166_1_ALPHA2;
        for w in arr.windows(2) {
            assert!(
                w[0] < w[1],
                "ISO_3166_1_ALPHA2 must be sorted ascending and deduped: {} >= {}",
                w[0],
                w[1]
            );
        }
    }
}
