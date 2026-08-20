//! Sequence / order diff (M5.md §3–§4).
//!
//! Entry point: `sequence_issues(old_nodes, new_nodes, outcome, viewport, new_lang) -> Vec<Issue>`
//!
//! ## Eligible pairs (S0)
//!
//! Only pairs with `band == Matched` **and** `stage == Identity` enter the order model.
//! Assignment-stage pairs are excluded even when their band is Matched, because the
//! stage-2 matcher uses position (page-y, bbox area) as a tiebreak signal. Reading
//! seq-index displacement off a position-influenced pairing is circular evidence: among
//! duplicate-content nodes (carousels, repeated logos) the matcher cannot know which copy
//! went where, and any apparent displacement is noise by construction.  Spec §8's
//! premise — "the identity-first matcher guarantees moved components are still paired
//! regardless of position" — is a statement about stage-1 (identity) matches only;
//! those are the only ones that make displacement meaningful, position-independent evidence.
//!
//! ## Displacement threshold (S4)
//!
//! A `component_reordered` is emitted only when block displacement (in eligible-pair rank
//! units) **strictly exceeds** `SEQ_MIN_DISPLACEMENT` (spec §6.2: "exceeds a threshold").
//! Displacement == SEQ_MIN_DISPLACEMENT is suppressed as extraction jitter or a knock-on
//! shift from a nearby removal. Swaps (S3) are never thresholded.
//!
//! ## Determinism
//!
//! No HashMap/HashSet in output path; all sorts total-ordered with node-id tie-break;
//! confidence mean computed in ascending old_rank order; evidence maps built in fixed
//! key order.

use std::collections::BTreeMap;

use crate::config::SEQ_MIN_DISPLACEMENT;
use crate::contract::{
    Anchors, Issue, IssueCategory, IssueSeverity, IssueType, Locator, SemanticNode,
};
use crate::issue::compute_issue_id;
use crate::matching::{MatchBand, MatchOutcome, MatchStage};

// ---------------------------------------------------------------------------
// Public entry point (M5.md §2)
// ---------------------------------------------------------------------------

/// Derive sequence / order issues from the matcher outcome.
///
/// `new_lang` — the new page's `<html lang>` attribute value (stamped on every emitted
/// issue to match the convention used by all other emitters; M6.md §6).
///
/// Returns `component_swapped` and `component_reordered` issues in old-rank order.
pub fn sequence_issues(
    old_nodes: &[SemanticNode],
    new_nodes: &[SemanticNode],
    outcome: &MatchOutcome,
    viewport: &str,
    new_lang: Option<String>,
) -> Vec<Issue> {
    // S0 — eligible pairs: Matched band AND Identity stage only (M5.md §3 S0).
    // Assignment-stage pairs used position as a tiebreak; displacement derived from them
    // is circular evidence and is excluded from the order model entirely.
    let eligible: Vec<usize> = outcome
        .pairs
        .iter()
        .enumerate()
        .filter(|(_, p)| p.band == MatchBand::Matched && p.stage == MatchStage::Identity)
        .map(|(i, _)| i)
        .collect();

    let n = eligible.len();
    if n < 2 {
        return Vec::new();
    }

    // S1 — order model (M5.md §3 S1).
    // Sort eligible pairs by (old.seq_index, old.id) to assign old_rank.
    let mut old_rank_order: Vec<usize> = eligible.clone();
    old_rank_order.sort_by(|&a, &b| {
        let pa = &outcome.pairs[a];
        let pb = &outcome.pairs[b];
        let oa = &old_nodes[pa.old_idx];
        let ob = &old_nodes[pb.old_idx];
        oa.seq_index
            .cmp(&ob.seq_index)
            .then_with(|| oa.id.cmp(&ob.id))
    });

    // Sort eligible pairs by (new.seq_index, new.id) to assign new_rank.
    let mut new_rank_order: Vec<usize> = eligible.clone();
    new_rank_order.sort_by(|&a, &b| {
        let pa = &outcome.pairs[a];
        let pb = &outcome.pairs[b];
        let na = &new_nodes[pa.new_idx];
        let nb = &new_nodes[pb.new_idx];
        na.seq_index
            .cmp(&nb.seq_index)
            .then_with(|| na.id.cmp(&nb.id))
    });

    // Map from pair-index (in outcome.pairs) → new_rank.
    // We use BTreeMap to ensure determinism (no HashMap).
    let mut pair_to_new_rank: BTreeMap<usize, usize> = BTreeMap::new();
    for (new_rank, &pair_idx) in new_rank_order.iter().enumerate() {
        pair_to_new_rank.insert(pair_idx, new_rank);
    }

    // perm[old_rank] = new_rank.
    let perm: Vec<usize> = old_rank_order
        .iter()
        .map(|pair_idx| *pair_to_new_rank.get(pair_idx).unwrap())
        .collect();

    // S2 — block decomposition (M5.md §3 S2).
    // Group old_rank positions into maximal runs contiguous in both orders:
    // ranks r, r+1 belong to one block iff perm[r+1] == perm[r] + 1.
    let blocks: Vec<Block> = build_blocks(&old_rank_order, &perm, old_nodes, new_nodes, outcome);
    let m = blocks.len();
    if m < 2 {
        return Vec::new();
    }

    // sigma[i] = position of B_i when blocks are sorted by new-rank start.
    // (i.e., sigma[old_block_index] = new_block_rank)
    let sigma: Vec<usize> = {
        let mut indexed: Vec<(usize, usize)> = blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (i, b.new_rank_start))
            .collect();
        // Sort by new_rank_start, tie-break by old_rank_start for stability.
        indexed.sort_by_key(|&(i, nrs)| (nrs, blocks[i].old_rank_start));
        // sigma_inv[new_rank] = old_index; sigma[old_index] = new_rank.
        let mut sigma = vec![0usize; m];
        for (new_block_rank, (old_block_idx, _)) in indexed.iter().enumerate() {
            sigma[*old_block_idx] = new_block_rank;
        }
        sigma
    };

    // Identity permutation → no issues.
    if sigma.iter().enumerate().all(|(i, &s)| s == i) {
        return Vec::new();
    }

    // S3 — swap detection (before LIS) (M5.md §3 S3).
    let mut consumed = vec![false; m];
    let mut issues: Vec<Issue> = Vec::new();

    // Detect transpositions: pairs (i, j) with i < j, sigma[i] == j && sigma[j] == i.
    // Iterate in old order to get deterministic emission.
    for i in 0..m {
        if consumed[i] {
            continue;
        }
        let j = sigma[i];
        if j > i && j < m && sigma[j] == i && !consumed[j] {
            // Exchange: blocks[i] and blocks[j] are swapped.
            consumed[i] = true;
            consumed[j] = true;
            if let Some(issue) = build_swap_issue(
                &blocks[i],
                &blocks[j],
                old_nodes,
                new_nodes,
                outcome,
                viewport,
                new_lang.clone(),
            ) {
                issues.push(issue);
            }
        }
    }

    // S4 — reorders over non-consumed blocks (M5.md §3 S4).
    // Collect non-consumed block indices in old order, extract their sigma values.
    let non_consumed: Vec<usize> = (0..m).filter(|&i| !consumed[i]).collect();
    if non_consumed.len() >= 2 {
        let sigma_sub: Vec<usize> = non_consumed.iter().map(|&i| sigma[i]).collect();
        // Compute LIS using patience algorithm with predecessor links.
        let lis_set = compute_lis_set(&sigma_sub);

        for (sub_idx, &block_idx) in non_consumed.iter().enumerate() {
            if lis_set.contains(&sub_idx) {
                continue; // Stable — inside LIS
            }
            let block = &blocks[block_idx];
            // Threshold: displacement in rank units.
            let old_rs = block.old_rank_start as u32;
            let new_rs = block.new_rank_start as u32;
            let displacement = old_rs.abs_diff(new_rs);
            // Spec §6.2 "exceeds a threshold" → strictly greater (displacement == threshold
            // is suppressed as extraction jitter or a knock-on shift from a nearby removal).
            if displacement <= SEQ_MIN_DISPLACEMENT {
                continue; // Suppressed
            }
            // Neighbors for remediation: nearest stable old-order neighbors.
            // "Stable" = the neighbor's sub_idx is in the LIS set.
            let before_block_idx: Option<usize> = (0..sub_idx)
                .rev()
                .find(|&si| lis_set.contains(&si))
                .map(|si| non_consumed[si]);

            let after_block_idx: Option<usize> = (sub_idx + 1..non_consumed.len())
                .find(|&si| lis_set.contains(&si))
                .map(|si| non_consumed[si]);

            if let Some(issue) = build_reorder_issue(
                block,
                before_block_idx.map(|bi| &blocks[bi]),
                after_block_idx.map(|bi| &blocks[bi]),
                n,
                old_nodes,
                new_nodes,
                outcome,
                viewport,
                new_lang.clone(),
            ) {
                issues.push(issue);
            }
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// Block type
// ---------------------------------------------------------------------------

/// A component-grained block of consecutive matched pairs.
#[derive(Debug, Clone)]
struct Block {
    /// Start of the contiguous old-rank range for this block.
    old_rank_start: usize,
    new_rank_start: usize,
    /// Pair indices from outcome.pairs for each member (in old_rank order).
    pair_indices: Vec<usize>,
}

impl Block {
    fn len(&self) -> usize {
        self.pair_indices.len()
    }
}

// ---------------------------------------------------------------------------
// S2 implementation
// ---------------------------------------------------------------------------

fn build_blocks(
    old_rank_order: &[usize],
    perm: &[usize],
    old_nodes: &[SemanticNode],
    new_nodes: &[SemanticNode],
    outcome: &MatchOutcome,
) -> Vec<Block> {
    let _ = (old_nodes, new_nodes, outcome); // kept for future use
    let n = old_rank_order.len();
    if n == 0 {
        return Vec::new();
    }

    let mut blocks = Vec::new();
    let mut start = 0usize;

    for i in 1..=n {
        let break_here = i == n || perm[i] != perm[i - 1] + 1;
        if break_here {
            let block_perm_start = perm[start];
            blocks.push(Block {
                old_rank_start: start,
                new_rank_start: block_perm_start,
                pair_indices: old_rank_order[start..i].to_vec(),
            });
            start = i;
        }
    }

    blocks
}

// ---------------------------------------------------------------------------
// Block representative
// ---------------------------------------------------------------------------

/// Return the representative (first heading, else first member) old node for a block.
///
/// "First" = lowest old seq_index within the block.
fn block_rep_old<'a>(
    block: &Block,
    old_nodes: &'a [SemanticNode],
    outcome: &MatchOutcome,
) -> &'a SemanticNode {
    // Members are already in old_rank (ascending seq_index) order.
    // Find first heading.
    for &pair_idx in &block.pair_indices {
        let node = &old_nodes[outcome.pairs[pair_idx].old_idx];
        if node.kind == "heading" {
            return node;
        }
    }
    // Fall back to first member.
    let first_pair_idx = block.pair_indices[0];
    &old_nodes[outcome.pairs[first_pair_idx].old_idx]
}

