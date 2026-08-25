/**
 * Port-parity U6: capture-time clickable-area hit-test probe.
 *
 * `runHitTestProbe` is exported for its type shape but is executed inside the
 * browser via page.evaluate() — like extractPageModel() in page-model.ts, it
 * must be self-contained (no imports used at runtime). Several helpers
 * (landmark detection, the nth-of-type selector builder) are DUPLICATED here
 * from extract/page-model.ts because in-page code cannot close over Node.js
 * imports. Each duplication site carries a comment pointing back to the
 * other file — keep the two in sync.
 */

/** Kinds that are probe-eligible regardless of the hasOnclick flag. */
const PROBE_ELIGIBLE_KINDS: readonly string[] = ["link", "button", "field", "form"];

/**
 * Pure eligibility predicate shared by capture.ts (to build the eligible-node
 * list before invoking the in-page probe) and tests. Probe-eligible nodes:
 * kind in {link, button, field, form} OR hasOnclick.
 */
export function isProbeEligible(kind: string, hasOnclick?: boolean): boolean {
  return PROBE_ELIGIBLE_KINDS.includes(kind) || hasOnclick === true;
}

/** Minimal per-node shape the in-page probe needs — coordinates are derived fresh, never passed in. */
export interface HitTestProbeInput {
  id: string;
  cssSelector: string | null;
  bbox: [number, number, number, number];
}

export type RawHitTestOutcome = "hit" | "miss" | "clipped" | "offViewport";

export interface RawHitTestPoint {
  o: RawHitTestOutcome;
  winner?: string;
}

export type RawHitTestSkipReason = "tooSmall" | "offDocument" | "detached";

export interface RawHitTestEntry {
  status: "sampled" | "skipped";
  skipReason?: RawHitTestSkipReason;
  gridSize?: number;
  points?: RawHitTestPoint[];
}

/**
 * The in-page hit-test probe executed via page.evaluate(runHitTestProbe, nodes).
 *
 * For each eligible node (in the order given — bundle/document order): re-
 * resolves the element via its stored cssSelector, scrolls it into view
 * (center), and samples a 5x5 grid of document.elementFromPoint calls against
 * the FRESH post-scroll viewport-relative rect, inset 2px per edge. Restores
 * scroll to (0,0) after the last node. Coordinates are never stored — only
 * the per-point outcome (and, for a miss, the winning element's
 * landmark-relative selector).
 */
