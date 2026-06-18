pub mod html;
pub mod json;
pub mod markdown;
pub mod outline;

use crate::contract::DiffResult;
use std::collections::BTreeSet;

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
