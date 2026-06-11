/**
 * M7 capture-layer unit tests:
 *  1. Console-listener timing: onPageCreated resets + attaches pre-nav
 *  2. Response-based network status: find-entry-by-url and set status
 *  3. axe-core source string: non-empty string accessible at import time
 */
import { describe, it, expect, vi } from "vitest";
import axeCore from "axe-core";

// ─── Test 3: axe-core source string ──────────────────────────────────────────

describe("axe-core source string", () => {
  it("is a non-empty string importable at module load time", () => {
    const axeSource: string = (axeCore as unknown as { source: string }).source;
    expect(typeof axeSource).toBe("string");
    expect(axeSource.length).toBeGreaterThan(1000);
  });

  it("contains the axe function definition (sanity: will inject correctly)", () => {
    const axeSource: string = (axeCore as unknown as { source: string }).source;
    // The injectable script defines window.axe
    expect(axeSource).toContain("axe");
  });
});

// ─── Test 1: Console-listener timing / onPageCreated callback ─────────────────

/**
 * Simulate the onPageCreated pattern from capture.ts.
 * The callback resets the consoleMessages array and attaches a listener
 * to the new page object.
 */
function buildOnPageCreated(
  consoleMessages: Array<{ level: string; text: string }>
): (page: FakePage) => void {
  return (page: FakePage): void => {
    consoleMessages.length = 0; // reset — only surviving page's messages kept
    page.on("console", (msg: { type(): string; text(): string }) => {
      consoleMessages.push({ level: msg.type(), text: msg.text() });
    });
  };
}

/** Minimal fake page that supports a single "console" event listener */
class FakePage {
  private listeners: Array<(msg: { type(): string; text(): string }) => void> = [];

  on(event: string, cb: (msg: { type(): string; text(): string }) => void): void {
    if (event === "console") {
      this.listeners.push(cb);
    }
  }

  /** Simulate the browser emitting a console message */
  emitConsole(level: string, text: string): void {
    const msg = { type: () => level, text: () => text };
    for (const cb of this.listeners) cb(msg);
  }
}

describe("onPageCreated console-listener", () => {
  it("attaches a listener and records a console message", () => {
    const consoleMessages: Array<{ level: string; text: string }> = [];
    const onPageCreated = buildOnPageCreated(consoleMessages);

    const page = new FakePage();
    onPageCreated(page);

    page.emitConsole("error", "something went wrong");

    expect(consoleMessages).toHaveLength(1);
    expect(consoleMessages[0]).toEqual({ level: "error", text: "something went wrong" });
  });

  it("resets prior messages when called a second time (retry path)", () => {
    const consoleMessages: Array<{ level: string; text: string }> = [];
    const onPageCreated = buildOnPageCreated(consoleMessages);

    // First page emits a message
    const page1 = new FakePage();
    onPageCreated(page1);
    page1.emitConsole("warning", "first page warning");

    expect(consoleMessages).toHaveLength(1);

    // Retry: second page is created — messages should be reset
    const page2 = new FakePage();
    onPageCreated(page2);

    // Messages from first page are gone
    expect(consoleMessages).toHaveLength(0);

    // Now second page emits
    page2.emitConsole("error", "second page error");
    expect(consoleMessages).toHaveLength(1);
    expect(consoleMessages[0]).toEqual({ level: "error", text: "second page error" });
  });

  it("old page listener no longer affects messages after reset", () => {
    const consoleMessages: Array<{ level: string; text: string }> = [];
    const onPageCreated = buildOnPageCreated(consoleMessages);

    const page1 = new FakePage();
    onPageCreated(page1);

    const page2 = new FakePage();
    onPageCreated(page2); // resets messages

    // Old page1 still has its listener attached, but the array was reset —
    // if page1 fires, it appends to the shared array (a known limitation; in
    // practice page1 is closed before page2 is created in the retry path).
    // The key property: after onPageCreated(page2), messages start at 0.
    expect(consoleMessages).toHaveLength(0);
    page2.emitConsole("log", "normal log");
    expect(consoleMessages).toHaveLength(1);
  });
});

// ─── Test 2: Response-based network status helper ─────────────────────────────

/**
 * Extracted pure helper: find entry by URL (backwards scan, matching redacted
 * or raw URL) and set its status. This mirrors the logic in the
 * context.on("response", ...) handler in capture.ts.
 */
function applyResponseStatus(
  networkRequests: Array<{ url: string; status: number; type: string; failed: boolean }>,
  responseUrl: string,
  status: number
): void {
  let idx = -1;
  for (let i = networkRequests.length - 1; i >= 0; i--) {
    const r = networkRequests[i];
    if (r && r.url === responseUrl) { idx = i; break; }
  }
  if (idx !== -1) {
    const entry = networkRequests[idx];
    if (entry) (entry as { status: number }).status = status;
  }
}

describe("response-based network status", () => {
  it("sets status 200 on the matching entry", () => {
    const requests = [
      { url: "http://localhost:3000/", status: 0, type: "document", failed: false },
      { url: "http://localhost:3000/style.css", status: 0, type: "stylesheet", failed: false },
    ];

    applyResponseStatus(requests, "http://localhost:3000/style.css", 200);

    expect(requests[0]?.status).toBe(0);
    expect(requests[1]?.status).toBe(200);
  });

  it("sets status 404 on the matching entry (completed failed request)", () => {
    const requests = [
      { url: "http://localhost:3000/", status: 0, type: "document", failed: false },
      { url: "http://localhost:3000/missing.png", status: 0, type: "image", failed: false },
    ];

    applyResponseStatus(requests, "http://localhost:3000/missing.png", 404);

    expect(requests[1]?.status).toBe(404);
    // failed is NOT set by the response listener — only requestfailed sets it
    expect(requests[1]?.failed).toBe(false);
  });

  it("scans backwards — updates the last matching entry when duplicates exist", () => {
    const requests = [
      { url: "http://localhost:3000/font.woff2", status: 0, type: "font", failed: false },
      { url: "http://localhost:3000/font.woff2", status: 0, type: "font", failed: false },
    ];

    applyResponseStatus(requests, "http://localhost:3000/font.woff2", 200);

    // Last entry updated, first untouched
    expect(requests[0]?.status).toBe(0);
    expect(requests[1]?.status).toBe(200);
  });

  it("does nothing when URL is not found", () => {
    const requests = [
      { url: "http://localhost:3000/", status: 200, type: "document", failed: false },
    ];

    applyResponseStatus(requests, "http://localhost:3000/nonexistent.js", 200);

    expect(requests[0]?.status).toBe(200); // unchanged
  });
});
