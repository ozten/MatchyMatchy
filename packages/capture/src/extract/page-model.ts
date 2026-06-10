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

export interface RawPageModelResult {
  nodes: RawSemanticNode[];
  pageHeight: number;
  landmarks: string[];
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
    let r = s.replace(/ /g, " ");
    // Strip C0 control chars (keep whitespace chars for collapse) and C1
    r = r.replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f\x80-\x9f]/g, "");
    // Collapse whitespace
    r = r.replace(/\s+/g, " ");
    r = r.trim();
    if (r.length > maxLen) r = r.slice(0, maxLen);
    return r;
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

  return {
    nodes,
    pageHeight: document.documentElement.scrollHeight,
    landmarks,
  };
}