/// Return the representative new node for a block (same pair as the old rep).
fn block_rep_new<'a>(
    block: &Block,
    old_nodes: &[SemanticNode],
    new_nodes: &'a [SemanticNode],
    outcome: &MatchOutcome,
) -> &'a SemanticNode {
    // Find the same pair index as block_rep_old.
    let rep_pair_idx = {
        let mut rep_idx = block.pair_indices[0];
        for &pair_idx in &block.pair_indices {
            let node = &old_nodes[outcome.pairs[pair_idx].old_idx];
            if node.kind == "heading" {
                rep_idx = pair_idx;
                break;
            }
        }
        rep_idx
    };
    &new_nodes[outcome.pairs[rep_pair_idx].new_idx]
}

// ---------------------------------------------------------------------------
// Confidence aggregation
// ---------------------------------------------------------------------------

/// Compute mean score over a set of pair indices, in ascending old_rank order.
///
/// "Ascending old_rank" = the order they appear in pair_indices (already old-rank ordered).
fn mean_score_in_old_rank_order(pair_indices: &[usize], outcome: &MatchOutcome) -> f64 {
    if pair_indices.is_empty() {
        return 0.0;
    }
    let sum: f64 = pair_indices.iter().map(|&pi| outcome.pairs[pi].score).sum();
    round4(sum / pair_indices.len() as f64)
}

fn round4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}

// ---------------------------------------------------------------------------
// Bbox union
// ---------------------------------------------------------------------------

/// Compute component-wise min/max bbox union over a block's nodes on the old side.
fn bbox_union_old(block: &Block, old_nodes: &[SemanticNode], outcome: &MatchOutcome) -> [i32; 4] {
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for &pair_idx in &block.pair_indices {
        let node = &old_nodes[outcome.pairs[pair_idx].old_idx];
        let [x, y, w, h] = node.bbox;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x + w);
        max_y = max_y.max(y + h);
    }
    [min_x, min_y, max_x - min_x, max_y - min_y]
}

/// Compute component-wise min/max bbox union over a block's nodes on the new side.
fn bbox_union_new(block: &Block, new_nodes: &[SemanticNode], outcome: &MatchOutcome) -> [i32; 4] {
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for &pair_idx in &block.pair_indices {
        let node = &new_nodes[outcome.pairs[pair_idx].new_idx];
        let [x, y, w, h] = node.bbox;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x + w);
        max_y = max_y.max(y + h);
    }
    [min_x, min_y, max_x - min_x, max_y - min_y]
}

// ---------------------------------------------------------------------------
// seq_index range helpers
// ---------------------------------------------------------------------------

fn seq_index_range_old(
    block: &Block,
    old_nodes: &[SemanticNode],
    outcome: &MatchOutcome,
) -> [u32; 2] {
    let mut min_si = u32::MAX;
    let mut max_si = 0u32;
    for &pair_idx in &block.pair_indices {
        let si = old_nodes[outcome.pairs[pair_idx].old_idx].seq_index;
        min_si = min_si.min(si);
        max_si = max_si.max(si);
    }
    [min_si, max_si]
}

fn seq_index_range_new(
    block: &Block,
    new_nodes: &[SemanticNode],
    outcome: &MatchOutcome,
) -> [u32; 2] {
    let mut min_si = u32::MAX;
    let mut max_si = 0u32;
    for &pair_idx in &block.pair_indices {
        let si = new_nodes[outcome.pairs[pair_idx].new_idx].seq_index;
        min_si = min_si.min(si);
        max_si = max_si.max(si);
    }
    [min_si, max_si]
}

// ---------------------------------------------------------------------------
// Anchor text truncated to 80 bytes, respecting UTF-8 char boundaries
// ---------------------------------------------------------------------------

/// Truncate `raw` to at most 80 bytes, stepping back to the nearest char boundary.
///
/// Deterministic: always trims to the longest prefix whose byte length ≤ 80.
fn truncate80(raw: &str) -> String {
    if raw.len() <= 80 {
        return raw.to_string();
    }
    let mut end = 80;
    while !raw.is_char_boundary(end) {
        end -= 1;
    }
    raw[..end].to_string()
}

fn rep_text(node: &SemanticNode) -> String {
    let raw = node
        .anchors
        .text
        .as_deref()
        .or(node.anchors.href.as_deref())
        .or(node.anchors.alt.as_deref())
        .or(node.text.as_deref())
        .unwrap_or("");
    truncate80(raw)
}

fn rep_text_or_href(node: &SemanticNode) -> String {
    let raw = node
        .anchors
        .text
        .as_deref()
        .filter(|s| !s.is_empty())
        .or_else(|| node.anchors.href.as_deref().filter(|s| !s.is_empty()))
        .or_else(|| node.text.as_deref().filter(|s| !s.is_empty()))
        .or_else(|| node.anchors.alt.as_deref().filter(|s| !s.is_empty()))
        .unwrap_or("");
    truncate80(raw)
}

// ---------------------------------------------------------------------------
// Anchors conversion from SemanticNode
// ---------------------------------------------------------------------------

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
// Block descriptor for evidence
// ---------------------------------------------------------------------------

