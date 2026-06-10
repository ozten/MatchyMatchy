//! Region -> node anchor linking (M1.md §5.3).
//!
//! Candidates = new-page nodes whose bbox ∩ region >= 30% of node bbox area
//! AND distinctive anchor (non-empty text, else href/alt/ariaLabel).
//! Pick minimum seqIndex; tie-break node id (lexicographic, total order).
//! Fallback: old-page nodes. Else null anchors.

use crate::config::REGION_NODE_OVERLAP;
use crate::contract::{Anchors, SemanticNode};
use crate::visual_diff::Rect;

/// Result of linking a region to nodes.
pub struct LinkResult {
    pub anchors: Anchors,
    pub css_selector_old: Option<String>,
    pub css_selector_new: Option<String>,
    pub seq_index_old: Option<u32>,
    pub seq_index_new: Option<u32>,
}

/// Find the best-matching node for a region bbox from a node list.
/// Returns the node with minimum seqIndex among candidates; tie-break by node id.
fn find_candidate<'a>(nodes: &'a [SemanticNode], region: &Rect) -> Option<&'a SemanticNode> {
    // Collect all candidates in a sorted-by-seqIndex, then id order (total order).
    let mut candidates: Vec<&SemanticNode> = nodes
        .iter()
        .filter(|node| {
            node.has_distinctive_anchor() && bbox_overlap_ratio(node, region) >= REGION_NODE_OVERLAP
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }

    // Sort by (seqIndex, id) for total order — deterministic minimum selection.
    candidates.sort_by(|a, b| a.seq_index.cmp(&b.seq_index).then_with(|| a.id.cmp(&b.id)));
    candidates.into_iter().next()
}

/// Compute overlap ratio: area of (node_bbox ∩ region) / node_bbox_area.
fn bbox_overlap_ratio(node: &SemanticNode, region: &Rect) -> f64 {
    let [nx, ny, nw, nh] = node.bbox;
    if nw <= 0 || nh <= 0 {
        return 0.0;
    }
    let node_area = nw as f64 * nh as f64;

    // Intersection
    let rx1 = region.x as i32;
    let ry1 = region.y as i32;
    let rx2 = (region.x + region.w) as i32;
    let ry2 = (region.y + region.h) as i32;

    let nx2 = nx + nw;
    let ny2 = ny + nh;

    let ix1 = nx.max(rx1);
    let iy1 = ny.max(ry1);
    let ix2 = nx2.min(rx2);
    let iy2 = ny2.min(ry2);

    if ix2 <= ix1 || iy2 <= iy1 {
        return 0.0;
    }

    let inter_area = (ix2 - ix1) as f64 * (iy2 - iy1) as f64;
    inter_area / node_area
}

/// Link a region to the best matching node from new-page nodes, falling back to old-page.
pub fn link_region(
    region: &Rect,
    new_nodes: &[SemanticNode],
    old_nodes: &[SemanticNode],
) -> LinkResult {
    // Try new-page nodes first.
    if let Some(node) = find_candidate(new_nodes, region) {
        return LinkResult {
            anchors: node_to_anchors(node),
            css_selector_old: None,
            css_selector_new: node.css_selector.clone(),
            seq_index_old: None,
            seq_index_new: Some(node.seq_index),
        };
    }

    // Fallback: old-page nodes.
    if let Some(node) = find_candidate(old_nodes, region) {
        return LinkResult {
            anchors: node_to_anchors(node),
            css_selector_old: node.css_selector.clone(),
            css_selector_new: None,
            seq_index_old: Some(node.seq_index),
            seq_index_new: None,
        };
    }

    // No candidate found: null anchors.
    LinkResult {
        anchors: Anchors::null(),
        css_selector_old: None,
        css_selector_new: None,
        seq_index_old: None,
        seq_index_new: None,
    }
}

