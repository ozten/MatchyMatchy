/**
 * Port-parity U12: the evolved "full" settle stage + "off" mode. Real
 * Chromium via `launchBrowser`/`createContext` against small fixture pages
 * served over a local HTTP server (see tests/helpers/serve-html.ts —
 * `stabilize()` navigates itself via `page.goto`, so `page.setContent()`
 * doesn't apply here). Legacy-mode characterization lives in
 * tests/stabilizer.test.ts.
 */
import { describe, it, expect, beforeAll, afterAll } from "vitest";
import type { Browser, BrowserContext } from "playwright";
import { stabilize } from "../src/stabilizer.js";
import type { StabilizationConfig } from "../src/schema.js";
import { launchBrowser, createContext } from "../src/browser-runner.js";
import { serveHtml } from "./helpers/serve-html.js";
import { baseStabilizationConfig } from "./helpers/stabilization-config.js";

const BROWSER_TIMEOUT_MS = 30_000;

describe("settle stage — 'full' mode / 'off' mode (browser)", () => {
  let browser: Browser;

  beforeAll(async () => {
    browser = await launchBrowser();
  }, BROWSER_TIMEOUT_MS);

  afterAll(async () => {
    await browser?.close();
  }, BROWSER_TIMEOUT_MS);

  async function withContext(
    viewport: { width: number; height: number },
    fn: (context: BrowserContext) => Promise<void>
  ): Promise<void> {
    const context = await createContext(browser, {
      name: "desktop",
      width: viewport.width,
      height: viewport.height,
      dsf: 1,
    });
    try {
      await fn(context);
    } finally {
      await context.close();
    }
  }

  // ── (a) static page → settle ran + quiescence reached quickly ──────────
  it("reaches quiescence quickly on a static page", async () => {
    const html = `<!doctype html><html><body style="margin:0">
      <div style="height:2000px">static content</div>
    </body></html>`;
    const served = await serveHtml(html);
    try {
      await withContext({ width: 800, height: 300 }, async (context) => {
        const config = baseStabilizationConfig({ settleMode: "full", settleMs: 0 });
        const { page, determinism } = await stabilize(
          context,
          served.url,
          config,
          [],
          [],
          [],
          () => {}
        );
        expect(determinism.settle).toBe("ran");
        expect(determinism.lazyLoadPass).toBe("skipped");
        expect(determinism.quiescence).toBe("reached");
        expect(determinism.settleGrowthCapped).toBeUndefined();
        expect(determinism.settleScrollIneffective).toBeUndefined();
        await page.close();
      });
    } finally {
      await served.close();
    }
  }, BROWSER_TIMEOUT_MS);

  // ── (b) scroll-position-driven reveal (no wall-clock dependence) ────────
  it("reveals below-fold content driven by scroll position, and it stays revealed after returning to top", async () => {
    const html = `<!doctype html><html><body style="margin:0">
      <div style="height:1200px">top</div>
      <div id="reveal" style="height:500px;opacity:0;">bottom content</div>
      <script>
        // Reveal-once-on-scroll-into-view, mirroring real scroll-triggered
        // reveal animation frameworks (IX2/AOS-style): driven entirely by
        // window.scrollY at the moment of the 'scroll' event, never by a
        // timer/rAF, and — once revealed — the state is NOT reverted when
        // scrolling back to the top (matching the real-world "reveal once"
        // semantics the settle stage's return-to-top step must not defeat).
        var revealed = false;
        window.addEventListener('scroll', function () {
          if (revealed) return;
          var el = document.getElementById('reveal');
          if (window.scrollY + window.innerHeight >= el.offsetTop) {
            el.style.opacity = '1';
            revealed = true;
          }
        });
      </script>
    </body></html>`;
    const served = await serveHtml(html);
    try {
      await withContext({ width: 800, height: 300 }, async (context) => {
        const config = baseStabilizationConfig({ settleMode: "full", settleMs: 0 });
        const { page, determinism } = await stabilize(
          context,
          served.url,
          config,
          [],
          [],
          [],
          () => {}
        );
        expect(determinism.settle).toBe("ran");
        const opacity = await page.evaluate(
          () => getComputedStyle(document.getElementById("reveal")!).opacity
        );
        expect(opacity).toBe("1");
        // Settle returns to the top — the reveal must persist regardless.
        const scrollY = await page.evaluate(() => window.scrollY);
        expect(scrollY).toBe(0);
        await page.close();
      });
    } finally {
      await served.close();
    }
  }, BROWSER_TIMEOUT_MS);

  // ── (c) rAF marquee → quiescence timeout recorded, capture completes ───
  it("records a quiescence timeout for a perpetually-animating rAF marquee, and capture still completes", async () => {
    const html = `<!doctype html><html><body style="margin:0">
      <div id="marquee">x</div>
      <script>
        function tick() {
          var el = document.getElementById('marquee');
          el.textContent = el.textContent === 'x' ? 'y' : 'x';
          requestAnimationFrame(tick);
        }
        requestAnimationFrame(tick);
      </script>
    </body></html>`;
    const served = await serveHtml(html);
    try {
      await withContext({ width: 800, height: 300 }, async (context) => {
        // Small window/timeout so a perpetual mutator reliably times out fast.
        const config = baseStabilizationConfig({
          settleMode: "full",
          settleMs: 0,
          quiescenceWindowMs: 100,
          quiescenceTimeoutMs: 300,
        });
        const { page, determinism } = await stabilize(
          context,
          served.url,
          config,
          [],
          [],
          [],
          () => {}
        );
        expect(determinism.settle).toBe("ran");
        expect(determinism.quiescence).toBe("timeout");
        // Capture pipeline completes normally (page usable, no throw).
        const title = await page.evaluate(() => document.readyState);
        expect(title).toBe("complete");
        await page.close();
      });
    } finally {
      await served.close();
    }
  }, BROWSER_TIMEOUT_MS);

  // ── (d) growing page (script appends content on scroll) → growth cap ───
  it("hits the growth cap on a page that keeps growing under scroll, and records settleGrowthCapped", async () => {
    const html = `<!doctype html><html><body style="margin:0">
      <div id="feed" style="height:2000px">seed</div>
      <script>
        window.addEventListener('scroll', function () {
          var d = document.createElement('div');
          d.style.height = '2000px';
          d.textContent = 'more';
          document.body.appendChild(d);
        });
      </script>
    </body></html>`;
    const served = await serveHtml(html);
    try {
      await withContext({ width: 800, height: 300 }, async (context) => {
        const config = baseStabilizationConfig({
          settleMode: "full",
          settleMs: 0,
          maxSettleSteps: 5,
          quiescenceWindowMs: 100,
          quiescenceTimeoutMs: 300,
        });
        const { page, determinism } = await stabilize(
          context,
          served.url,
          config,
          [],
          [],
          [],
          () => {}
        );
        expect(determinism.settle).toBe("ran");
        expect(determinism.settleGrowthCapped).toBe(true);
        await page.close();
      });
    } finally {
      await served.close();
    }
  }, BROWSER_TIMEOUT_MS);

  // ── (e) transform-scroll site (overflow:hidden ⇒ scrollTo is a no-op) ──
  it("detects scroll-ineffectiveness on a transform-scroll site and records settleScrollIneffective", async () => {
    const html = `<!doctype html><html><body style="margin:0">
      <style>html, body { overflow: hidden; height: 100%; margin: 0; }</style>
      <div id="track" style="height:3000px;"></div>
    </body></html>`;
    const served = await serveHtml(html);
    try {
      await withContext({ width: 800, height: 300 }, async (context) => {
        const config = baseStabilizationConfig({
          settleMode: "full",
          settleMs: 0,
          quiescenceWindowMs: 100,
          quiescenceTimeoutMs: 300,
        });
        const { page, determinism } = await stabilize(
          context,
          served.url,
          config,
          [],
          [],
          [],
          () => {}
        );
        expect(determinism.settle).toBe("ran");
        expect(determinism.settleScrollIneffective).toBe(true);
        // Still performs image-await + quiescence — the pipeline completes.
        expect(["reached", "timeout"]).toContain(determinism.quiescence);
        await page.close();
      });
    } finally {
      await served.close();
    }
  }, BROWSER_TIMEOUT_MS);

  // ── (f) "off" mode: step 8 skipped entirely ─────────────────────────────
  it("skips step 8 entirely under settleMode 'off'", async () => {
    const html = `<!doctype html><html><body style="margin:0"><div style="height:2000px"></div></body></html>`;
    const served = await serveHtml(html);
    try {
      await withContext({ width: 800, height: 300 }, async (context) => {
        const config = baseStabilizationConfig({ settleMode: "off" });
        const { page, determinism } = await stabilize(
          context,
          served.url,
          config,
          [],
          [],
          [],
          () => {}
        );
        expect(determinism.lazyLoadPass).toBe("skipped");
        expect(determinism.settle).toBe("skipped");
        expect(determinism.quiescence).toBe("notRun");
        await page.close();
      });
    } finally {
      await served.close();
    }
  }, BROWSER_TIMEOUT_MS);

  // ── (g) full mode + no clock (freezeTime: false) → wall-timeout dwell ──
  it("completes settle via wall-clock dwell when the clock is not installed (freezeTime: false)", async () => {
    const html = `<!doctype html><html><body style="margin:0"><div style="height:1200px">static</div></body></html>`;
    const served = await serveHtml(html);
    try {
      await withContext({ width: 800, height: 300 }, async (context) => {
        const config = baseStabilizationConfig({
          settleMode: "full",
          freezeTime: false,
          settleMs: 0,
          settleDwellMs: 50,
          quiescenceWindowMs: 100,
          quiescenceTimeoutMs: 1000,
        });
        const { page, determinism } = await stabilize(
          context,
          served.url,
          config,
          [],
          [],
          [],
          () => {}
        );
        expect(determinism.timeFrozen).toBe("skipped");
        expect(determinism.settle).toBe("ran");
        expect(["reached", "timeout"]).toContain(determinism.quiescence);
        await page.close();
      });
    } finally {
      await served.close();
    }
  }, BROWSER_TIMEOUT_MS);
});