fn block_descriptor(
    block: &Block,
    old_nodes: &[SemanticNode],
    new_nodes: &[SemanticNode],
    outcome: &MatchOutcome,
) -> serde_json::Value {
    let rep_old = block_rep_old(block, old_nodes, outcome);
    let rep_pair_idx = {
        let mut idx = block.pair_indices[0];
        for &pi in &block.pair_indices {
            if old_nodes[outcome.pairs[pi].old_idx].kind == "heading" {
                idx = pi;
                break;
            }
        }
        idx
    };
    let pair = &outcome.pairs[rep_pair_idx];
    let bbox_old = bbox_union_old(block, old_nodes, outcome);
    let bbox_new = bbox_union_new(block, new_nodes, outcome);
    let si_old = seq_index_range_old(block, old_nodes, outcome);
    let si_new = seq_index_range_new(block, new_nodes, outcome);

    let anchors_val = serde_json::json!({
        "text": rep_old.anchors.text,
        "role": rep_old.anchors.role,
        "href": rep_old.anchors.href,
        "alt": rep_old.anchors.alt,
        "ariaLabel": rep_old.anchors.aria_label,
        "nearestHeading": rep_old.anchors.nearest_heading,
        "landmark": rep_old.anchors.landmark,
        "ordinalInLandmark": rep_old.anchors.ordinal_in_landmark,
    });

    let stage_str = match pair.stage {
        MatchStage::Identity => "identity",
        MatchStage::Assignment => "assignment",
    };

    // Build signals in sorted key order (BTreeMap iteration).
    let mut signals_map = serde_json::Map::new();
    for (k, v) in &pair.signals {
        signals_map.insert(k.clone(), serde_json::Value::from(*v));
    }

    serde_json::json!({
        "anchors": anchors_val,
        "bboxOld": bbox_old,
        "bboxNew": bbox_new,
        "seqIndexOldRange": [si_old[0], si_old[1]],
        "seqIndexNewRange": [si_new[0], si_new[1]],
        "nodeCount": block.len(),
        "match": {
            "stage": stage_str,
            "score": round4(pair.score),
            "signals": serde_json::Value::Object(signals_map),
        }
    })
}

// ---------------------------------------------------------------------------
// S3: component_swapped issue construction
// ---------------------------------------------------------------------------

fn build_swap_issue(
    block_a: &Block,
    block_b: &Block,
    old_nodes: &[SemanticNode],
    new_nodes: &[SemanticNode],
    outcome: &MatchOutcome,
    viewport: &str,
    new_lang: Option<String>,
) -> Option<Issue> {
    let rep_a_old = block_rep_old(block_a, old_nodes, outcome);
    let rep_b_old = block_rep_old(block_b, old_nodes, outcome);
    let rep_a_new = block_rep_new(block_a, old_nodes, new_nodes, outcome);
    let _rep_b_new = block_rep_new(block_b, old_nodes, new_nodes, outcome);

    let text_a = rep_text(rep_a_old);
    let text_b = rep_text(rep_b_old);

    // Confidence: mean over A∪B in ascending old_rank order.
    let mut all_pairs: Vec<usize> = block_a
        .pair_indices
        .iter()
        .chain(block_b.pair_indices.iter())
        .copied()
        .collect();
    // Sort by old node seq_index, tie-break by old node id.
    all_pairs.sort_by(|&a, &b| {
        let oa = &old_nodes[outcome.pairs[a].old_idx];
        let ob = &old_nodes[outcome.pairs[b].old_idx];
        oa.seq_index
            .cmp(&ob.seq_index)
            .then_with(|| oa.id.cmp(&ob.id))
    });
    let confidence = mean_score_in_old_rank_order(&all_pairs, outcome);

    let anchors_a = node_to_anchors(rep_a_old);
    let id = compute_issue_id(&IssueType::ComponentSwapped, viewport, &anchors_a, None);

    let message = format!(
        "Components swapped: \"{}\" and \"{}\" exchanged document order; \"{}\" now renders before \"{}\".",
        text_a, text_b, text_b, text_a
    );

    // Locator: primary = block A rep.
    let bbox_a_old = bbox_union_old(block_a, old_nodes, outcome);
    let bbox_a_new = bbox_union_new(block_a, new_nodes, outcome);
    let locator = Locator {
        anchors: anchors_a.clone(),
        css_selector_old: rep_a_old.css_selector.clone(),
        css_selector_new: rep_a_new.css_selector.clone(),
        bbox_old: Some(bbox_a_old),
        bbox_new: Some(bbox_a_new),
        seq_index_old: Some(rep_a_old.seq_index),
        seq_index_new: Some(rep_a_new.seq_index),
    };

    let block_a_desc = block_descriptor(block_a, old_nodes, new_nodes, outcome);
    let block_b_desc = block_descriptor(block_b, old_nodes, new_nodes, outcome);

    let evidence = serde_json::json!({
        "blockA": block_a_desc,
        "blockB": block_b_desc,
        "oldOrder": [text_a, text_b],
        "newOrder": [text_b, text_a],
    });

    let near_a = rep_a_old
        .anchors
        .nearest_heading
        .as_deref()
        .unwrap_or(&text_a);
    let remediation = serde_json::json!({
        "action": "reorder_components",
        "findBy": {
            "grep": [rep_text_or_href(rep_a_old), rep_text_or_href(rep_b_old)],
            "near": near_a,
        },
        "expectedOrder": [text_a, text_b],
        "note": format!(
            "Restore original document order: \"{}\" must precede \"{}\". Grep targets may hit repo source or CMS content; the anchors identify the sections either way.",
            text_a, text_b
        ),
    });

    Some(Issue {
        id,
        issue_type: IssueType::ComponentSwapped,
        category: IssueCategory::Structure,
        severity: IssueSeverity::Error,
        confidence,
        viewport: viewport.to_string(),
        locale: new_lang,
        goal: Some("G3".to_string()),
        message,
        locator,
        evidence,
        remediation: Some(remediation),
    })
}

