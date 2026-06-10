/**
 * Pure helpers for computed-style capture.
 *
 * NOTE: page.evaluate cannot close over Node.js imports, so anything needed
 * inside the browser evaluation pass is DUPLICATED inline in page-model.ts.
 * Each duplication site has a comment pointing back here, and vice-versa.
 *
 * These exports are for use in the Node.js test layer only (unit tests for
 * property list and value capping).
 */

/**
 * The curated CSS property list read via getComputedStyle(el).getPropertyValue(p).
 *
 * NOTE: duplicated inline in extractPageModel() in page-model.ts — keep in sync.
 *
 * "background" shorthand is captured here but EXCLUDED from the diff property
 * list in analyze (covered by background-color/background-image; avoids double-
 * reporting). "border" shorthand stays in the diff list (no border longhands
 * captured).
 */
export const COMPUTED_STYLE_PROPS: readonly string[] = [
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
];

/**
 * Maximum character length for a computed CSS property value.
 *
 * NOTE: duplicated inline in extractPageModel() in page-model.ts — keep in sync.
 */
export const COMPUTED_STYLE_VALUE_MAX_LEN = 1000;

/**
 * Cap a computed CSS value at COMPUTED_STYLE_VALUE_MAX_LEN characters and
 * strip C0/C1 control characters (same pattern as the text sanitizer).
 *
 * NOTE: logic duplicated inline in extractPageModel() in page-model.ts — keep in sync.
 */
export function capStyleValue(value: string): string {
  // Strip C0 control chars (keep normal whitespace) and C1 control chars
  let result = value.replace(/[\x00-\x08\x0B\x0C\x0E-\x1F\x7F\x80-\x9F]/g, "");
  if (result.length > COMPUTED_STYLE_VALUE_MAX_LEN) {
    result = result.slice(0, COMPUTED_STYLE_VALUE_MAX_LEN);
  }
  return result;
}