export function runHitTestProbe(
  nodes: HitTestProbeInput[]
): Record<string, RawHitTestEntry> {
  const GRID_SIZE = 5;
  const INSET_PX = 2;

  // ── Landmark + selector-builder helpers ──────────────────────────────────
  // NOTE: duplicated from extract/page-model.ts (TAG_TO_LANDMARK,
  // ROLE_TO_LANDMARK, getLandmarkRole, getNearestLandmarkElement,
  // buildSelector) — in-page code must be self-contained. Keep in sync with
  // page-model.ts.
  const TAG_TO_LANDMARK: Record<string, string> = {
    header: "banner",
    nav: "navigation",
    main: "main",
    footer: "contentinfo",
    aside: "complementary",
    form: "form",
  };

  const ROLE_TO_LANDMARK: Record<string, string> = {
    banner: "banner",
    navigation: "navigation",
    main: "main",
    contentinfo: "contentinfo",
    complementary: "complementary",
    form: "form",
    region: "region",
  };

  function getLandmarkRole(el: Element): string | null {
    const role = el.getAttribute("role");
    if (role) {
      const mapped = ROLE_TO_LANDMARK[role.toLowerCase()];
      if (mapped) return mapped;
    }
    const tag = el.tagName.toLowerCase();
    return TAG_TO_LANDMARK[tag] ?? null;
  }

  function getNearestLandmarkElement(el: Element): Element | null {
    let current: Element | null = el.parentElement;
    while (current && current !== document.documentElement) {
      if (getLandmarkRole(current)) return current;
      current = current.parentElement;
    }
    return null;
  }

  function buildSelector(el: Element, landmarkEl: Element | null): string {
    const parts: string[] = [];
    let current: Element | null = el;
    const root = landmarkEl ?? document.body;

    while (current && current !== root && current !== document.documentElement) {
      const tag = current.tagName.toLowerCase();
      const parent: Element | null = current.parentElement;
      if (!parent) break;
      let idx = 1;
      for (let i = 0; i < parent.children.length; i++) {
        const sibling = parent.children[i];
        if (!sibling) continue;
        if (sibling === current) break;
        if (sibling.tagName.toLowerCase() === tag) idx++;
      }
      parts.unshift(`${tag}:nth-of-type(${idx})`);
      current = parent;
    }

    return parts.join(" > ") || el.tagName.toLowerCase();
  }

  // ── Label / labeled-control association ──────────────────────────────────
  // A winner counts as a hit when it IS the node's associated <label> (via
  // .control), or the node is a labelable control whose .labels list contains
  // (or contains/is contained by) the winner.
  function isLabelAssociatedHit(target: Element, winner: Element): boolean {
    if (winner.tagName === "LABEL") {
      const control = (winner as HTMLLabelElement).control;
      if (control === target) return true;
    }
    const labels = (target as HTMLInputElement).labels;
    if (labels) {
      for (let i = 0; i < labels.length; i++) {
        const lbl = labels[i];
        if (!lbl) continue;
        if (lbl === winner) return true;
        if (lbl.contains(winner)) return true;
        if (winner.contains(lbl)) return true;
      }
    }
    return false;
  }

  const docWidth = document.documentElement.scrollWidth;
  const docHeight = document.documentElement.scrollHeight;

  const result: Record<string, RawHitTestEntry> = {};

  for (let ni = 0; ni < nodes.length; ni++) {
    const node = nodes[ni]!;

    // The tooSmall gate uses the ORIGINAL extraction-time bbox (page coords,
    // scroll-invariant width/height) — catches semantically tiny nodes (e.g.
    // a 1px-tall skip-link) regardless of current scroll position, before we
    // ever touch the DOM for this node.
    const bboxW = node.bbox[2];
    const bboxH = node.bbox[3];
    if (bboxW < 5 || bboxH < 5) {
      result[node.id] = { status: "skipped", skipReason: "tooSmall" };
      continue;
    }

    if (!node.cssSelector) {
      result[node.id] = { status: "skipped", skipReason: "detached" };
      continue;
    }

    let el: Element | null = null;
    try {
      el = document.querySelector(node.cssSelector);
    } catch {
      el = null;
    }
    if (!el) {
      result[node.id] = { status: "skipped", skipReason: "detached" };
      continue;
    }

    try {
      el.scrollIntoView({ block: "center", inline: "nearest" });
    } catch {
      // scrollIntoView failing is not itself a skip reason — fall through to
      // whatever rect getBoundingClientRect() below reports.
    }

    const rect = el.getBoundingClientRect();
    const pageX = rect.left + window.scrollX;
    const pageY = rect.top + window.scrollY;
    const offDocument =
      rect.width <= 0 ||
      rect.height <= 0 ||
      pageX + rect.width <= 0 ||
      pageY + rect.height <= 0 ||
      pageX >= docWidth ||
      pageY >= docHeight;
    if (offDocument) {
      result[node.id] = { status: "skipped", skipReason: "offDocument" };
      continue;
    }

    const left = rect.left + INSET_PX;
    const right = rect.right - INSET_PX;
    const top = rect.top + INSET_PX;
    const bottom = rect.bottom - INSET_PX;

    const points: RawHitTestPoint[] = [];
    for (let row = 0; row < GRID_SIZE; row++) {
      const y = top + ((bottom - top) * row) / (GRID_SIZE - 1);
      for (let col = 0; col < GRID_SIZE; col++) {
        const x = left + ((right - left) * col) / (GRID_SIZE - 1);

        if (x < 0 || y < 0 || x >= window.innerWidth || y >= window.innerHeight) {
          points.push({ o: "offViewport" });
          continue;
        }

        const winner = document.elementFromPoint(x, y);
        if (!winner) {
          points.push({ o: "offViewport" });
          continue;
        }

        if (winner === el || el.contains(winner) || isLabelAssociatedHit(el, winner)) {
          points.push({ o: "hit" });
          continue;
        }

        if (winner.contains(el)) {
          points.push({ o: "clipped" });
          continue;
        }

        const winnerLandmarkEl = getNearestLandmarkElement(winner);
        const winnerSelector = buildSelector(winner, winnerLandmarkEl);
        points.push({ o: "miss", winner: winnerSelector });
      }
    }

    result[node.id] = { status: "sampled", gridSize: GRID_SIZE, points };
  }

  // Restore scroll position after probing every node.
  window.scrollTo(0, 0);

  return result;
}