// ---------------------------------------------------------------------------
// S4: component_reordered issue construction
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn build_reorder_issue(
    block: &Block,
    before_block: Option<&Block>,
    after_block: Option<&Block>,
    n_matched: usize,
    old_nodes: &[SemanticNode],
    new_nodes: &[SemanticNode],
    outcome: &MatchOutcome,
    viewport: &str,
    new_lang: Option<String>,
) -> Option<Issue> {
    let rep_old = block_rep_old(block, old_nodes, outcome);
    let rep_new = block_rep_new(block, old_nodes, new_nodes, outcome);

    let text_rep = rep_text(rep_old);
    let confidence = mean_score_in_old_rank_order(&block.pair_indices, outcome);

    let anchors = node_to_anchors(rep_old);
    let id = compute_issue_id(&IssueType::ComponentReordered, viewport, &anchors, None);

    let message = format!(
        "Component moved: \"{}\" moved from document position {} to {} of {} matched components.",
        text_rep, block.old_rank_start, block.new_rank_start, n_matched
    );

    let bbox_old = bbox_union_old(block, old_nodes, outcome);
    let bbox_new = bbox_union_new(block, new_nodes, outcome);
    let locator = Locator {
        anchors: anchors.clone(),
        css_selector_old: rep_old.css_selector.clone(),
        css_selector_new: rep_new.css_selector.clone(),
        bbox_old: Some(bbox_old),
        bbox_new: Some(bbox_new),
        seq_index_old: Some(rep_old.seq_index),
        seq_index_new: Some(rep_new.seq_index),
    };

    let displacement = (block.old_rank_start as i64 - block.new_rank_start as i64).unsigned_abs();

    let block_desc = block_descriptor(block, old_nodes, new_nodes, outcome);

    // movedBefore / movedAfter: anchor text of old-order stable neighbors.
    let moved_before = after_block.map(|b| rep_text(block_rep_old(b, old_nodes, outcome)));
    let moved_after = before_block.map(|b| rep_text(block_rep_old(b, old_nodes, outcome)));

    let mut evidence_map = serde_json::Map::new();
    evidence_map.insert("block".to_string(), block_desc);
    evidence_map.insert(
        "displacement".to_string(),
        serde_json::Value::from(displacement),
    );
    if let Some(ref mb) = moved_before {
        evidence_map.insert(
            "movedBefore".to_string(),
            serde_json::Value::from(mb.clone()),
        );
    }
    if let Some(ref ma) = moved_after {
        evidence_map.insert(
            "movedAfter".to_string(),
            serde_json::Value::from(ma.clone()),
        );
    }
    let evidence = serde_json::Value::Object(evidence_map);

    // remediation expectedOrder: [after, target, before] filtered for edges.
    let mut expected_order: Vec<String> = Vec::new();
    if let Some(ref ma) = moved_after {
        expected_order.push(ma.clone());
    }
    expected_order.push(text_rep.clone());
    if let Some(ref mb) = moved_before {
        expected_order.push(mb.clone());
    }

    let mut find_by_map = serde_json::Map::new();
    find_by_map.insert(
        "grep".to_string(),
        serde_json::json!([rep_text_or_href(rep_old)]),
    );
    find_by_map.insert(
        "near".to_string(),
        serde_json::Value::from(
            rep_old
                .anchors
                .nearest_heading
                .as_deref()
                .unwrap_or(&text_rep)
                .to_string(),
        ),
    );

    let mut remediation_map = serde_json::Map::new();
    remediation_map.insert(
        "action".to_string(),
        serde_json::Value::from("reorder_components"),
    );
    remediation_map.insert("findBy".to_string(), serde_json::Value::Object(find_by_map));
    remediation_map.insert(
        "target".to_string(),
        serde_json::Value::from(text_rep.clone()),
    );
    if let Some(ref ma) = moved_after {
        remediation_map.insert("after".to_string(), serde_json::Value::from(ma.clone()));
    }
    if let Some(ref mb) = moved_before {
        remediation_map.insert("before".to_string(), serde_json::Value::from(mb.clone()));
    }
    remediation_map.insert(
        "expectedOrder".to_string(),
        serde_json::Value::Array(
            expected_order
                .into_iter()
                .map(serde_json::Value::from)
                .collect(),
        ),
    );

    Some(Issue {
        id,
        issue_type: IssueType::ComponentReordered,
        category: IssueCategory::Structure,
        severity: IssueSeverity::Error,
        confidence,
        viewport: viewport.to_string(),
        locale: new_lang,
        goal: Some("G3".to_string()),
        message,
        locator,
        evidence,
        remediation: Some(serde_json::Value::Object(remediation_map)),
    })
}

// ---------------------------------------------------------------------------
// LIS: patience algorithm with predecessor links (M5.md §3 S4)
// ---------------------------------------------------------------------------

/// Compute the set of *indices* (into `values`) that form the LIS.
///
/// Patience algorithm: iterate ascending, lower-bound replacement.
/// Among equal-length candidates, this yields one deterministic chain.
/// Returns a `BTreeSet` of indices in `values` that belong to the LIS.
fn compute_lis_set(values: &[usize]) -> std::collections::BTreeSet<usize> {
    let n = values.len();
    if n == 0 {
        return std::collections::BTreeSet::new();
    }

    // tails[i] = index into values of the smallest tail element for IS of length i+1.
    let mut tails: Vec<usize> = Vec::new();
    // pred[i] = predecessor index for values[i] in the chain.
    let mut pred: Vec<Option<usize>> = vec![None; n];

    for i in 0..n {
        let v = values[i];
        // Binary search for leftmost position where tails[pos] value >= v.
        let pos = tails.partition_point(|&t| values[t] < v);
        if pos == tails.len() {
            tails.push(i);
        } else {
            tails[pos] = i;
        }
        if pos > 0 {
            pred[i] = Some(tails[pos - 1]);
        }
    }

    // Reconstruct the LIS by following predecessor links from the last tail.
    let mut lis_indices = std::collections::BTreeSet::new();
    let mut current = *tails.last().unwrap();
    lis_indices.insert(current);
    while let Some(p) = pred[current] {
        lis_indices.insert(p);
        current = p;
    }

    lis_indices
}

