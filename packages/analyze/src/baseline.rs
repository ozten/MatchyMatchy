//! Baseline accept-list for suppressing known issues (spec §7.4, M8.md §3).
//!
//! Format: JSON array of `{ "id": "issue_...", "note"?: "..." }`.
//! Notes are advisory and ignored. Unknown/extra fields ignored (lenient).

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Context;

/// A set of issue ids to suppress.
#[derive(Debug, Clone, Default)]
pub struct Baseline {
    ids: BTreeSet<String>,
}

impl Baseline {
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Construct directly from ids (used in tests/CLI).
    pub fn from_ids<I: IntoIterator<Item = String>>(ids: I) -> Self {
        Self {
            ids: ids.into_iter().collect(),
        }
    }

    /// Iterate over all baseline ids in sorted order.
    pub fn iter_ids(&self) -> impl Iterator<Item = &String> {
        self.ids.iter()
    }
}

/// Parse the inner JSON bytes; factored out to allow unit-testing without fs.
pub fn parse(bytes: &[u8]) -> anyhow::Result<Baseline> {
    let entries: Vec<serde_json::Value> =
        serde_json::from_slice(bytes).context("failed to parse baseline JSON")?;
    let mut ids = BTreeSet::new();
    for entry in entries {
        if let Some(id_str) = entry.get("id").and_then(|v| v.as_str()) {
            ids.insert(id_str.to_string());
        }
        // Entries without a string "id" are silently skipped.
    }
    Ok(Baseline { ids })
}

/// Load a baseline accept-list from a JSON file on disk.
///
/// File must be a JSON array of `{ "id": "...", "note"?: "..." }`.
/// Missing-id entries are skipped (lenient). Parse/read errors are reported with context.
pub fn load(path: &Path) -> anyhow::Result<Baseline> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read baseline file: {}", path.display()))?;
    parse(&bytes).with_context(|| format!("failed to parse baseline file: {}", path.display()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_two_entries_one_with_note() {
        let json = r#"[
            {"id": "issue_aabbcc001122", "note": "intentional change"},
            {"id": "issue_112233445566"}
        ]"#;
        let baseline = parse(json.as_bytes()).expect("parse must succeed");
        assert_eq!(baseline.len(), 2);
        assert!(baseline.contains("issue_aabbcc001122"));
        assert!(baseline.contains("issue_112233445566"));
        assert!(!baseline.is_empty());
    }

    #[test]
    fn test_parse_empty_array() {
        let json = r#"[]"#;
        let baseline = parse(json.as_bytes()).expect("parse must succeed");
        assert!(baseline.is_empty());
        assert_eq!(baseline.len(), 0);
    }

    #[test]
    fn test_entry_missing_id_is_skipped() {
        let json = r#"[
            {"note": "no id here"},
            {"id": "issue_goodid000000"}
        ]"#;
        let baseline = parse(json.as_bytes()).expect("parse must succeed");
        assert_eq!(baseline.len(), 1, "entry missing id must be skipped");
        assert!(baseline.contains("issue_goodid000000"));
    }

    #[test]
    fn test_from_ids() {
        let ids = vec!["issue_aaa".to_string(), "issue_bbb".to_string()];
        let baseline = Baseline::from_ids(ids);
        assert_eq!(baseline.len(), 2);
        assert!(baseline.contains("issue_aaa"));
        assert!(baseline.contains("issue_bbb"));
    }

    #[test]
    fn test_invalid_json_returns_error() {
        let bad = b"not json";
        assert!(parse(bad).is_err());
    }

    #[test]
    fn test_id_not_string_is_skipped() {
        let json = r#"[{"id": 42}, {"id": "issue_valid000000"}]"#;
        let baseline = parse(json.as_bytes()).expect("parse must succeed");
        assert_eq!(baseline.len(), 1);
        assert!(baseline.contains("issue_valid000000"));
    }
}