/// Convert a SemanticNode's anchors to the Issue Anchors struct.
fn node_to_anchors(node: &SemanticNode) -> Anchors {
    Anchors {
        text: node.anchors.text.clone(),
        role: node.anchors.role.clone(),
        href: node.anchors.href.clone(),
        alt: node.anchors.alt.clone(),
        aria_label: node.anchors.aria_label.clone(),
        nearest_heading: node.anchors.nearest_heading.clone(),
        landmark: node.anchors.landmark.clone(),
        ordinal_in_landmark: node.anchors.ordinal_in_landmark,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{NodeAnchors, SemanticNode};

    fn make_node(id: &str, seq_index: u32, bbox: [i32; 4], text: Option<&str>) -> SemanticNode {
        SemanticNode {
            id: id.to_string(),
            kind: "text".to_string(),
            role: None,
            text: text.map(str::to_string),
            acc_name: text.map(str::to_string),
            href: None,
            image_alt: None,
            bbox,
            seq_index,
            anchors: NodeAnchors {
                text: text.map(str::to_string),
                role: None,
                href: None,
                alt: None,
                aria_label: None,
                nearest_heading: None,
                landmark: None,
                ordinal_in_landmark: None,
            },
            css_selector: Some(format!("span#{}", id)),
            ..Default::default()
        }
    }

    #[test]
    fn test_link_picks_minimum_seqindex() {
        let region = Rect {
            x: 0,
            y: 0,
            w: 200,
            h: 100,
        };
        let nodes = vec![
            make_node("node_5", 5, [10, 10, 100, 50], Some("Later text")),
            make_node("node_1", 1, [20, 20, 80, 40], Some("First text")),
            make_node("node_3", 3, [30, 30, 60, 30], Some("Middle text")),
        ];
        let result = link_region(&region, &nodes, &[]);
        // Should pick node_1 (seqIndex=1, minimum)
        assert_eq!(result.anchors.text.as_deref(), Some("First text"));
        assert_eq!(result.seq_index_new, Some(1));
    }

    #[test]
    fn test_link_tie_break_by_id() {
        let region = Rect {
            x: 0,
            y: 0,
            w: 200,
            h: 100,
        };
        let nodes = vec![
            make_node("node_b", 2, [10, 10, 100, 50], Some("Text B")),
            make_node("node_a", 2, [20, 20, 80, 40], Some("Text A")),
        ];
        let result = link_region(&region, &nodes, &[]);
        // Both have seqIndex=2; tie-break by id: "node_a" < "node_b"
        assert_eq!(result.anchors.text.as_deref(), Some("Text A"));
    }

    #[test]
    fn test_link_fallback_to_old_nodes() {
        let region = Rect {
            x: 0,
            y: 0,
            w: 200,
            h: 100,
        };
        // New nodes don't overlap
        let new_nodes = vec![make_node(
            "node_1",
            1,
            [500, 500, 100, 100],
            Some("Far away"),
        )];
        let old_nodes = vec![make_node("node_2", 2, [10, 10, 100, 50], Some("Old text"))];
        let result = link_region(&region, &new_nodes, &old_nodes);
        assert_eq!(result.anchors.text.as_deref(), Some("Old text"));
        assert_eq!(result.seq_index_old, Some(2));
        assert_eq!(result.seq_index_new, None);
    }

    #[test]
    fn test_link_null_anchors_when_no_candidates() {
        let region = Rect {
            x: 0,
            y: 0,
            w: 50,
            h: 50,
        };
        // Nodes don't overlap with region
        let nodes = vec![make_node("node_1", 1, [500, 500, 100, 100], Some("Far"))];
        let result = link_region(&region, &nodes, &[]);
        assert!(result.anchors.text.is_none());
        assert!(result.seq_index_new.is_none());
    }

    #[test]
    fn test_link_node_without_distinctive_anchor_skipped() {
        let region = Rect {
            x: 0,
            y: 0,
            w: 200,
            h: 100,
        };
        // Node overlaps but has no distinctive anchor
        let mut node = make_node("node_1", 1, [10, 10, 100, 50], None);
        node.anchors.text = None;
        node.anchors.href = None;
        node.anchors.alt = None;
        node.anchors.aria_label = None;
        let result = link_region(&region, &[node], &[]);
        // Should get null anchors since no distinctive anchor
        assert!(result.anchors.text.is_none());
    }
}
