/**
 * Page-model extraction: runs inside the browser via page.evaluate().
 * This file exports types and an extraction function that is serialized and
 * injected into the page context.
 *
 * All code in extractPageModel must be self-contained (no imports used at runtime).
 */

export interface RawSemanticNode {
  id: string;
  kind: string;
  role: string | null;
  text: string | null;
  accName: string | null;
  href: string | null;
  imageAlt: string | null;
  bbox: [number, number, number, number];
  seqIndex: number;
  anchors: {
    text: string | null;
    role: string | null;
    href: string | null;
    alt: string | null;
    ariaLabel: string | null;
    nearestHeading: string | null;
    landmark: string | null;
    ordinalInLandmark: number | null;
  };
  cssSelector: string | null;
  // M3 fields
  rawHref: string | null;
  src: string | null;
  naturalWidth: number | null;
  naturalHeight: number | null;
  loaded: boolean | null;
  headingLevel: number | null;
}

/** Descriptor for a true ancestor element (not itself a SemanticNode). */
export interface RawAncestorDescriptor {
  id: string;
  tag: string;
  bbox: [number, number, number, number];
  depth: number;
  cssSelector: string | null;
  anchors: {
    text: string | null;
    role: null;
    href: null;
    alt: null;
    ariaLabel: null;
    nearestHeading: string | null;
    landmark: string | null;
    ordinalInLandmark: number | null;
  };
}

/** Style candidates metadata emitted alongside computedStyles. */
export interface RawStyleCandidates {
  ancestors: RawAncestorDescriptor[];
  chains: Record<string, string[]>;
  budget: number;
  truncated: boolean;
  droppedCount: number;
}

export interface RawLandmarkRect {
  path: string;
  role: string;
  heading: string | null;
  bbox: [number, number, number, number];
}

export interface RawPageModelResult {
  nodes: RawSemanticNode[];
  pageHeight: number;
  landmarks: string[];
  landmarkRects: RawLandmarkRect[];
  computedStyles: Record<string, Record<string, string>>;
  styleCandidates: RawStyleCandidates;
}

/**
 * The extraction function executed inside the browser context via page.evaluate().
 * maxTextLength is passed as an argument.
 */