// ---------------------------------------------------------------------------
// Unit tests (M5.md §6.4)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{NodeAnchors, SemanticNode};
    use crate::matching::{MatchBand, MatchOutcome, MatchStage, MatchedPair};
    use std::collections::BTreeMap;

    // ---- Node builder (mirrors matching.rs test helper) ----

    fn make_node(id: &str, kind: &str, text: Option<&str>, seq_index: u32) -> SemanticNode {
        SemanticNode {
            id: id.to_string(),
            kind: kind.to_string(),
            role: None,
            text: text.map(|s| s.to_string()),
            acc_name: None,
            href: None,
            image_alt: None,
            bbox: [0, seq_index as i32 * 100, 500, 80],
            seq_index,
            anchors: NodeAnchors {
                text: text.map(|s| s.to_string()),
                role: None,
                href: None,
                alt: None,
                aria_label: None,
                nearest_heading: text.map(|s| s.to_string()),
                landmark: None,
                ordinal_in_landmark: None,
            },
            css_selector: None,
            raw_href: None,
            src: None,
            natural_width: None,
            natural_height: None,
            loaded: None,
            heading_level: if kind == "heading" { Some(2) } else { None },
            has_onclick: None,
        }
    }

    fn make_pair(old_idx: usize, new_idx: usize, score: f64, band: MatchBand) -> MatchedPair {
        MatchedPair {
            old_idx,
            new_idx,
            score,
            stage: MatchStage::Identity,
            band,
            signals: BTreeMap::new(),
        }
    }

    fn make_outcome(pairs: Vec<MatchedPair>) -> MatchOutcome {
        MatchOutcome {
            pairs,
            missing_old: Vec::new(),
            added_new: Vec::new(),
        }
    }

    // ---- Tests ----

    /// identity permutation → no issues
    #[test]
    fn test_identity_permutation_no_issues() {
        // 3 nodes, all in same order old=new.
        let old_nodes = vec![
            make_node("a", "heading", Some("Section A"), 0),
            make_node("b", "text", Some("Paragraph A"), 1),
            make_node("c", "heading", Some("Section B"), 2),
        ];
        let new_nodes = old_nodes.clone();
        let pairs = vec![
            make_pair(0, 0, 0.95, MatchBand::Matched),
            make_pair(1, 1, 0.95, MatchBand::Matched),
            make_pair(2, 2, 0.95, MatchBand::Matched),
        ];
        let outcome = make_outcome(pairs);
        let issues = sequence_issues(&old_nodes, &new_nodes, &outcome, "desktop", None);
        assert!(issues.is_empty(), "identity should produce no issues");
    }

    /// adjacent two-block swap → single component_swapped, zero reorders (synthetic v07)
    #[test]
    fn test_adjacent_swap_single_component_swapped() {
        // old: [A-heading, A-text, B-heading, B-text]
        // new: [B-heading, B-text, A-heading, A-text]
        let old_nodes = vec![
            make_node("a1", "heading", Some("Section A"), 0),
            make_node("a2", "text", Some("Content A"), 1),
            make_node("b1", "heading", Some("Section B"), 2),
            make_node("b2", "text", Some("Content B"), 3),
        ];
        let new_nodes = vec![
            make_node("b1", "heading", Some("Section B"), 0),
            make_node("b2", "text", Some("Content B"), 1),
            make_node("a1", "heading", Some("Section A"), 2),
            make_node("a2", "text", Some("Content A"), 3),
        ];
        // Pairs: old[0]→new[2], old[1]→new[3], old[2]→new[0], old[3]→new[1]
        let pairs = vec![
            make_pair(0, 2, 0.97, MatchBand::Matched),
            make_pair(1, 3, 0.97, MatchBand::Matched),
            make_pair(2, 0, 0.97, MatchBand::Matched),
            make_pair(3, 1, 0.97, MatchBand::Matched),
        ];
        let outcome = make_outcome(pairs);
        let issues = sequence_issues(&old_nodes, &new_nodes, &outcome, "desktop", None);
        assert_eq!(issues.len(), 1, "exactly one issue");
        assert_eq!(issues[0].issue_type, IssueType::ComponentSwapped);
        // Verify no reorders
        for issue in &issues {
            assert_ne!(issue.issue_type, IssueType::ComponentReordered);
        }
    }

    /// far-apart swap with stable middle (σ=[2,1,0]) → single component_swapped
    #[test]
    fn test_far_apart_swap_stable_middle() {
        // old: [A, M, B] (each a single heading node)
        // new: [B, M, A] — A and B swapped, M stays
        // sigma = [2, 1, 0] — transposition (0,2)
        let old_nodes = vec![
            make_node("a", "heading", Some("Block A"), 0),
            make_node("m", "heading", Some("Middle"), 1),
            make_node("b", "heading", Some("Block B"), 2),
        ];
        let new_nodes = vec![
            make_node("b", "heading", Some("Block B"), 0),
            make_node("m", "heading", Some("Middle"), 1),
            make_node("a", "heading", Some("Block A"), 2),
        ];
        let pairs = vec![
            make_pair(0, 2, 0.95, MatchBand::Matched), // A → new[2]
            make_pair(1, 1, 0.95, MatchBand::Matched), // M → new[1]
            make_pair(2, 0, 0.95, MatchBand::Matched), // B → new[0]
        ];
        let outcome = make_outcome(pairs);
        let issues = sequence_issues(&old_nodes, &new_nodes, &outcome, "desktop", None);
        let swap_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::ComponentSwapped)
            .collect();
        let reorder_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::ComponentReordered)
            .collect();
        assert_eq!(swap_issues.len(), 1, "exactly one swap");
        assert_eq!(reorder_issues.len(), 0, "no reorders");
    }

    /// rotation [X,P,Q]→[P,Q,X] → single component_swapped (S3 amendment, M5.md §3)
    ///
    /// S2 collapses P and Q into one block (they stay consecutive in both orders), so
    /// S3 sees a 2-block transposition (X↔PQ) and emits one `component_swapped`.
    #[test]
    fn test_rotation_xpq_is_swap() {
        // old: [X(0), P(1), Q(2)]; new: [P(0), Q(1), X(2)]
        // perm = [2, 0, 1]: perm[2]=1=perm[1]+1 → P and Q collapse into one block.
        // Blocks: {X}(new_rank=2), {PQ}(new_rank=0). sigma=[1,0] → transposition → one swap.
        let old_nodes = vec![
            make_node("x", "heading", Some("Block X"), 0),
            make_node("p", "heading", Some("Block P"), 1),
            make_node("q", "text", Some("Block Q"), 2),
        ];
        let new_nodes = vec![
            make_node("p", "heading", Some("Block P"), 0),
            make_node("q", "text", Some("Block Q"), 1),
            make_node("x", "heading", Some("Block X"), 2),
        ];
        let pairs = vec![
            make_pair(0, 2, 0.95, MatchBand::Matched), // X → new[2]
            make_pair(1, 0, 0.95, MatchBand::Matched), // P → new[0]
            make_pair(2, 1, 0.95, MatchBand::Matched), // Q → new[1]
        ];
        let outcome = make_outcome(pairs);
        let issues = sequence_issues(&old_nodes, &new_nodes, &outcome, "desktop", None);
        let swap_count = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::ComponentSwapped)
            .count();
        let reorder_count = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::ComponentReordered)
            .count();
        assert_eq!(swap_count, 1, "one swap (X↔PQ block)");
        assert_eq!(reorder_count, 0, "no reorders");
    }

    /// multi-block move with no block-level transposition → component_reordered, no swap
    ///
    /// Permutation (at node level, no block collapse): perm=[2,4,1,0,3]
    ///   old: [A(0), B(1), C(2), D(3), E(4)]
    ///   new: [D(0), C(1), A(2), E(3), B(4)]
    ///   sigma (all single blocks): [2,4,1,0,3]
    ///   No transpositions. LIS = {D(pos3), E(pos4)}.
    ///   Non-LIS with strict > threshold:
    ///     A: disp=|0-2|=2, NOT > SEQ_MIN_DISPLACEMENT(2) → suppressed.
    ///     B: disp=|1-4|=3, > 2 → emitted.
    ///     C: disp=|2-1|=1, ≤ 2 → suppressed.
    ///   Result: 1 reorder (B), 0 swaps.
    #[test]
    fn test_rotation_one_reorder_no_swap() {
        // perm=[2,4,1,0,3]: no consecutive adjacent pairs → all single-node blocks.
        let old_nodes = vec![
            make_node("a", "heading", Some("Block A"), 0),
            make_node("b", "heading", Some("Block B"), 1),
            make_node("c", "heading", Some("Block C"), 2),
            make_node("d", "heading", Some("Block D"), 3),
            make_node("e", "heading", Some("Block E"), 4),
        ];
        // new array: D(seq=0), C(seq=1), A(seq=2), E(seq=3), B(seq=4)
        let new_nodes = vec![
            make_node("d", "heading", Some("Block D"), 0),
            make_node("c", "heading", Some("Block C"), 1),
            make_node("a", "heading", Some("Block A"), 2),
            make_node("e", "heading", Some("Block E"), 3),
            make_node("b", "heading", Some("Block B"), 4),
        ];
        // pairs: A→new[2], B→new[4], C→new[1], D→new[0], E→new[3]
        let pairs = vec![
            make_pair(0, 2, 0.95, MatchBand::Matched), // A → new[2]
            make_pair(1, 4, 0.95, MatchBand::Matched), // B → new[4]
            make_pair(2, 1, 0.95, MatchBand::Matched), // C → new[1]
            make_pair(3, 0, 0.95, MatchBand::Matched), // D → new[0]
            make_pair(4, 3, 0.95, MatchBand::Matched), // E → new[3]
        ];
        let outcome = make_outcome(pairs);
        let issues = sequence_issues(&old_nodes, &new_nodes, &outcome, "desktop", None);
        let swap_count = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::ComponentSwapped)
            .count();
        let reorder_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::ComponentReordered)
            .collect();
        assert_eq!(swap_count, 0, "no swaps");
        // B (disp=3 > SEQ_MIN_DISPLACEMENT=2) must emit.
        let has_b = reorder_issues.iter().any(|i| i.message.contains("Block B"));
        assert!(
            has_b,
            "Block B (displacement=3 > threshold) must be emitted"
        );
        // A (disp=2 == SEQ_MIN_DISPLACEMENT) must NOT emit (strict > threshold).
        let has_a = reorder_issues.iter().any(|i| i.message.contains("Block A"));
        assert!(
            !has_a,
            "Block A (displacement=2 == threshold) must be suppressed by strict > rule"
        );
    }

    /// two independent exchanges → two component_swapped
    #[test]
    fn test_two_independent_swaps() {
        // old: [A, B, C, D]
        // new: [B, A, D, C]
        // sigma = [1, 0, 3, 2] → two transpositions: (0,1) and (2,3)
        let old_nodes = vec![
            make_node("a", "heading", Some("Block A"), 0),
            make_node("b", "heading", Some("Block B"), 1),
            make_node("c", "heading", Some("Block C"), 2),
            make_node("d", "heading", Some("Block D"), 3),
        ];
        let new_nodes = vec![
            make_node("b", "heading", Some("Block B"), 0),
            make_node("a", "heading", Some("Block A"), 1),
            make_node("d", "heading", Some("Block D"), 2),
            make_node("c", "heading", Some("Block C"), 3),
        ];
        let pairs = vec![
            make_pair(0, 1, 0.95, MatchBand::Matched), // A → new[1]
            make_pair(1, 0, 0.95, MatchBand::Matched), // B → new[0]
            make_pair(2, 3, 0.95, MatchBand::Matched), // C → new[3]
            make_pair(3, 2, 0.95, MatchBand::Matched), // D → new[2]
        ];
        let outcome = make_outcome(pairs);
        let issues = sequence_issues(&old_nodes, &new_nodes, &outcome, "desktop", None);
        let swap_count = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::ComponentSwapped)
            .count();
        assert_eq!(swap_count, 2, "two swaps");
        let reorder_count = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::ComponentReordered)
            .count();
        assert_eq!(reorder_count, 0, "no reorders");
    }

    /// ABCD→CDAB → ONE swap (block decomposition collapses it)
    #[test]
    fn test_abcd_cdab_one_swap_block_collapse() {
        // old: [A, B, C, D] (each a single node, all paired)
        // new: [C, D, A, B]
        // After block decomposition:
        //   old_rank_order: 0→A, 1→B, 2→C, 3→D
        //   perm = [2, 3, 0, 1]
        //   Block 0: old_rank 0-1 (A,B), new_rank_start=2
        //   Block 1: old_rank 2-3 (C,D), new_rank_start=0
        //   sigma = [1, 0] → one transposition (0,1)
        let old_nodes = vec![
            make_node("a", "text", Some("A"), 0),
            make_node("b", "text", Some("B"), 1),
            make_node("c", "text", Some("C"), 2),
            make_node("d", "text", Some("D"), 3),
        ];
        let new_nodes = vec![
            make_node("c", "text", Some("C"), 0),
            make_node("d", "text", Some("D"), 1),
            make_node("a", "text", Some("A"), 2),
            make_node("b", "text", Some("B"), 3),
        ];
        let pairs = vec![
            make_pair(0, 2, 0.95, MatchBand::Matched), // A → new[2]
            make_pair(1, 3, 0.95, MatchBand::Matched), // B → new[3]
            make_pair(2, 0, 0.95, MatchBand::Matched), // C → new[0]
            make_pair(3, 1, 0.95, MatchBand::Matched), // D → new[1]
        ];
        let outcome = make_outcome(pairs);
        let issues = sequence_issues(&old_nodes, &new_nodes, &outcome, "desktop", None);
        let swap_count = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::ComponentSwapped)
            .count();
        assert_eq!(swap_count, 1, "block collapse yields exactly one swap");
        let reorder_count = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::ComponentReordered)
            .count();
        assert_eq!(reorder_count, 0, "no reorders");
    }

    /// strict threshold boundary: displacement == SEQ_MIN_DISPLACEMENT suppressed,
    /// displacement == SEQ_MIN_DISPLACEMENT+1 emitted (spec §6.2 "exceeds").
    ///
    /// Uses perm=[2,4,1,0,3], sigma=[2,4,1,0,3], LIS={D(pos3),E(pos4)}.
    /// Non-LIS blocks:
    ///   A: disp=|0-2|=2 == SEQ_MIN_DISPLACEMENT → suppressed (strictly > required).
    ///   B: disp=|1-4|=3 == SEQ_MIN_DISPLACEMENT+1 → emitted.
    ///   C: disp=|2-1|=1 < SEQ_MIN_DISPLACEMENT → suppressed.
    #[test]
    fn test_strict_threshold_boundary() {
        assert_eq!(SEQ_MIN_DISPLACEMENT, 2, "threshold constant must be 2");

        // perm=[2,4,1,0,3]: no adjacent pairs, all single-node blocks.
        let old_nodes = vec![
            make_node("a", "heading", Some("Block A"), 0),
            make_node("b", "heading", Some("Block B"), 1),
            make_node("c", "heading", Some("Block C"), 2),
            make_node("d", "heading", Some("Block D"), 3),
            make_node("e", "heading", Some("Block E"), 4),
        ];
        // new array: D(seq=0), C(seq=1), A(seq=2), E(seq=3), B(seq=4)
        let new_nodes = vec![
            make_node("d", "heading", Some("Block D"), 0),
            make_node("c", "heading", Some("Block C"), 1),
            make_node("a", "heading", Some("Block A"), 2),
            make_node("e", "heading", Some("Block E"), 3),
            make_node("b", "heading", Some("Block B"), 4),
        ];
        let pairs = vec![
            make_pair(0, 2, 0.95, MatchBand::Matched), // A → new[2]
            make_pair(1, 4, 0.95, MatchBand::Matched), // B → new[4]
            make_pair(2, 1, 0.95, MatchBand::Matched), // C → new[1]
            make_pair(3, 0, 0.95, MatchBand::Matched), // D → new[0]
            make_pair(4, 3, 0.95, MatchBand::Matched), // E → new[3]
        ];
        let outcome = make_outcome(pairs);
        let issues = sequence_issues(&old_nodes, &new_nodes, &outcome, "desktop", None);

        // A (displacement == SEQ_MIN_DISPLACEMENT = 2) must be suppressed.
        let a_emitted = issues.iter().any(|i| {
            i.issue_type == IssueType::ComponentReordered && i.message.contains("Block A")
        });
        assert!(
            !a_emitted,
            "Block A (displacement=2 == SEQ_MIN_DISPLACEMENT) must be suppressed by strict > rule"
        );

        // C (displacement=1 < SEQ_MIN_DISPLACEMENT) must be suppressed.
        let c_emitted = issues.iter().any(|i| {
            i.issue_type == IssueType::ComponentReordered && i.message.contains("Block C")
        });
        assert!(!c_emitted, "Block C (displacement=1) must be suppressed");

        // B (displacement=3 == SEQ_MIN_DISPLACEMENT+1) must be emitted.
        let b_emitted = issues.iter().any(|i| {
            i.issue_type == IssueType::ComponentReordered && i.message.contains("Block B")
        });
        assert!(
            b_emitted,
            "Block B (displacement=3 = SEQ_MIN_DISPLACEMENT+1) must be emitted"
        );

        let reorder_count = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::ComponentReordered)
            .count();
        assert_eq!(
            reorder_count, 1,
            "exactly one reorder: B(disp=3); A(disp=2) and C(disp=1) suppressed"
        );
    }

    /// uncertain-band pairs excluded (swap whose B-side pairs are Uncertain → no issue)
    #[test]
    fn test_uncertain_band_pairs_excluded() {
        // A and B would swap, but B's pairs are Uncertain → excluded from eligible.
        // Only A is eligible (Matched), so no swap can be detected.
        let old_nodes = vec![
            make_node("a1", "heading", Some("Section A"), 0),
            make_node("b1", "heading", Some("Section B"), 1),
        ];
        let new_nodes = vec![
            make_node("b1", "heading", Some("Section B"), 0),
            make_node("a1", "heading", Some("Section A"), 1),
        ];
        let pairs = vec![
            make_pair(0, 1, 0.97, MatchBand::Matched),   // A: matched
            make_pair(1, 0, 0.62, MatchBand::Uncertain), // B: uncertain → excluded
        ];
        let outcome = make_outcome(pairs);
        let issues = sequence_issues(&old_nodes, &new_nodes, &outcome, "desktop", None);
        assert!(
            issues.is_empty(),
            "uncertain pairs excluded → no issue emitted"
        );
    }

    /// assignment-stage pairs excluded: cross-paired duplicate-content nodes (v08 shape)
    ///
    /// Simulates what v08-cta-removed produced: duplicate-href links in a carousel were
    /// cross-paired at MatchStage::Assignment after a 2-node CTA removal shifted positional
    /// tiebreaks. Two of those cross-pairs form a block-level transposition (component_swapped
    /// candidate) and one has a large displacement (component_reordered candidate). Since all
    /// pairs are Assignment-stage, S0 excludes them all → zero issues.
    #[test]
    fn test_assignment_stage_pairs_excluded() {
        // Three duplicate carousel links — same text/href, different positions.
        // The matcher assigned them in crossed order at Assignment stage.
        let make_link = |id: &str, seq: u32| -> SemanticNode {
            let mut n = make_node(id, "text", Some("https://example.com/reviews"), seq);
            n.anchors.href = Some("https://example.com/reviews".to_string());
            n.anchors.text = Some("https://example.com/reviews".to_string());
            n
        };
        let old_nodes = vec![
            make_link("link_a", 87),
            make_link("link_b", 94),
            make_link("link_c", 102),
        ];
        // In new page the links are at shifted positions (CTA removed above them).
        let new_nodes = vec![
            make_link("link_a", 88), // would match link_a
            make_link("link_b", 92), // cross-match target
            make_link("link_c", 85), // cross-match target
        ];
        // Cross-pairing at Assignment stage: link_a↔new[2], link_b↔new[0], link_c↔new[1].
        // This forms: link_a→new[2] (disp large), link_b→new[0] (disp=2), link_c→new[1].
        // At block level: transposition (link_a, link_b-link_c) would form a swap candidate.
        let mut pair_a = make_pair(0, 2, 0.78, MatchBand::Matched);
        pair_a.stage = MatchStage::Assignment;
        let mut pair_b = make_pair(1, 0, 0.78, MatchBand::Matched);
        pair_b.stage = MatchStage::Assignment;
        let mut pair_c = make_pair(2, 1, 0.78, MatchBand::Matched);
        pair_c.stage = MatchStage::Assignment;
        let outcome = make_outcome(vec![pair_a, pair_b, pair_c]);
        let issues = sequence_issues(&old_nodes, &new_nodes, &outcome, "desktop", None);
        assert!(
            issues.is_empty(),
            "assignment-stage pairs must be excluded from sequence diff (S0 filter)"
        );
    }

    /// ambiguous-LIS permutation → pinned canonical displaced set (determinism)
    ///
    /// sigma=[3,0,1,2] (4-element rotation with no block collapse).
    /// No transpositions. LIS (patience on [3,0,1,2]):
    ///   i=0,v=3: tails=[0(A)]
    ///   i=1,v=0: pos=0, tails=[1(B)]
    ///   i=2,v=1: append, tails=[1(B),2(C)]
    ///   i=3,v=2: append, tails=[1(B),2(C),3(D)]
    /// Reconstruct: last=3(D,v=2), pred=2(C,v=1), pred=1(B,v=0). LIS={1,2,3}=B,C,D.
    /// Non-LIS: A (sub_idx=0), displacement=|0-3|=3 > SEQ_MIN_DISPLACEMENT=2 → emitted.
    /// Determinism test: two runs must produce identical issue ids.
    #[test]
    fn test_ambiguous_lis_determinism() {
        // perm=[3,0,1,2]: no adjacent pairs (3≠1, 0≠4, 1≠1... wait perm[2]=1, perm[1]=0: 0+1=1=perm[2]!)
        // Actually: check adjacency: perm[1]=0, perm[0]+1=4 ✓ not adjacent.
        //           perm[2]=1, perm[1]+1=1 → ADJACENT! B and C collapse into one block.
        // So perm=[3,0,1,2] collapses B,C,D into a single block (0,1,2 are consecutive starting at 0).
        // Blocks: {A}(new_rank=3), {BCD}(new_rank=0).
        // sigma=[1,0] → transposition → swap! Not what we want.
        //
        // Use perm=[3,1,4,0,2] for 5 nodes instead (verified no adjacency):
        // perm[1]=1, perm[0]+1=4 ✓; perm[2]=4, perm[1]+1=2 ✓; perm[3]=0, perm[2]+1=5 ✓; perm[4]=2, perm[3]+1=1 ✓.
        // sigma: sort by new_rank: (new=0,old=3),(new=1,old=1),(new=2,old=4),(new=3,old=0),(new=4,old=2).
        // sigma[0]=3,sigma[1]=1,sigma[2]=4,sigma[3]=0... wait sigma[i] = new_block_rank of old block i.
        // sigma[0]=3(A→new_rank=3), sigma[1]=1(B→new_rank=1), sigma[2]=4(C→new_rank=4),
        // sigma[3]=0(D→new_rank=0), sigma[4]=2(E→new_rank=2).
        // sigma=[3,1,4,0,2].
        // Transpositions: (0,3): sigma[0]=3,sigma[3]=0 → i=0,j=3: j==sigma[0] AND sigma[j]==i ✓ → SWAP!
        // Not good. Need a permutation with no transpositions and a non-LIS block with disp>2.
        //
        // Use perm=[4,1,2,3,0] for 5 nodes (5-cycle):
        // perm[1]=1,perm[0]+1=5 ✓; perm[2]=2,perm[1]+1=2 → ADJACENT! B and C collapse.
        //
        // Back to 4-node case. Try perm=[3,0,2,1]:
        // perm[1]=0,perm[0]+1=4 ✓; perm[2]=2,perm[1]+1=1 ✓; perm[3]=1,perm[2]+1=3 ✓. All single blocks.
        // sigma: (new=0,old=1),(new=1,old=3),(new=2,old=2),(new=3,old=0).
        // sigma[0]=3,sigma[1]=0,sigma[2]=2,sigma[3]=1 → sigma=[3,0,2,1].
        // Transpositions: sigma[0]=3,sigma[3]=1≠0. sigma[1]=0,sigma[0]=3≠1. No transpositions.
        // LIS patience on [3,0,2,1]:
        //   i=0,v=3: tails=[0]
        //   i=1,v=0: pos=0, tails=[1]
        //   i=2,v=2: append, tails=[1,2]
        //   i=3,v=1: pos=1 (values[1]=0<1≤values[2]=2 → pos=1), tails=[1,3]
        // Reconstruct: last=3(v=1), pred[3]=1(at step i=3 pos=1, tails[0]=1). pred[1]=None.
        // LIS={1,3}→B(sigma=0) and D(sigma=1). Two different LIS of length 2 exist: {B,D}([0,1])
        // and {B,C}([0,2]). The patience algorithm pins {B,D}.
        // Non-LIS: A(sub=0,disp=|0-3|=3>2→emit) and C(sub=2,disp=|2-2|=0→suppress).
        // Result: 1 reorder (A, disp=3). Determinism test: run twice → same ids.
        let old_nodes = vec![
            make_node("a", "heading", Some("Block A"), 0), // sigma→3
            make_node("b", "heading", Some("Block B"), 1), // sigma→0
            make_node("c", "heading", Some("Block C"), 2), // sigma→2
            make_node("d", "heading", Some("Block D"), 3), // sigma→1
        ];
        // new array: B(seq=0), D(seq=1), C(seq=2), A(seq=3)
        // (perm[0]=3→A goes to new_rank=3, perm[1]=0→B goes to new_rank=0,
        //  perm[2]=2→C stays, perm[3]=1→D goes to new_rank=1)
        let new_nodes = vec![
            make_node("b", "heading", Some("Block B"), 0),
            make_node("d", "heading", Some("Block D"), 1),
            make_node("c", "heading", Some("Block C"), 2),
            make_node("a", "heading", Some("Block A"), 3),
        ];
        // pairs: A→new[3], B→new[0], C→new[2], D→new[1]
        let pairs = vec![
            make_pair(0, 3, 0.95, MatchBand::Matched), // A → new[3]
            make_pair(1, 0, 0.95, MatchBand::Matched), // B → new[0]
            make_pair(2, 2, 0.95, MatchBand::Matched), // C → new[2]
            make_pair(3, 1, 0.95, MatchBand::Matched), // D → new[1]
        ];
        let outcome = make_outcome(pairs);
        let issues = sequence_issues(&old_nodes, &new_nodes, &outcome, "desktop", None);
        // Check determinism: run twice and compare issue ids.
        let issues2 = sequence_issues(&old_nodes, &new_nodes, &outcome, "desktop", None);
        let ids1: Vec<_> = issues.iter().map(|i| i.id.clone()).collect();
        let ids2: Vec<_> = issues2.iter().map(|i| i.id.clone()).collect();
        assert_eq!(ids1, ids2, "LIS result must be deterministic");

        // A (displacement=3 > SEQ_MIN_DISPLACEMENT=2) must emit.
        let a_emitted = issues.iter().any(|i| {
            i.issue_type == IssueType::ComponentReordered && i.message.contains("Block A")
        });
        assert!(a_emitted, "Block A (displacement=3) must be emitted");

        let reorder_count = issues
            .iter()
            .filter(|i| i.issue_type == IssueType::ComponentReordered)
            .count();
        assert_eq!(reorder_count, 1, "exactly one reorder (A displaced by 3)");
    }

    /// id stability under bbox/score jitter (re-capture stability §7.1)
    #[test]
    fn test_id_stability_under_bbox_score_jitter() {
        // The issue id is computed from type, viewport, and rep's anchors — not bbox or score.
        let old_nodes_v1 = vec![
            make_node("a", "heading", Some("Section A"), 0),
            make_node("b", "heading", Some("Section B"), 1),
        ];
        let old_nodes_v2 = vec![
            // Same content, different bbox (y shifted by 5px)
            SemanticNode {
                bbox: [0, 5, 500, 80],
                ..make_node("a", "heading", Some("Section A"), 0)
            },
            SemanticNode {
                bbox: [0, 105, 500, 80],
                ..make_node("b", "heading", Some("Section B"), 1)
            },
        ];
        let new_nodes = vec![
            make_node("b", "heading", Some("Section B"), 0),
            make_node("a", "heading", Some("Section A"), 1),
        ];
        // Pairs with different scores (score jitter).
        let pairs_v1 = vec![
            make_pair(0, 1, 0.97, MatchBand::Matched),
            make_pair(1, 0, 0.97, MatchBand::Matched),
        ];
        let pairs_v2 = vec![
            make_pair(0, 1, 0.94, MatchBand::Matched), // score jittered
            make_pair(1, 0, 0.94, MatchBand::Matched),
        ];
        let outcome_v1 = make_outcome(pairs_v1);
        let outcome_v2 = make_outcome(pairs_v2);

        let issues_v1 = sequence_issues(&old_nodes_v1, &new_nodes, &outcome_v1, "desktop", None);
        let issues_v2 = sequence_issues(&old_nodes_v2, &new_nodes, &outcome_v2, "desktop", None);

        assert!(!issues_v1.is_empty(), "should produce issues");
        assert_eq!(issues_v1.len(), issues_v2.len(), "same number of issues");
        for (i1, i2) in issues_v1.iter().zip(issues_v2.iter()) {
            assert_eq!(i1.id, i2.id, "id must be stable under bbox/score jitter");
            assert_eq!(i1.issue_type, i2.issue_type, "type must be stable");
        }
    }

    /// truncate80 never panics on multibyte chars straddling the 80-byte boundary,
    /// and issue construction stays panic-free for such anchor text.
    #[test]
    fn test_truncate80_multibyte_char_boundary() {
        // Build a string: 79 ASCII bytes then a UTF-8 RIGHT SINGLE QUOTATION MARK (U+2019)
        // which encodes as 3 bytes (0xE2 0x80 0x99).  Total length = 82 bytes > 80.
        // A raw byte slice [..80] would land in the middle of the 3-byte sequence → panic.
        let ascii_part = "a".repeat(79);
        let multibyte = "\u{2019}"; // 3 bytes: 0xE2 0x80 0x99
        let long_text = format!("{}{}", ascii_part, multibyte);
        assert_eq!(long_text.len(), 82, "setup: 79 + 3 = 82 bytes");

        let result = truncate80(&long_text);

        // Must be valid UTF-8 (String guarantees this).
        // Must be ≤ 80 bytes.
        assert!(
            result.len() <= 80,
            "truncated string must be ≤ 80 bytes, got {}",
            result.len()
        );
        // Must equal the 79-char ASCII prefix (the multibyte char was trimmed).
        assert_eq!(
            result, ascii_part,
            "multibyte char at boundary must be dropped"
        );

        // Exercise the full issue-construction path: swap two nodes whose anchor text
        // is the long string, verifying no panic and a well-formed message.
        let long_node = |id: &str, seq: u32| -> SemanticNode {
            let mut n = make_node(id, "heading", Some(&long_text), seq);
            n.anchors.text = Some(long_text.clone());
            n
        };
        let old_nodes = vec![long_node("x", 0), long_node("y", 1)];
        let new_nodes = vec![long_node("y", 0), long_node("x", 1)];
        let pairs = vec![
            make_pair(0, 1, 0.95, MatchBand::Matched),
            make_pair(1, 0, 0.95, MatchBand::Matched),
        ];
        let outcome = make_outcome(pairs);
        // Must not panic.
        let issues = sequence_issues(&old_nodes, &new_nodes, &outcome, "desktop", None);
        assert_eq!(issues.len(), 1, "one swap issue");
        // The message must be valid UTF-8 (it is, since it's a String) and the
        // rep text embedded in it must be ≤ 80 bytes.
        let msg = &issues[0].message;
        assert!(
            msg.is_ascii() || msg.is_char_boundary(0),
            "message must be valid UTF-8"
        );
        // The truncated rep text appears in the message; verify no stray bytes.
        assert!(
            msg.contains(&ascii_part),
            "truncated ASCII prefix must appear in message"
        );
    }

    /// WP-B (M6.md §6): new_lang is stamped on every emitted sequence issue.
    ///
    /// Two nodes swap → one component_swapped. The supplied lang must appear on it.
    /// One node reorders → one component_reordered. The supplied lang must appear on it.
    #[test]
    fn test_sequence_issues_locale_stamped() {
        // Swap scenario: old=[A, B], new=[B, A] → one component_swapped.
        let old_nodes = vec![
            make_node("a", "heading", Some("Section A"), 0),
            make_node("b", "heading", Some("Section B"), 1),
        ];
        let new_nodes = vec![
            make_node("b", "heading", Some("Section B"), 0),
            make_node("a", "heading", Some("Section A"), 1),
        ];
        let pairs = vec![
            make_pair(0, 1, 0.97, MatchBand::Matched),
            make_pair(1, 0, 0.97, MatchBand::Matched),
        ];
        let outcome = make_outcome(pairs);
        let issues = sequence_issues(
            &old_nodes,
            &new_nodes,
            &outcome,
            "desktop",
            Some("en-US".to_string()),
        );
        assert_eq!(issues.len(), 1, "exactly one swap issue");
        assert_eq!(
            issues[0].locale,
            Some("en-US".to_string()),
            "component_swapped must carry the supplied locale"
        );

        // Reorder scenario: old=[A, B, C, D, E] with one B displaced far.
        // perm=[2,4,1,0,3] (same as test_rotation_one_reorder_no_swap).
        let old_nodes2 = vec![
            make_node("a", "heading", Some("Block A"), 0),
            make_node("b", "heading", Some("Block B"), 1),
            make_node("c", "heading", Some("Block C"), 2),
            make_node("d", "heading", Some("Block D"), 3),
            make_node("e", "heading", Some("Block E"), 4),
        ];
        let new_nodes2 = vec![
            make_node("d", "heading", Some("Block D"), 0),
            make_node("c", "heading", Some("Block C"), 1),
            make_node("a", "heading", Some("Block A"), 2),
            make_node("e", "heading", Some("Block E"), 3),
            make_node("b", "heading", Some("Block B"), 4),
        ];
        let pairs2 = vec![
            make_pair(0, 2, 0.95, MatchBand::Matched),
            make_pair(1, 4, 0.95, MatchBand::Matched),
            make_pair(2, 1, 0.95, MatchBand::Matched),
            make_pair(3, 0, 0.95, MatchBand::Matched),
            make_pair(4, 3, 0.95, MatchBand::Matched),
        ];
        let outcome2 = make_outcome(pairs2);
        let issues2 = sequence_issues(
            &old_nodes2,
            &new_nodes2,
            &outcome2,
            "desktop",
            Some("en-US".to_string()),
        );
        let reorder_issues: Vec<_> = issues2
            .iter()
            .filter(|i| i.issue_type == IssueType::ComponentReordered)
            .collect();
        assert!(
            !reorder_issues.is_empty(),
            "expected at least one reorder issue"
        );
        for issue in &reorder_issues {
            assert_eq!(
                issue.locale,
                Some("en-US".to_string()),
                "component_reordered must carry the supplied locale"
            );
        }

        // None lang → locale field is None. Use the original swap scenario.
        let pairs_swap = vec![
            make_pair(0, 1, 0.97, MatchBand::Matched),
            make_pair(1, 0, 0.97, MatchBand::Matched),
        ];
        let outcome_swap = make_outcome(pairs_swap);
        let issues_no_lang =
            sequence_issues(&old_nodes, &new_nodes, &outcome_swap, "desktop", None);
        assert_eq!(issues_no_lang.len(), 1);
        assert_eq!(
            issues_no_lang[0].locale, None,
            "None new_lang must produce locale: None"
        );
    }
}
