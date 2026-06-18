pub mod html;
pub mod json;
pub mod markdown;
pub mod outline;

use crate::contract::DiffResult;
use std::collections::BTreeSet;

/// Report disclosure mode: Compact (default — progressive-disclosure ToC) or
/// Full (legacy complete dump, opt-in via --full; byte-identical to pre-feature).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisclosureMode {
    Compact,
    Full,
}

impl DisclosureMode {
    pub fn from_full_flag(full: bool) -> Self {
        if full {
            Self::Full
        } else {
            Self::Compact
        }
    }
}

/// Returns true when the issue evidence indicates an uncertain element pairing —
/// the matcher could not confidently establish correspondence.
pub fn is_uncertain_pairing(evidence: &serde_json::Value) -> bool {
    evidence
        .get("match")
        .and_then(|m| m.get("uncertainPairing"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Display section key `(landmark, heading)` for grouping and drill-command generation.
/// - `landmark` is `None` → `"(page)"`.
/// - `nearest_heading` is `None` → `"\u{2014}"` (em dash).
///
/// Shared by markdown.rs, outline.rs, and html.rs. Any change to the defaults
/// here affects byte-identity of all three renderers.
pub fn section_key_of(issue: &crate::contract::Issue) -> (String, String) {
    let lm = issue
        .locator
        .anchors
        .landmark
        .as_deref()
        .unwrap_or("(page)")
        .to_string();
    let hd = issue
        .locator
        .anchors
        .nearest_heading
        .as_deref()
        .unwrap_or("\u{2014}") // em dash
        .to_string();
    (lm, hd)
}

/// The set of issue ids claimed by any saturated region (demoted into the region
/// rollup). Mirrors the construction in `json.rs` (`region_claimed_ids`). Used by the
/// markdown and HTML renderers to avoid double-reporting region members in the
/// per-issue listings — the region rollup is their single representation.
///
/// Deterministic: returns a `BTreeSet` (sorted, no iteration-order dependence).
pub fn claimed_issue_ids(result: &DiffResult) -> BTreeSet<&str> {
    result
        .regions
        .iter()
        .flat_map(|r| r.member_issue_ids.iter().map(|s| s.as_str()))
        .collect()
}