export function extractPageModel(maxTextLength: number): RawPageModelResult {
  // ── Normalization helpers (must be inline — no imports in browser context) ──

  function normalizeStr(s: string, maxLen: number): string {
    if (!s) return s;
    // Replace NBSP with space
    let r = s.replace(/ /g, " ");
    // Strip C0 control chars (keep whitespace chars for collapse) and C1
    r = r.replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f\x80-\x9f]/g, "");
    // Collapse whitespace
    r = r.replace(/\s+/g, " ");
    r = r.trim();
    if (r.length > maxLen) r = r.slice(0, maxLen);
    return r;
  }

  // ── Curated CSS property list ──────────────────────────────────────────────
  // NOTE: This list is also defined in extract/computed-style.ts (Node.js side).
  // Keep the two in sync.
  //
  // margin-top/right/bottom/left are read via the Typed OM (computedStyleMap)
  // rather than getComputedStyle — see M4.md §4b item 3 for rationale: Chromium's
  // getComputedStyle resolved value for `margin: auto` is a used value that proved
  // unstable (0px / 104px / 120px) across byte-identical captures; computedStyleMap
  // returns the computed value "auto" (deterministic).
  // 33 properties total (issue #4 / R4b: text-decoration-line, z-index,
  // max-width, pointer-events added — text-decoration-line NOT the
  // `text-decoration` shorthand, which embeds color and would be noise).
  const COMPUTED_STYLE_PROPS: string[] = [
    "color",
    "background-color",
    "background-image",
    "background",
    "border",
    "border-radius",
    "box-shadow",
    "font-family",
    "font-size",
    "font-weight",
    "line-height",
    "letter-spacing",
    "text-align",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "display",
    "position",
    "opacity",
    "flex-direction",
    "justify-content",
    "align-items",
    "gap",
    "grid-template-columns",
    "text-decoration-line",
    "z-index",
    "max-width",
    "pointer-events",
  ];

  // The four margin properties read via Typed OM instead of getComputedStyle.
  // See comment above and M4.md §4b item 3.
  const MARGIN_PROPS: string[] = [
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
  ];

  // Maximum character length for a computed CSS property value.
  // NOTE: also defined in extract/computed-style.ts — keep in sync.
  const COMPUTED_STYLE_VALUE_MAX_LEN = 1000;

  /**
   * Cap and sanitize a computed CSS value.
   * NOTE: logic also in extract/computed-style.ts capStyleValue() — keep in sync.
   */
  function capStyleValue(value: string): string {
    let result = value.replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f\x80-\x9f]/g, "");
    if (result.length > COMPUTED_STYLE_VALUE_MAX_LEN) {
      result = result.slice(0, COMPUTED_STYLE_VALUE_MAX_LEN);
    }
    return result;
  }

  /** Read all curated properties for an element.
   *
   * margin-* are read via the Typed OM (computedStyleMap) to get computed values
   * ("auto" serializes as "auto"); all other properties use getComputedStyle.
   * Falls back to getComputedStyle for margins if computedStyleMap throws.
   */
  function readComputedStyle(el: Element): Record<string, string> {
    const cs = window.getComputedStyle(el);
    const out: Record<string, string> = {};

    // Attempt to read margin-* via Typed OM once per element (M4.md §4b item 3).
    // computedStyleMap() is Chromium-only — the only engine in scope for this tool.
    const marginValues: Record<string, string> = {};
    try {
      // Cast through unknown: TypeScript lib does not always include computedStyleMap.
      const styleMap = (el as unknown as { computedStyleMap(): StylePropertyMapReadOnly }).computedStyleMap();
      for (let mi = 0; mi < MARGIN_PROPS.length; mi++) {
        const prop = MARGIN_PROPS[mi]!;
        try {
          const val = styleMap.get(prop);
          if (val != null) {
            marginValues[prop] = capStyleValue(val.toString());
          }
        } catch {
          // Property not in map — fall through to getComputedStyle below.
        }
      }
    } catch {
      // computedStyleMap() not available or threw — margins will fall back to
      // getComputedStyle via the main loop below.
    }

    for (let i = 0; i < COMPUTED_STYLE_PROPS.length; i++) {
      const prop = COMPUTED_STYLE_PROPS[i]!;
      // Use Typed OM value for margins when available; otherwise getComputedStyle.
      if (prop in marginValues) {
        out[prop] = marginValues[prop]!;
      } else {
        const raw = cs.getPropertyValue(prop);
        if (raw) out[prop] = capStyleValue(raw);
      }
    }
    return out;
  }

  // ── Landmark detection ─────────────────────────────────────────────────────

  // Map from element tag/role to landmark role name
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

  function getNearestLandmark(el: Element): string | null {
    let current: Element | null = el.parentElement;
    while (current && current !== document.documentElement) {
      const lm = getLandmarkRole(current);
      if (lm) return lm;
      current = current.parentElement;
    }
    return null;
  }

  function getNearestLandmarkElement(el: Element): Element | null {
    let current: Element | null = el.parentElement;
    while (current && current !== document.documentElement) {
      if (getLandmarkRole(current)) return current;
      current = current.parentElement;
    }
    return null;
  }

  // ── Classification ─────────────────────────────────────────────────────────

  function classifyElement(
    el: Element
  ): { kind: string; role: string | null } | null {
    const tag = el.tagName.toLowerCase();
    const role = el.getAttribute("role")?.toLowerCase() ?? null;

    // heading: h1-h6
    if (/^h[1-6]$/.test(tag)) {
      return { kind: "heading", role: "heading" };
    }

    // link: a[href]
    if (tag === "a" && (el as HTMLAnchorElement).href) {
      return { kind: "link", role: role ?? "link" };
    }

    // button: button, [role=button], input[type=submit|button]
    if (
      tag === "button" ||
      role === "button" ||
      (tag === "input" &&
        ["submit", "button"].includes(
          (el as HTMLInputElement).type?.toLowerCase()
        ))
    ) {
      return { kind: "button", role: role ?? "button" };
    }

    // image: img
    if (tag === "img") {
      return { kind: "image", role: role ?? "img" };
    }

    // field: input, select, textarea (excluding submit/button)
    if (
      tag === "select" ||
      tag === "textarea" ||
      (tag === "input" &&
        !["submit", "button", "hidden"].includes(
          (el as HTMLInputElement).type?.toLowerCase()
        ))
    ) {
      return { kind: "field", role: role ?? tag };
    }

    // form: form
    if (tag === "form") {
      return { kind: "form", role: role ?? "form" };
    }

    // text: element with at least 1 direct child text node with non-whitespace
    const childNodes = el.childNodes;
    for (let i = 0; i < childNodes.length; i++) {
      const node = childNodes[i];
      if (node && node.nodeType === Node.TEXT_NODE && /\S/.test(node.textContent ?? "")) {
        return { kind: "text", role: role ?? null };
      }
    }

    return null;
  }

  // ── Visibility + bbox check ────────────────────────────────────────────────

  const docWidth = document.documentElement.scrollWidth;
  const docHeight = document.documentElement.scrollHeight;
  const scrollX = window.scrollX;
  const scrollY = window.scrollY;

  function getPageBbox(el: Element): [number, number, number, number] | null {
    const rect = el.getBoundingClientRect();
    const x = Math.floor(rect.left + scrollX);
    const y = Math.floor(rect.top + scrollY);
    const w = Math.ceil(rect.width);
    const h = Math.ceil(rect.height);
    return [x, y, w, h];
  }

  function isVisible(el: Element): boolean {
    // checkVisibility checks display:none, visibility:hidden, opacity:0
    if (!el.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })) {
      return false;
    }
    const rect = el.getBoundingClientRect();
    // bbox area must be > 0
    if (rect.width <= 0 || rect.height <= 0) return false;
    // must intersect page bounds
    const pageX = rect.left + scrollX;
    const pageY = rect.top + scrollY;
    if (pageX + rect.width <= 0 || pageY + rect.height <= 0) return false;
    if (pageX >= docWidth || pageY >= docHeight) return false;
    return true;
  }

  // ── CSS selector builder (landmark-relative) ──────────────────────────────

  function buildSelector(el: Element, landmarkEl: Element | null): string {
    const parts: string[] = [];
    let current: Element | null = el;
    const root = landmarkEl ?? document.body;

    while (current && current !== root && current !== document.documentElement) {
      const tag = current.tagName.toLowerCase();
      const parent: Element | null = current.parentElement;
      if (!parent) break;
      // Count same-tag siblings before current
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

  // Broken images render at zero size when CSS gives them no dimensions, which
  // the area>0 visibility rule would drop — but the element is still in the DOM
  // and its load failure is exactly what G7 detects (M3.md D13). Keep an <img>
  // despite zero area iff it is broken (errored: complete with no intrinsic
  // width) and not hidden by CSS.
  function isBrokenVisibleImage(el: Element): boolean {
    if (!(el instanceof HTMLImageElement)) return false;
    if (!(el.complete && el.naturalWidth === 0)) return false;
    return el.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true });
  }

  // ── Document walk ──────────────────────────────────────────────────────────

  const nodes: RawSemanticNode[] = [];
  let seqIndex = 0;
  let lastHeadingText: string | null = null;
  let lastHeadingEl: Element | null = null;

  // Cache for first-visible-heading text per <section> element (D14).
  const sectionFirstHeadingCache = new Map<Element, string | null>();

  // Find the nearest ancestor <section> element (tag name "section" only).
  // Always starts from parentElement, for both heading and non-heading nodes.
  function getNearestSection(el: Element): Element | null {
    let cur: Element | null = el.parentElement;
    while (cur && cur !== document.documentElement) {
      if (cur.tagName.toLowerCase() === "section") return cur;
      cur = cur.parentElement;
    }
    return null;
  }

  // Return the normalized text of the first visible heading (h1–h6) inside
  // `container`, lazily computed and cached. Returns null when none found.
  function getSectionFirstHeadingText(container: Element): string | null {
    if (sectionFirstHeadingCache.has(container)) {
      return sectionFirstHeadingCache.get(container)!;
    }
    const headings = container.querySelectorAll("h1,h2,h3,h4,h5,h6");
    let result: string | null = null;
    for (let i = 0; i < headings.length; i++) {
      const h = headings[i];
      if (h && isVisible(h)) {
        const raw = (h as HTMLElement).innerText ?? "";
        const normalized = raw ? normalizeStr(raw, maxTextLength) : null;
        result = normalized || null;
        break;
      }
    }
    sectionFirstHeadingCache.set(container, result);
    return result;
  }

  // D14: compute nearestHeading for a node whose element is `el`.
  // The `alreadyNormalizedHeadingText` is the post-update lastHeadingText
  // (already set if this node is a heading, giving self-anchoring behavior).
  function computeNearestHeading(el: Element): string | null {
    const container = getNearestSection(el);
    if (container === null) {
      // No <section> ancestor: M1 behavior
      return lastHeadingText;
    }
    if (lastHeadingEl !== null && container.contains(lastHeadingEl)) {
      // Preceding heading is inside the same section: M1 behavior
      return lastHeadingText;
    }
    // Preceding heading is outside the section (or there is no preceding heading).
    // Use the section's first visible heading.
    const sectionFirst = getSectionFirstHeadingText(container);
    if (sectionFirst !== null) {
      return sectionFirst;
    }
    // Section has no visible heading: fall back to M1 behavior
    return lastHeadingText;
  }

  // Track ordinal counters: Map<landmarkEl, Map<kindText, count>>
  // Using a flat approach: for each (landmarkEl, kind, text) triple
  const ordinalCounters = new Map<
    Element | null,
    Map<string, number>
  >();

  function getOrdinal(
    landmarkEl: Element | null,
    kind: string,
    text: string | null
  ): number {
    if (!ordinalCounters.has(landmarkEl)) {
      ordinalCounters.set(landmarkEl, new Map());
    }
    const map = ordinalCounters.get(landmarkEl)!;
    const key = `${kind}:${text ?? ""}`;
    const count = (map.get(key) ?? 0) + 1;
    map.set(key, count);
    return count;
  }

  // Walk DOM in document order
  const walker = document.createTreeWalker(
    document.body,
    NodeFilter.SHOW_ELEMENT
  );

  // Map from element to its assigned node id (node_N or later anc_N)
  const elementToNodeId = new Map<Element, string>();

  let current: Node | null = walker.currentNode;
  while (current) {
    const el = current as Element;
    if (isVisible(el) || isBrokenVisibleImage(el)) {
      const classification = classifyElement(el);
      if (classification) {
        const { kind, role } = classification;
        const bbox = getPageBbox(el);
        if (bbox) {
          // Text extraction
          const rawText =
            kind === "image"
              ? null
              : (el as HTMLElement).innerText ?? null;
          const normalizedText = rawText
            ? normalizeStr(rawText, maxTextLength)
            : null;
          const textVal = normalizedText || null;

          // Accessible name
          const ariaLabel = el.getAttribute("aria-label");
          const altAttr =
            el instanceof HTMLImageElement ? el.alt : null;
          const normalizedAriaLabel = ariaLabel
            ? normalizeStr(ariaLabel, maxTextLength)
            : null;
          const normalizedAlt = altAttr
            ? normalizeStr(altAttr, maxTextLength)
            : null;
          const accName =
            normalizedAriaLabel || normalizedAlt || textVal;

          // Href
          let hrefVal: string | null = null;
          if (kind === "link" && el instanceof HTMLAnchorElement) {
            hrefVal = el.href || null; // already absolute in browser
          }

          // Image alt
          const imageAlt =
            kind === "image" && el instanceof HTMLImageElement
              ? normalizeStr(el.alt ?? "", maxTextLength) || null
              : null;

          // rawHref (link only: un-resolved href attribute)
          let rawHrefVal: string | null = null;
          if (kind === "link" && el instanceof HTMLAnchorElement) {
            const raw = el.getAttribute("href");
            rawHrefVal = raw ? normalizeStr(raw, maxTextLength) : null;
          }

          // src, naturalWidth, naturalHeight, loaded (image only)
          let srcVal: string | null = null;
          let naturalWidthVal: number | null = null;
          let naturalHeightVal: number | null = null;
          let loadedVal: boolean | null = null;
          if (kind === "image" && el instanceof HTMLImageElement) {
            srcVal = (el.currentSrc || el.src) || null;
            naturalWidthVal = el.naturalWidth;
            naturalHeightVal = el.naturalHeight;
            loadedVal = el.complete && el.naturalWidth > 0;
          }

          // headingLevel (heading only)
          let headingLevelVal: number | null = null;
          if (kind === "heading") {
            const match = el.tagName.toLowerCase().match(/^h([1-6])$/);
            if (match && match[1]) {
              headingLevelVal = parseInt(match[1], 10);
            }
          }

          // Heading tracking (update BEFORE building anchors so heading nodes
          // self-anchor — D14 point 3).
          if (kind === "heading") {
            lastHeadingText = textVal;
            lastHeadingEl = el;
          }

          // Landmark
          const landmark = getNearestLandmark(el);
          const landmarkEl = getNearestLandmarkElement(el);

          // Ordinal
          const ordinalInLandmark = landmark
            ? getOrdinal(landmarkEl, kind, textVal)
            : null;

          // CSS selector
          const cssSelector = buildSelector(el, landmarkEl);

          // D14: compute nearestHeading using section-aware rule
          const nearestHeading = computeNearestHeading(el);

          const nodeId = `node_${seqIndex}`;
          elementToNodeId.set(el, nodeId);

          nodes.push({
            id: nodeId,
            kind,
            role,
            text: textVal,
            accName: accName || null,
            href: hrefVal,
            imageAlt,
            bbox,
            seqIndex,
            anchors: {
              text: textVal,
              role,
              href: hrefVal,
              alt: imageAlt,
              ariaLabel: normalizedAriaLabel,
              nearestHeading,
              landmark,
              ordinalInLandmark,
            },
            cssSelector,
            rawHref: rawHrefVal,
            src: srcVal,
            naturalWidth: naturalWidthVal,
            naturalHeight: naturalHeightVal,
            loaded: loadedVal,
            headingLevel: headingLevelVal,
          });

          seqIndex++;
        }
      }
    }
    current = walker.nextNode();
  }

  // ── Collect page landmarks ──────────────────────────────────────────────

  const landmarkSet = new Set<string>();
  document.querySelectorAll(
    "header,nav,main,footer,aside,form,[role]"
  ).forEach((el) => {
    const lm = getLandmarkRole(el);
    if (lm) landmarkSet.add(lm);
  });
  const landmarks = Array.from(landmarkSet).sort();

  // ── Collect landmarkRects (WP-G) ───────────────────────────────────────
  // Collect geometry for landmark elements + direct section/id-div children of main.
  // Cap at 64 entries total. Document order.

  const LANDMARK_SELECTOR =
    "header,nav,main,footer,aside,form[aria-label],form[aria-labelledby]," +
    "[role=banner],[role=navigation],[role=main],[role=contentinfo]," +
    "[role=complementary],[role=form]";

  // Derive role for landmark rect: explicit role attr wins; else tag-based mapping.
  // header→banner only when NOT inside main/article; footer→contentinfo likewise.
  function getLandmarkRectRole(el: Element): string | null {
    const roleAttr = el.getAttribute("role");
    if (roleAttr) {
      const mapped = ROLE_TO_LANDMARK[roleAttr.toLowerCase()];
      if (mapped) return mapped;
    }
    const tag = el.tagName.toLowerCase();
    if (tag === "header") {
      // Only landmark banner when not inside main or article
      let anc: Element | null = el.parentElement;
      while (anc && anc !== document.documentElement) {
        const t = anc.tagName.toLowerCase();
        if (t === "main" || t === "article") return null;
        anc = anc.parentElement;
      }
      return "banner";
    }
    if (tag === "footer") {
      let anc: Element | null = el.parentElement;
      while (anc && anc !== document.documentElement) {
        const t = anc.tagName.toLowerCase();
        if (t === "main" || t === "article") return null;
        anc = anc.parentElement;
      }
      return "contentinfo";
    }
    return TAG_TO_LANDMARK[tag] ?? null;
  }

  // Find the first h1-h3 inside an element (trimmed, capped 80 chars).
  function getFirstHeadingInside(container: Element): string | null {
    const hs = container.querySelectorAll("h1,h2,h3");
    for (let i = 0; i < hs.length; i++) {
      const h = hs[i];
      if (!h) continue;
      const raw = (h as HTMLElement).innerText ?? "";
      const trimmed = raw.replace(/\s+/g, " ").trim();
      if (trimmed) return trimmed.length > 80 ? trimmed.slice(0, 80) : trimmed;
    }
    return null;
  }

  const landmarkRects: RawLandmarkRect[] = [];
  // Track how many times each role has been seen (for [2],[3] suffixing).
  const roleCount = new Map<string, number>();

  // Pass 1: landmark elements in document order (deduplicated by identity via a Set).
  const seenLandmarkEls = new Set<Element>();
  const landmarkEls = document.querySelectorAll(LANDMARK_SELECTOR);
  for (let i = 0; i < landmarkEls.length; i++) {
    if (landmarkRects.length >= 64) break;
    const el = landmarkEls[i];
    if (!el || seenLandmarkEls.has(el)) continue;
    seenLandmarkEls.add(el);
    const role = getLandmarkRectRole(el);
    if (!role) continue;
    const bbox = getPageBbox(el);
    if (!bbox) continue;
    const count = (roleCount.get(role) ?? 0) + 1;
    roleCount.set(role, count);
    const path = count === 1 ? role : `${role}[${count}]`;
    landmarkRects.push({
      path,
      role,
      heading: getFirstHeadingInside(el),
      bbox,
    });
  }

  // Pass 2: direct children of main that are section, or div-with-id.
  const mainEl = document.querySelector("main") ?? document.querySelector("[role=main]");
  if (mainEl) {
    let sectionIdx = 0;
    const mainChildren = mainEl.children;
    for (let i = 0; i < mainChildren.length; i++) {
      if (landmarkRects.length >= 64) break;
      const child = mainChildren[i];
      if (!child) continue;
      const childTag = child.tagName.toLowerCase();
      const isSection = childTag === "section";
      const isDivWithId = childTag === "div" && child.id !== "";
      if (!isSection && !isDivWithId) continue;
      sectionIdx++;
      const bbox = getPageBbox(child);
      if (!bbox) continue;
      landmarkRects.push({
        path: `main › section[${sectionIdx}]`,
        role: "region",
        heading: getFirstHeadingInside(child),
        bbox,
      });
    }
  }

  // ── Build computed-style candidate set (§4.4) ──────────────────────────────
  //
  // Candidates = every SemanticNode element + each node's ancestor chain up to
  // and including its nearest landmark element (body fallback), deduped.
  // Budget = 2000 total entries; on overflow drop deepest-ancestor entries first
  // (tie-break: later document order first), never drop SemanticNode entries.
  //
  // Step 1: collect all ancestor elements across all node chains.
  //         If an ancestor IS a node element, reuse its node id.
  //         True ancestors get temporary placeholder ids resolved in step 3.

  // Set of elements that are SemanticNode elements (for dedup check)
  // We already have elementToNodeId for O(1) lookup.

  // For each node, walk parentElement up to landmark (or body), collecting
  // ancestor elements (excluding the node itself). Dedup by element identity.

  // Map from ancestor element -> { element, depth, docOrder } (true ancestors only)
  // "depth" = distance from documentElement (root), used for budget drop order.
  const trueAncestorElements = new Map<Element, { depth: number; docOrderIndex: number }>();

  // chains (before anc_N assignment): Map<nodeId, ancestor Element[]> nearest->furthest
  const rawChains = new Map<string, Element[]>();

  // We need document order index for true ancestors. We'll assign these after
  // collecting all ancestors by doing a final document-order walk over them.

  // Collect all unique ancestor elements across all nodes:
  for (let ni = 0; ni < nodes.length; ni++) {
    const node = nodes[ni]!;
    const nodeEl = elementToNodeId;
    // Find the element that maps to this node id
    // We need the element itself. Since we stored nodeId->element in elementToNodeId,
    // we need to reverse-look it up. Let's rebuild element lookup.
    // Actually, elementToNodeId maps element -> nodeId, so we need the inverse.
    // We'll build an inverse map in the next step. For now, collect all node elements.
    void nodeEl; // suppress lint
  }

  // Build nodeId -> element inverse map
  const nodeIdToElement = new Map<string, Element>();
  elementToNodeId.forEach((id, el) => {
    nodeIdToElement.set(id, el);
  });

  // For each node, collect ancestor chain
  for (let ni = 0; ni < nodes.length; ni++) {
    const node = nodes[ni]!;
    const el = nodeIdToElement.get(node.id);
    if (!el) continue;

    // Nearest landmark element for this node (boundary)
    const landmarkBoundary = getNearestLandmarkElement(el);
    const boundary = landmarkBoundary ?? document.body;

    const chain: Element[] = [];

    let ancestor: Element | null = el.parentElement;
    while (ancestor && ancestor !== document.documentElement) {
      // Include boundary (landmark or body) itself
      const isNodeEl = elementToNodeId.has(ancestor);
      if (isNodeEl) {
        // It's a SemanticNode element — reuse its node id, add to chain
        chain.push(ancestor);
      } else {
        // True ancestor — add to chain and to trueAncestorElements
        chain.push(ancestor);
        if (!trueAncestorElements.has(ancestor)) {
          // Compute depth (distance from document root)
          let depth = 0;
          let cur: Element | null = ancestor;
          while (cur && cur !== document.documentElement) {
            depth++;
            cur = cur.parentElement;
          }
          // docOrderIndex will be assigned later during document-order walk
          trueAncestorElements.set(ancestor, { depth, docOrderIndex: -1 });
        }
      }
      if (ancestor === boundary) break;
      ancestor = ancestor.parentElement;
    }

    rawChains.set(node.id, chain);
  }

  // Assign docOrderIndex to all true ancestor elements using a document-order walk.
  // We do a single TreeWalker pass over the body to get stable document ordering.
  let docOrderCounter = 0;
  const ancWalker = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT);
  let ancCurrent: Node | null = ancWalker.currentNode;
  while (ancCurrent) {
    const el = ancCurrent as Element;
    if (trueAncestorElements.has(el)) {
      const info = trueAncestorElements.get(el)!;
      info.docOrderIndex = docOrderCounter++;
    }
    ancCurrent = ancWalker.nextNode();
  }

  // Assign anc_N ids to true ancestors in document order (ascending docOrderIndex).
  // Sort entries by docOrderIndex ascending.
  const sortedTrueAncestors: Array<{ el: Element; depth: number; docOrderIndex: number }> = [];
  trueAncestorElements.forEach((info, el) => {
    sortedTrueAncestors.push({ el, depth: info.depth, docOrderIndex: info.docOrderIndex });
  });
  sortedTrueAncestors.sort((a, b) => a.docOrderIndex - b.docOrderIndex);

  // Assign anc_N ids
  const trueAncestorToId = new Map<Element, string>();
  for (let i = 0; i < sortedTrueAncestors.length; i++) {
    const entry = sortedTrueAncestors[i]!;
    trueAncestorToId.set(entry.el, `anc_${i}`);
  }

  // Total candidate count = nodes.length + sortedTrueAncestors.length
  const STYLE_BUDGET = 2000;
  const totalCandidates = nodes.length + sortedTrueAncestors.length;
  let truncated = false;
  let droppedCount = 0;

  // Determine which ancestor elements to drop on budget overflow.
  // Drop deepest-DOM-depth first, tie-break: later document order first (higher docOrderIndex).
  const droppedAncestorIds = new Set<string>();

  if (totalCandidates > STYLE_BUDGET) {
    const excess = totalCandidates - STYLE_BUDGET;
    truncated = true;
    droppedCount = excess;

    // Sort candidates to drop: deepest depth first, then higher docOrderIndex first (later doc order)
    const dropCandidates = sortedTrueAncestors.slice();
    dropCandidates.sort((a, b) => {
      if (b.depth !== a.depth) return b.depth - a.depth;
      // Tie-break: later document order first (higher docOrderIndex)
      return b.docOrderIndex - a.docOrderIndex;
    });

    for (let i = 0; i < excess && i < dropCandidates.length; i++) {
      const entry = dropCandidates[i]!;
      const id = trueAncestorToId.get(entry.el);
      if (id) droppedAncestorIds.add(id);
    }
  }

  // Build computedStyles: collect styles for all non-dropped candidates
  const computedStyles: Record<string, Record<string, string>> = {};

  // Node entries (never dropped)
  for (let ni = 0; ni < nodes.length; ni++) {
    const node = nodes[ni]!;
    const el = nodeIdToElement.get(node.id);
    if (el) {
      computedStyles[node.id] = readComputedStyle(el);
    }
  }

  // Ancestor entries (drop those in droppedAncestorIds)
  for (let i = 0; i < sortedTrueAncestors.length; i++) {
    const entry = sortedTrueAncestors[i]!;
    const id = trueAncestorToId.get(entry.el)!;
    if (!droppedAncestorIds.has(id)) {
      computedStyles[id] = readComputedStyle(entry.el);
    }
  }

  // ── Build ancestor descriptors ─────────────────────────────────────────────
  //
  // For ancestors that are NOT dropped, build the full AncestorDescriptor.
  // ordinalInLandmark = 1-based document-order index among ancestor candidates
  // sharing the same landmark.

  // For nearestHeading of an ancestor: text of first heading contained WITHIN it
  // in document order; if none, fall back to same preceding-heading rule nodes use.
  function getAncestorNearestHeading(
    ancEl: Element,
    _fallbackHeadingText: string | null
  ): string | null {
    // First: find first heading element contained within this ancestor
    const headingsInside = ancEl.querySelectorAll("h1,h2,h3,h4,h5,h6");
    for (let i = 0; i < headingsInside.length; i++) {
      const h = headingsInside[i];
      if (h && isVisible(h)) {
        const raw = (h as HTMLElement).innerText ?? "";
        const normalized = normalizeStr(raw, maxTextLength);
        if (normalized) return normalized;
      }
    }
    // Fallback: use the preceding-heading rule (same as nodes)
    // Since we walk in document order, _fallbackHeadingText is the lastHeadingText
    // at the point we reach this ancestor. However, we're building ancestors
    // after the node walk, so we don't have per-ancestor "preceding" heading.
    // We'll compute it using a different approach: find the last heading in document
    // order before this ancestor (using compareDocumentPosition).
    // For simplicity and correctness, we do a quick scan of headings on the page
    // that precede this element.
    let bestHeadingText: string | null = null;
    const allHeadings = document.querySelectorAll("h1,h2,h3,h4,h5,h6");
    for (let i = 0; i < allHeadings.length; i++) {
      const h = allHeadings[i]!;
      // Check if h comes before ancEl in document order
      const pos = ancEl.compareDocumentPosition(h);
      if (pos & Node.DOCUMENT_POSITION_PRECEDING) {
        // h is before ancEl; take the last such heading
        if (isVisible(h)) {
          const raw = (h as HTMLElement).innerText ?? "";
          const normalized = normalizeStr(raw, maxTextLength);
          if (normalized) bestHeadingText = normalized;
        }
      }
    }
    return bestHeadingText;
  }

  // ── Pre-compute text-bearing node elements (for ancestor text inheritance) ─
  // A "text-bearing semantic node" is any node in `nodes` whose anchors.text
  // (equivalently .text) is a non-empty string. We collect their elements now
  // so the per-ancestor containment count is O(ancestors × textNodes).
  const textBearingNodeElements: Array<{ el: Element; text: string }> = [];
  for (let ni = 0; ni < nodes.length; ni++) {
    const node = nodes[ni]!;
    if (node.anchors.text) {
      const el = nodeIdToElement.get(node.id);
      if (el) {
        textBearingNodeElements.push({ el, text: node.anchors.text });
      }
    }
  }

  // Build ordinalInLandmark counters for ancestors
  // Key: landmark name, Value: count of ancestors seen so far in that landmark
  const ancestorOrdinalCounters = new Map<string | null, number>();

  // We need to process ancestors in document order for ordinal assignment
  const ancestorDescriptors: RawAncestorDescriptor[] = [];

  for (let i = 0; i < sortedTrueAncestors.length; i++) {
    const entry = sortedTrueAncestors[i]!;
    const id = trueAncestorToId.get(entry.el)!;
    if (droppedAncestorIds.has(id)) continue;

    const el = entry.el;
    const bbox = getPageBbox(el);
    const bboxVal: [number, number, number, number] = bbox ?? [0, 0, 0, 0];
    const tag = el.tagName.toLowerCase();
    const landmarkEl = getNearestLandmarkElement(el);
    // If el IS a landmark, its own landmark role is its landmark
    const selfLandmark = getLandmarkRole(el);
    const landmark = selfLandmark ?? getNearestLandmark(el);

    // ordinalInLandmark: 1-based index among ancestor candidates in same landmark
    const ordinalKey = landmark ?? "__none__";
    const prevOrdinal = ancestorOrdinalCounters.get(ordinalKey) ?? 0;
    const ordinalInLandmark = prevOrdinal + 1;
    ancestorOrdinalCounters.set(ordinalKey, ordinalInLandmark);

    const cssSelector = buildSelector(el, landmarkEl);
    const nearestHeading = getAncestorNearestHeading(el, null);

    // Ancestor anchor-text inheritance (§4b item 2):
    // Count text-bearing semantic nodes whose element is contained within this
    // ancestor (strict: el.contains(nodeEl) && el !== nodeEl).
    // If exactly one, inherit its text; zero or >1 → null.
    let inheritedText: string | null = null;
    let containedCount = 0;
    let lastContainedText: string | null = null;
    for (let ti = 0; ti < textBearingNodeElements.length; ti++) {
      const entry2 = textBearingNodeElements[ti]!;
      if (el.contains(entry2.el) && el !== entry2.el) {
        containedCount++;
        lastContainedText = entry2.text;
        if (containedCount > 1) break; // early exit: >1 already decided
      }
    }
    if (containedCount === 1) {
      inheritedText = lastContainedText;
    }

    ancestorDescriptors.push({
      id,
      tag,
      bbox: bboxVal,
      depth: entry.depth,
      cssSelector,
      anchors: {
        text: inheritedText,
        role: null,
        href: null,
        alt: null,
        ariaLabel: null,
        nearestHeading,
        landmark,
        ordinalInLandmark,
      },
    });
  }

  // ── Build chains (nodeId -> id[]) nearest->furthest ────────────────────────
  // Chains reference node ids (for SemanticNode ancestors) and anc_N ids.
  // Drop entries for dropped ancestors from chains.

  const chains: Record<string, string[]> = {};
  // Build chains in node-id order (nodes are already in seqIndex/document order)
  for (let ni = 0; ni < nodes.length; ni++) {
    const node = nodes[ni]!;
    const rawChain = rawChains.get(node.id) ?? [];
    const resolvedChain: string[] = [];
    for (let ci = 0; ci < rawChain.length; ci++) {
      const ancEl = rawChain[ci]!;
      const nodeId = elementToNodeId.get(ancEl);
      if (nodeId) {
        // Ancestor is a SemanticNode element — use its node id
        resolvedChain.push(nodeId);
      } else {
        const ancId = trueAncestorToId.get(ancEl);
        if (ancId && !droppedAncestorIds.has(ancId)) {
          resolvedChain.push(ancId);
        }
        // else: dropped, skip
      }
    }
    if (resolvedChain.length > 0) {
      chains[node.id] = resolvedChain;
    }
  }

  const styleCandidates: RawStyleCandidates = {
    ancestors: ancestorDescriptors,
    chains,
    budget: STYLE_BUDGET,
    truncated,
    droppedCount,
  };

  return {
    nodes,
    pageHeight: document.documentElement.scrollHeight,
    landmarks,
    landmarkRects,
    computedStyles,
    styleCandidates,
  };
}
