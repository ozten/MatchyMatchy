import { describe, it, expect } from "vitest";
import { normalizeText, redactUrl, DEFAULT_REDACT_PARAMS } from "../src/normalize.js";

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

// ---------------------------------------------------------------------------
// DEFAULT_REDACT_PARAMS — gate symmetry (R5 redaction-hygiene)
// These tests verify capture ⊇ gate: every param in pair_privacy.py
// SECRET_NAMES is redacted by DEFAULT_REDACT_PARAMS before bundle write.
// ---------------------------------------------------------------------------
describe("DEFAULT_REDACT_PARAMS gate symmetry", () => {
  it("redacts api_key (Weglot and similar API key query params)", () => {
    const url = "https://cdn.weglot.com/widget?api_key=wg_secret123&other=keep";
    const result = redactUrl(url, DEFAULT_REDACT_PARAMS);
    expect(result).not.toContain("wg_secret123");
    expect(result).toContain("api_key=…redacted…");
    expect(result).toContain("other=keep");
  });

  it("redacts sid (Google Analytics / Bing / LeadFeeder session params)", () => {
    const url = "https://www.google-analytics.com/collect?v=1&sid=abc123xyz&t=event";
    const result = redactUrl(url, DEFAULT_REDACT_PARAMS);
    expect(result).not.toContain("abc123xyz");
    expect(result).toContain("sid=…redacted…");
    expect(result).toContain("t=event");
  });

  it("redacts api_key and sid together while leaving non-sensitive params", () => {
    const url = "https://x.com/a?api_key=wg_secret&sid=abc&foo=bar";
    const result = redactUrl(url, DEFAULT_REDACT_PARAMS);
    expect(result).not.toContain("wg_secret");
    expect(result).not.toContain("=abc");
    expect(result).toContain("api_key=…redacted…");
    expect(result).toContain("sid=…redacted…");
    expect(result).toContain("foo=bar");
  });

  it("redacts password / passwd / pwd", () => {
    for (const param of ["password", "passwd", "pwd"]) {
      const url = `https://example.com/login?${param}=hunter2`;
      const result = redactUrl(url, DEFAULT_REDACT_PARAMS);
      expect(result, `${param} should be redacted`).not.toContain("hunter2");
      expect(result).toContain(`${param}=…redacted…`);
    }
  });

  it("redacts secret and client_secret", () => {
    const url = "https://auth.example.com/token?client_secret=cs_xyz&secret=s_abc";
    const result = redactUrl(url, DEFAULT_REDACT_PARAMS);
    expect(result).not.toContain("cs_xyz");
    expect(result).not.toContain("s_abc");
    expect(result).toContain("client_secret=…redacted…");
    expect(result).toContain("secret=…redacted…");
  });

  it("redacts bearer and jwt", () => {
    const url = "https://api.example.com/v1?bearer=tok_abc&jwt=eyJhbGc";
    const result = redactUrl(url, DEFAULT_REDACT_PARAMS);
    expect(result).not.toContain("tok_abc");
    expect(result).not.toContain("eyJhbGc");
    expect(result).toContain("bearer=…redacted…");
    expect(result).toContain("jwt=…redacted…");
  });

  it("redacts session / sessionid", () => {
    const url = "https://example.com/app?session=sess_xyz&sessionid=sid_abc";
    const result = redactUrl(url, DEFAULT_REDACT_PARAMS);
    expect(result).not.toContain("sess_xyz");
    expect(result).not.toContain("sid_abc");
    expect(result).toContain("session=…redacted…");
    expect(result).toContain("sessionid=…redacted…");
  });

  it("contains all gate secret names", () => {
    // Ensure the default list covers every name in pair_privacy.py SECRET_NAMES
    const gateSecretNames = new Set([
      "token", "sig", "signature", "key", "auth", "apikey", "access_token",
      "password", "passwd", "pwd", "secret", "client_secret",
      "bearer", "jwt", "session", "sessionid", "sid", "api_key",
    ]);
    const defaultSet = new Set(DEFAULT_REDACT_PARAMS.map((p) => p.toLowerCase()));
    for (const name of gateSecretNames) {
      expect(defaultSet.has(name), `DEFAULT_REDACT_PARAMS missing gate secret: ${name}`).toBe(true);
    }
  });
});
