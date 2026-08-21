/**
 * Port-parity U9: ::before/::after pseudo-element capture tests.
 *
 * Launches a real headless Chromium (via the same launchBrowser/createContext
 * helpers capture.ts uses) against small local fixture pages built with
 * page.setContent(), matching the project's existing browser-test pattern
 * (see tests/hit-test.test.ts).
 */
import { describe, it, expect, beforeAll, afterAll, beforeEach, afterEach } from "vitest";
import type { Browser, BrowserContext, Page } from "playwright";
import { launchBrowser, createContext } from "../src/browser-runner.js";
import { extractPageModel } from "../src/extract/page-model.js";
import { CaptureBundleSchema } from "../src/schema.js";

const BROWSER_TIMEOUT_MS = 30_000;

describe("pseudo-element capture (browser)", () => {
  let browser: Browser;
  let context: BrowserContext;
  let page: Page;

  beforeAll(async () => {
    browser = await launchBrowser();
  }, BROWSER_TIMEOUT_MS);

  afterAll(async () => {
    await browser?.close();
  }, BROWSER_TIMEOUT_MS);

  beforeEach(async () => {
    context = await createContext(browser, { name: "desktop", width: 800, height: 600, dsf: 1 });
    page = await context.newPage();
  });

  afterEach(async () => {
    await context?.close();
  });

  // ── (a) decorative leaf div with painted ::before tick → tier "selector" ──
  it("captures a decorative [data-hr-corner-top]::before tick as tier 'selector'", async () => {
    await page.setContent(`
      <html><body style="margin:0">
        <style>
          [data-hr-corner-top]::before {
            content: "";
            display: block;
            width: 10px;
            height: 10px;
            background: red;
          }
        </style>
        <div data-hr-corner-top="tick"></div>
      </body></html>
    `);

    const result = await page.evaluate(extractPageModel, 500);
    const entries = Object.values(result.pseudoElements);
    expect(entries).toHaveLength(1);
    const entry = entries[0]!;
    expect(entry.ownerTier).toBe("selector");
    expect(entry.ownerSelector).toBeDefined();
    expect(entry.ownerSelector).toContain("data-hr-corner-top");
    expect(entry.before).toBeDefined();
    expect(entry.before?.content).toBe(`""`);
    expect(entry.before?.width).toBe("10px");
    expect(entry.before?.height).toBe("10px");
  });

  // ── (b) semantic node (link) with ::before icon → tier "node" ────────────
  it("captures a semantic node's ::before icon as tier 'node' keyed by the node's id", async () => {
    await page.setContent(`
      <html><body style="margin:0">
        <style>
          #navlink::before {
            content: "\\00BB";
            display: inline-block;
          }
        </style>
        <a id="navlink" href="/home">Home</a>
      </body></html>
    `);

    const result = await page.evaluate(extractPageModel, 500);
    const linkNode = result.nodes.find((n) => n.href === "http://localhost/home" || n.text === "Home");
    expect(linkNode).toBeDefined();

    const entry = result.pseudoElements[linkNode!.id];
    expect(entry).toBeDefined();
    expect(entry?.ownerTier).toBe("node");
    expect(entry?.ownerNodeId).toBe(linkNode!.id);
    expect(entry?.before).toBeDefined();
  });

  // ── (c) content: none / normal not captured; empty-string content captured ──
  it("does not capture content:none or content:normal, but does capture an empty-string content with a visible box", async () => {
    await page.setContent(`
      <html><body style="margin:0">
        <style>
          #none::before { content: none; display: block; width: 10px; height: 10px; background: blue; }
          #normal::before { display: block; width: 10px; height: 10px; background: green; }
          #empty::before { content: ""; display: block; width: 10px; height: 10px; background: purple; }
        </style>
        <div id="none"></div>
        <div id="normal"></div>
        <div id="empty"></div>
      </body></html>
    `);

    const result = await page.evaluate(extractPageModel, 500);
    const entries = Object.values(result.pseudoElements);
    expect(entries).toHaveLength(1);
    expect(entries[0]?.ownerSelector).toBe("#empty");
    expect(entries[0]?.before?.content).toBe(`""`);
  });

  // ── (d) budget: 300 painted pseudos → 250 kept, 50 dropped, deterministic ──
  it("caps at 250 entries with deterministic drop order and byte-identical output across two runs", async () => {
    const divs = Array.from({ length: 300 }, (_, i) => `<div class="p"></div>`).join("");
    await page.setContent(`
      <html><body style="margin:0">
        <style>
          .p { display: block; height: 3px; }
          .p::before { content: "x"; display: block; width: 2px; height: 2px; }
        </style>
        ${divs}
      </body></html>
    `);

    const run1 = await page.evaluate(extractPageModel, 500);
    const run2 = await page.evaluate(extractPageModel, 500);

    expect(Object.keys(run1.pseudoElements)).toHaveLength(250);
    expect(run1.pseudoTruncated).toEqual({ droppedCount: 50 });
    expect(JSON.stringify(run1.pseudoElements)).toBe(JSON.stringify(run2.pseudoElements));
    expect(JSON.stringify(run1.pseudoTruncated)).toBe(JSON.stringify(run2.pseudoTruncated));
  });

  // ── (e) static position, no resolvable offsets → entry present, bbox absent ──
  it("captures a statically-positioned pseudo without a bbox", async () => {
    await page.setContent(`
      <html><body style="margin:0">
        <style>
          #stat::before {
            content: "*";
            display: inline-block;
          }
        </style>
        <div id="stat">text</div>
      </body></html>
    `);

    const result = await page.evaluate(extractPageModel, 500);
    const entries = Object.values(result.pseudoElements);
    expect(entries).toHaveLength(1);
    expect(entries[0]?.before?.position).toBe("static");
    expect(entries[0]?.before?.bbox).toBeUndefined();
  });

  // ── (f) visibility:hidden subtree with painted ::before → not captured ───
  it("excludes a painted pseudo inside a visibility:hidden subtree (hideSelectors simulation)", async () => {
    await page.setContent(`
      <html><body style="margin:0">
        <style>
          [data-hidden-owner]::before { content: "y"; display: block; width: 5px; height: 5px; }
        </style>
        <div style="visibility:hidden">
          <div data-hidden-owner="x"></div>
        </div>
      </body></html>
    `);

    const result = await page.evaluate(extractPageModel, 500);
    expect(Object.keys(result.pseudoElements)).toHaveLength(0);
  });

  // ── (g) oversized/control-char id & data-* values → capped and stripped ──
  it("caps and strips an oversized, control-character-laden data-* attribute value before embedding it in ownerSelector", async () => {
    const controlChars = "\x01\x02";
    const longValue = controlChars + "B".repeat(30);
    await page.setContent(`
      <html><body style="margin:0">
        <style>
          [data-hr-marker]::before { content: "z"; display: block; width: 4px; height: 4px; }
        </style>
        <div data-hr-marker="${longValue}"></div>
      </body></html>
    `);

    // maxTextLength = 20: caps the (control-char-stripped) 30 'B' run to 20.
    const result = await page.evaluate(extractPageModel, 20);
    const entries = Object.values(result.pseudoElements);
    expect(entries).toHaveLength(1);
    const expectedValue = "B".repeat(20);
    expect(entries[0]?.ownerSelector).toBe(`[data-hr-marker="${expectedValue}"]`);
    // No control characters or truncated-away characters survive.
    expect(entries[0]!.ownerSelector).not.toContain("\x01");
    expect(entries[0]!.ownerSelector).not.toContain("\x02");
  });

  // ── (h) bundle with pseudoElements validates against the zod schema ──────
  it("produces a pseudoElements map that validates against the updated CaptureBundleSchema", async () => {
    await page.setContent(`
      <html><body style="margin:0">
        <style>
          [data-hr-corner-top]::before {
            content: "";
            display: block;
            width: 10px;
            height: 10px;
            background: red;
          }
        </style>
        <div data-hr-corner-top="tick"></div>
      </body></html>
    `);

    const result = await page.evaluate(extractPageModel, 500);

    const bundle = {
      schemaVersion: "1.1" as const,
      capturedAt: "2026-01-01T00:00:00.000Z",
      viewport: { name: "desktop", width: 1440, height: 1000, dsf: 1 },
      environment: { os: "linux", chromiumBuild: "Chromium/130.0.0.0", playwright: "1.60.0", dsf: 1 },
      determinism: {
        animationsDisabled: "ran" as const,
        reducedMotion: "ran" as const,
        timeFrozen: "ran" as const,
        randomStubbed: "ran" as const,
        fontsReady: "ran" as const,
        imagesDecoded: "ran" as const,
        lazyLoadPass: "ran" as const,
        settled: "ran" as const,
        clicked: [],
        hidden: [],
        masked: [],
        retriedWithoutTimeFreeze: false,
      },
      page: {
        url: "http://localhost:3000/",
        finalUrl: "http://localhost:3000/",
        redirectChain: [],
        statusCode: 200,
        title: "Test",
        metaDescription: "",
        canonical: null,
        lang: "en",
        pageHeight: 600,
        nodes: [],
        landmarks: [],
        network: { requests: [] },
        console: [],
        a11y: { violations: [] },
        linkProbes: [],
      },
      computedStyles: {},
      styleCandidates: { ancestors: [], chains: {}, budget: 2000, truncated: false, droppedCount: 0 },
      screenshots: { fullPage: "desktop/old.png", viewport: "desktop/old-vp.png" },
      pseudoElements: result.pseudoElements,
    };

    const parsed = CaptureBundleSchema.safeParse(bundle);
    expect(parsed.success).toBe(true);
  });
});
