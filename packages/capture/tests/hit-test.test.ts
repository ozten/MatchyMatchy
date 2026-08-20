/**
 * Port-parity U6: capture-time clickable-area hit-test probe tests.
 *
 * Launches a real headless Chromium (via the same launchBrowser/createContext
 * helpers capture.ts uses) against small local fixture pages built with
 * page.setContent(), matching the project's existing browser-test pattern.
 * Chromium is resolved from PLAYWRIGHT_BROWSERS_PATH (repo-local
 * .pw-browsers), exported by the Makefile.
 */
import { describe, it, expect, beforeAll, afterAll, beforeEach, afterEach } from "vitest";
import type { Browser, BrowserContext, Page } from "playwright";
import { launchBrowser, createContext } from "../src/browser-runner.js";
import {
  runHitTestProbe,
  isProbeEligible,
  type HitTestProbeInput,
} from "../src/extract/hit-test.js";
import { extractPageModel } from "../src/extract/page-model.js";
import { CaptureBundleSchema } from "../src/schema.js";

const BROWSER_TIMEOUT_MS = 30_000;

describe("hit-test probe (browser)", () => {
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

  // ── (a) fully clickable button → 25/25 hit ───────────────────────────────
  it("records 25/25 hits for a fully clickable, unobstructed button", async () => {
    await page.setContent(`
      <html><body style="margin:0">
        <button id="btn" style="width:200px;height:50px;">Click me</button>
      </body></html>
    `);

    const nodes: HitTestProbeInput[] = [
      { id: "node_0", cssSelector: "#btn", bbox: [0, 0, 200, 50] },
    ];
    const result = await page.evaluate(runHitTestProbe, nodes);

    expect(result["node_0"]?.status).toBe("sampled");
    expect(result["node_0"]?.gridSize).toBe(5);
    const points = result["node_0"]?.points ?? [];
    expect(points).toHaveLength(25);
    expect(points.every((p) => p.o === "hit")).toBe(true);
  });

  // ── (b) sibling image occludes half the button → misses record the image's selector ──
  it("records misses with the occluding sibling image's selector", async () => {
    await page.setContent(`
      <html><body style="margin:0">
        <div style="position:relative;width:200px;height:50px;">
          <button id="btn2" style="width:200px;height:50px;margin:0;padding:0;">Click me</button>
          <img id="ov" style="position:absolute;top:0;left:0;width:90px;height:50px;z-index:2;">
        </div>
      </body></html>
    `);

    const nodes: HitTestProbeInput[] = [
      { id: "node_0", cssSelector: "#btn2", bbox: [0, 0, 200, 50] },
    ];
    const result = await page.evaluate(runHitTestProbe, nodes);

    const points = result["node_0"]?.points ?? [];
    expect(points).toHaveLength(25);

    // Grid columns (inset 2px, 200px wide button) land at x ≈ 2, 51, 100, 149, 198.
    // The image covers [0, 90), so the two leftmost columns (x=2, x=51) miss to
    // the image; the three rightmost columns (x=100, 149, 198) hit the button.
    const misses = points.filter((p) => p.o === "miss");
    const hits = points.filter((p) => p.o === "hit");
    expect(misses.length).toBe(10); // 2 columns x 5 rows
    expect(hits.length).toBe(15); // 3 columns x 5 rows
    for (const m of misses) {
      expect(m.winner).toBeDefined();
      expect(m.winner).toContain("img");
    }
  });

  // ── (c) pill-radius CTA → corners clipped, interior hits ────────────────
  it("records clipped at the four grid corners of a pill-radius CTA, hit elsewhere", async () => {
    await page.setContent(`
      <html><body style="margin:0">
        <div id="parent" style="position:relative;width:400px;height:400px;background:yellow;">
          <button id="cta" style="position:absolute;top:50px;left:50px;width:200px;height:50px;border-radius:999px;background:blue;border:none;color:white;">Click me</button>
        </div>
      </body></html>
    `);

    const nodes: HitTestProbeInput[] = [
      { id: "node_0", cssSelector: "#cta", bbox: [50, 50, 200, 50] },
    ];
    const result = await page.evaluate(runHitTestProbe, nodes);

    const points = result["node_0"]?.points ?? [];
    expect(points).toHaveLength(25);

    // For a 200x50 pill (border-radius clamps to the 25px half-height "stadium"
    // cap), the four extreme grid corners (row-major indices 0, 4, 20, 24) fall
    // outside the rounded cap and resolve to the parent — clipped. The three
    // middle columns (indices whose col is 1, 2 or 3) sit in the flat middle of
    // the pill for every row and always hit. No point may ever "miss" on a pill
    // shape (the winner is always either the button itself or its ancestor).
    const cornerIdx = [0, 4, 20, 24];
    for (const idx of cornerIdx) {
      expect(points[idx]?.o).toBe("clipped");
      expect(points[idx]?.winner).toBeUndefined();
    }
    const middleColumnIdx = points
      .map((_, i) => i)
      .filter((i) => [1, 2, 3].includes(i % 5));
    for (const idx of middleColumnIdx) {
      expect(points[idx]?.o).toBe("hit");
    }
    expect(points.some((p) => p.o === "miss")).toBe(false);
    // The exact outer-column rows affected by the cap curvature (beyond the
    // literal 4 corners) is a browser rasterization detail, not part of the
    // contract — assert only that at least the 4 corners are clipped and
    // everything else is hit or clipped (never a miss).
    for (const p of points) {
      expect(["hit", "clipped"]).toContain(p.o);
    }
  });

  // ── (d) 1px-tall skip-link → skipped(tooSmall) ──────────────────────────
  it("skips a 1px-tall skip-link as tooSmall", async () => {
    await page.setContent(`
      <html><body style="margin:0">
        <a id="skip" href="#main" style="position:absolute;height:1px;width:100px;overflow:hidden;">Skip to content</a>
      </body></html>
    `);

    const nodes: HitTestProbeInput[] = [
      { id: "node_0", cssSelector: "#skip", bbox: [0, 0, 100, 1] },
    ];
    const result = await page.evaluate(runHitTestProbe, nodes);

    expect(result["node_0"]).toEqual({ status: "skipped", skipReason: "tooSmall" });
  });

  // ── (e) label-wrapped input → points landing on the label count as hit ─
  it("counts points landing on the associated label as hits for the input node", async () => {
    await page.setContent(`
      <html><body style="margin:0">
        <label id="lbl" style="display:inline-block;margin:0;padding:0;">
          <input id="inp" type="text" style="pointer-events:none;width:200px;height:30px;display:block;margin:0;padding:0;border:1px solid #000;box-sizing:border-box;">
        </label>
      </body></html>
    `);

    const nodes: HitTestProbeInput[] = [
      { id: "node_0", cssSelector: "#inp", bbox: [0, 0, 200, 30] },
    ];
    const result = await page.evaluate(runHitTestProbe, nodes);

    expect(result["node_0"]?.status).toBe("sampled");
    const points = result["node_0"]?.points ?? [];
    expect(points).toHaveLength(25);
    // Every grid point resolves (via elementFromPoint) to the wrapping
    // <label>, whose .control is the input — the label-association rule
    // must count these as hits, not misses.
    expect(points.every((p) => p.o === "hit")).toBe(true);
  });

  // ── (f) determinism: two runs on the same page → byte-identical JSON ────
  it("produces byte-identical hitTests JSON across two runs on the same page", async () => {
    await page.setContent(`
      <html><body style="margin:0">
        <button id="btn3" style="width:150px;height:40px;">Submit</button>
        <a id="lnk" href="/about" style="display:block;width:120px;height:30px;">About</a>
      </body></html>
    `);

    const nodes: HitTestProbeInput[] = [
      { id: "node_0", cssSelector: "#btn3", bbox: [0, 0, 150, 40] },
      { id: "node_1", cssSelector: "#lnk", bbox: [0, 40, 120, 30] },
    ];

    const run1 = await page.evaluate(runHitTestProbe, nodes);
    const run2 = await page.evaluate(runHitTestProbe, nodes);

    expect(JSON.stringify(run1)).toBe(JSON.stringify(run2));
  });

  // ── (g) hasOnclick eligibility ───────────────────────────────────────────
  it("extracts hasOnclick and makes an onclick div probe-eligible; a plain div is not", async () => {
    await page.setContent(`
      <html><body style="margin:0">
        <div id="clickableDiv" onclick="void 0">Click this div</div>
        <div id="plainDiv">Not clickable</div>
      </body></html>
    `);

    const pageModel = await page.evaluate(extractPageModel, 500);
    const clickableNode = pageModel.nodes.find(
      (n) => n.text === "Click this div"
    );
    const plainNode = pageModel.nodes.find((n) => n.text === "Not clickable");

    expect(clickableNode).toBeDefined();
    expect(plainNode).toBeDefined();

    expect(clickableNode?.hasOnclick).toBe(true);
    expect(plainNode?.hasOnclick).toBeUndefined();

    expect(isProbeEligible(clickableNode!.kind, clickableNode!.hasOnclick)).toBe(true);
    expect(isProbeEligible(plainNode!.kind, plainNode!.hasOnclick)).toBe(false);
  });

  // ── bundle validation with hitTests present ─────────────────────────────
  it("produces a hitTests map that validates against the updated CaptureBundleSchema", async () => {
    await page.setContent(`
      <html><body style="margin:0">
        <button id="btn4" style="width:120px;height:40px;">OK</button>
      </body></html>
    `);

    const nodes: HitTestProbeInput[] = [
      { id: "node_0", cssSelector: "#btn4", bbox: [0, 0, 120, 40] },
    ];
    const probeResult = await page.evaluate(runHitTestProbe, nodes);

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
        hitTestProbe: "ran" as const,
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
        nodes: [
          {
            id: "node_0",
            kind: "button" as const,
            role: "button",
            text: "OK",
            accName: "OK",
            href: null,
            imageAlt: null,
            bbox: [0, 0, 120, 40] as [number, number, number, number],
            seqIndex: 0,
            anchors: {
              text: "OK",
              role: "button",
              href: null,
              alt: null,
              ariaLabel: null,
              nearestHeading: null,
              landmark: null,
              ordinalInLandmark: null,
            },
            cssSelector: "#btn4",
            rawHref: null,
            src: null,
            naturalWidth: null,
            naturalHeight: null,
            loaded: null,
            headingLevel: null,
          },
        ],
        landmarks: [],
        network: { requests: [] },
        console: [],
        a11y: { violations: [] },
        linkProbes: [],
      },
      computedStyles: {},
      styleCandidates: { ancestors: [], chains: {}, budget: 2000, truncated: false, droppedCount: 0 },
      screenshots: { fullPage: "desktop/old.png", viewport: "desktop/old-vp.png" },
      hitTests: probeResult,
    };

    const result = CaptureBundleSchema.safeParse(bundle);
    expect(result.success).toBe(true);
  });
});
