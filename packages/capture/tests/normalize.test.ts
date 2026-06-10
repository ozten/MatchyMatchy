import { describe, it, expect } from "vitest";
import { normalizeText, redactUrl } from "../src/normalize.js";

describe("normalizeText", () => {
  it("replaces NBSP with regular space", () => {
    const input = "Hello World";
    expect(normalizeText(input)).toBe("Hello World");
  });

  it("collapses multiple whitespace chars to single space", () => {
    expect(normalizeText("Hello   World")).toBe("Hello World");
    expect(normalizeText("Hello\t\tWorld")).toBe("Hello World");
    expect(normalizeText("Hello\n\nWorld")).toBe("Hello World");
  });

  it("trims leading and trailing whitespace", () => {
    expect(normalizeText("  Hello  ")).toBe("Hello");
    expect(normalizeText("\n\tHello\n\t")).toBe("Hello");
  });

  it("strips C0 control chars (except whitespace for collapse)", () => {
    // \x01-\x08, \x0b, \x0c, \x0e-\x1f
    expect(normalizeText("Hel\x01lo")).toBe("Hello");
    expect(normalizeText("Hel\x08lo")).toBe("Hello");
    expect(normalizeText("Hel\x1flo")).toBe("Hello");
    // \x7f DEL
    expect(normalizeText("Hel\x7flo")).toBe("Hello");
  });

  it("strips C1 control chars (0x80-0x9F)", () => {
    expect(normalizeText("Hel\x80lo")).toBe("Hello");
    expect(normalizeText("Hel\x9flo")).toBe("Hello");
  });

  it("caps text at 500 chars by default", () => {
    const long = "a".repeat(600);
    const result = normalizeText(long);
    expect(result.length).toBe(500);
  });

  it("caps text at custom maxLength", () => {
    const long = "hello world extra";
    expect(normalizeText(long, 5)).toBe("hello");
  });

  it("returns empty string unchanged", () => {
    expect(normalizeText("")).toBe("");
  });

  it("handles multiple NBSP characters", () => {
    const input = "A B C";
    expect(normalizeText(input)).toBe("A B C");
  });

  it("collapses NBSP followed by space", () => {
    const input = "A  B";
    expect(normalizeText(input)).toBe("A B");
  });
});

describe("redactUrl", () => {
  const SENSITIVE_PARAMS = [
    "token",
    "sig",
    "signature",
    "key",
    "auth",
    "apikey",
    "access_token",
  ];

  it("replaces sensitive query params with redacted marker", () => {
    const url = "https://example.com/path?token=secret123&other=value";
    const result = redactUrl(url, SENSITIVE_PARAMS);
    expect(result).toContain("token=");
    expect(result).toContain("…redacted…");
    expect(result).not.toContain("secret123");
    expect(result).toContain("other=value");
  });

  it("is case-insensitive on parameter names", () => {
    const url = "https://example.com/path?TOKEN=secret&Key=value&Api_Key=other";
    const result = redactUrl(url, ["token", "key"]);
    expect(result).not.toContain("secret");
    expect(result).not.toContain("value");
    // api_key not in list, should be preserved
    expect(result).toContain("Api_Key=other");
  });

  it("redacts multiple sensitive params", () => {
    const url = "https://example.com/?token=t1&sig=s1&other=keep";
    const result = redactUrl(url, SENSITIVE_PARAMS);
    expect(result).not.toContain("t1");
    expect(result).not.toContain("s1");
    expect(result).toContain("other=keep");
  });

  it("leaves URLs without sensitive params unchanged", () => {
    const url = "https://example.com/path?foo=bar&baz=qux";
    expect(redactUrl(url, SENSITIVE_PARAMS)).toBe(url);
  });

  it("handles URLs with no query string", () => {
    const url = "https://example.com/path";
    expect(redactUrl(url, SENSITIVE_PARAMS)).toBe(url);
  });

  it("handles empty redactParams list", () => {
    const url = "https://example.com/?token=secret";
    expect(redactUrl(url, [])).toBe(url);
  });

  it("does not include Authorization header value in output (header redaction is at recording level)", () => {
    // Authorization headers are never recorded in network.requests
    // This test documents that redactUrl is for URL params, not headers
    const url = "https://example.com/api?auth=bearer_token";
    const result = redactUrl(url, SENSITIVE_PARAMS);
    expect(result).not.toContain("bearer_token");
  });

  it("redacts access_token param", () => {
    const url = "https://example.com/?access_token=mytoken123";
    const result = redactUrl(url, SENSITIVE_PARAMS);
    expect(result).not.toContain("mytoken123");
    expect(result).toContain("…redacted…");
  });
});
