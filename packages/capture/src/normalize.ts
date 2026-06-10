/**
 * Pure text normalization and URL redaction helpers.
 * These are importable from both extraction (page.evaluate) and capture contexts.
 * Note: page.evaluate serializes functions, so functions used inside page.evaluate
 * must be self-contained or duplicated. These helpers are for the Node.js side.
 */

/**
 * Normalize a page-derived string:
 * - Replace NBSP (U+00A0) with regular space
 * - Collapse consecutive whitespace to single space
 * - Trim leading/trailing whitespace
 * - Strip C0 control chars (U+0000–U+001F except tab/newline used for whitespace collapse)
 *   and C1 control chars (U+0080–U+009F)
 * - Cap at maxLength characters
 */
export function normalizeText(text: string, maxLength = 500): string {
  if (!text) return text;
  // Replace NBSP with space
  let result = text.replace(/ /g, " ");
  // Strip C0 control chars (keep \t \n \r for whitespace collapse below)
  // and C1 control chars (0x80-0x9F)
  result = result.replace(/[\x00-\x08\x0B\x0C\x0E-\x1F\x7F\x80-\x9F]/g, "");
  // Collapse whitespace (tabs, newlines, spaces) to single space
  result = result.replace(/\s+/g, " ");
  // Trim
  result = result.trim();
  // Cap at maxLength
  if (result.length > maxLength) {
    result = result.slice(0, maxLength);
  }
  return result;
}

/**
 * Redact sensitive query parameters in a URL string.
 * Replaces the value of matching params with "…redacted…".
 * Matching is case-insensitive on parameter names.
 */
export function redactUrl(url: string, redactParams: string[]): string {
  if (!redactParams.length) return url;
  const lowerParams = new Set(redactParams.map((p) => p.toLowerCase()));

  // Use regex-based replacement to avoid URLSearchParams encoding the marker
  return url.replace(
    /([?&])([^=&#]+)=([^&#]*)/g,
    (match, sep: string, key: string, _value: string) => {
      if (lowerParams.has(key.toLowerCase())) {
        return `${sep}${key}=…redacted…`;
      }
      return match;
    }
  );
}

/**
 * Default sensitive query parameter names to redact.
 */
export const DEFAULT_REDACT_PARAMS = [
  "token",
  "sig",
  "signature",
  "key",
  "auth",
  "apikey",
  "access_token",
];
