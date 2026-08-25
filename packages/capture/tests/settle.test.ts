/**
 * Port-parity U12: the evolved "full" settle stage + "off" mode. Real
 * Chromium via `launchBrowser`/`createContext` against small fixture pages
 * served over a local HTTP server (see tests/helpers/serve-html.ts —
 * `stabilize()` navigates itself via `page.goto`, so `page.setContent()`
 * doesn't apply here). Legacy-mode characterization lives in
 * tests/stabilizer.test.ts.
 */
import { describe, it, expect, beforeAll, afterAll } from "vitest";
import type { Browser, BrowserContext, Page } from "playwright";
import * as http from "http";
import type { AddressInfo } from "net";
import { stabilize } from "../src/stabilizer.js";
import type { StabilizationConfig } from "../src/schema.js";
import { launchBrowser, createContext } from "../src/browser-runner.js";
import { serveHtml } from "./helpers/serve-html.js";
import { baseStabilizationConfig } from "./helpers/stabilization-config.js";

const BROWSER_TIMEOUT_MS = 30_000;

/**
 * Like `serveHtml`, but a single `notFoundPath` returns a genuine 404
 * instead of the fixed HTML body — needed to reproduce a real "already
 * failed before settle runs" image (bug 1's exact scenario), which requires
 * an actual non-2xx response, not just invalid image bytes.
 */
async function serveHtmlWith404(
  html: string,
  notFoundPath: string
): Promise<{ url: string; close: () => Promise<void> }> {
  const server = http.createServer((req, res) => {
    if (req.url === notFoundPath) {
      res.writeHead(404, { "Content-Type": "text/plain" });
      res.end("not found");
      return;
    }
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(html);
  });
  await new Promise<void>((resolve) => {
    server.listen(0, "127.0.0.1", resolve);
  });
  const { port } = server.address() as AddressInfo;
  return {
    url: `http://127.0.0.1:${port}/`,
    close: () =>
      new Promise<void>((resolve, reject) => {
        server.close((err) => (err ? reject(err) : resolve()));
      }),
  };
}

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

  // ── (h) BUG 1 — an already-404'd image must never hang the image-await
  //        phase. `page.goto({ waitUntil: "load" })` already blocks on the
  //        window `load` event, which itself waits for every <img> to
  //        finish attempting its load (success or error) — so by the time
  //        settle's step 8 runs, this image's error event has ALREADY fired
  //        and will never fire again. The old code's 10s-per-phase deadline
  //        THREW on expiry if any image-await call ever failed to settle;
  //        this test pins the fast, non-hanging path for the ordinary
  //        already-settled case. ─────────────────────────────────────────
  it("treats an already-404'd image as settled instantly and completes settle in bounded time", async () => {
    const html = `<!doctype html><html><body style="margin:0">
      <div style="height:800px">content</div>
      <img id="broken" src="/missing.png" width="10" height="10">
    </body></html>`;
    const served = await serveHtmlWith404(html, "/missing.png");
    try {
      await withContext({ width: 800, height: 300 }, async (context) => {
        const config = baseStabilizationConfig({ settleMode: "full", settleMs: 0 });
        const t0 = Date.now();
        const { page, determinism } = await stabilize(
          context,
          served.url,
          config,
          [],
          [],
          [],
          () => {}
        );
        const elapsedMs = Date.now() - t0;
        expect(determinism.settle).toBe("ran");
        // Well under the old 10s-per-phase hang (two image-await calls could
        // have consumed up to ~20s if either one hung).
        expect(elapsedMs).toBeLessThan(8000);

        const imgState = await page.evaluate(() => {
          const img = document.getElementById("broken") as HTMLImageElement;
          return { complete: img.complete, naturalWidth: img.naturalWidth };
        });
        expect(imgState.complete).toBe(true);
        expect(imgState.naturalWidth).toBe(0);
        await page.close();
      });
    } finally {
      await served.close();
    }
  }, BROWSER_TIMEOUT_MS);

  // ── (i) BUG 2 — settle must restore page state on ANY failure. Forces a
  //        throw partway through the scroll loop (after scrollY has already
  //        moved off 0) via a test-only monkeypatch of `page.evaluate`
  //        installed through the `onPageCreated` hook, then asserts BOTH
  //        that the failure is recorded AND that runFullSettle's finally
  //        block restored scroll to (0, 0) before extraction would run. ──
  it("restores scroll to (0,0) via the finally block when a settle phase throws mid-pass", async () => {
    const html = `<!doctype html><html><body style="margin:0">
      <div style="height:3000px">tall content</div>
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
          () => {},
          (pg: Page) => {
            // Test hook: force the settle stage's SECOND per-step
            // `scrollTo(0, y)` evaluate call to throw, simulating a
            // mid-phase failure once the page has already scrolled away
            // from the top (the first scrollTo call is allowed to succeed).
            const original = pg.evaluate.bind(pg);
            let scrollStepCalls = 0;
            (pg as unknown as { evaluate: unknown }).evaluate = (
              fn: unknown,
              ...args: unknown[]
            ) => {
              if (typeof fn === "function" && fn.toString().includes("scrollTo(0, y)")) {
                scrollStepCalls += 1;
                if (scrollStepCalls === 2) {
                  return Promise.reject(new Error("injected test failure: forced settle throw"));
                }
              }
              return (original as (...a: unknown[]) => unknown)(fn, ...args);
            };
          }
        );
        expect(determinism.settle).toBe("failed");
        const scrollY = await page.evaluate(() => window.scrollY);
        expect(scrollY).toBe(0);
        await page.close();
      });
    } finally {
      await served.close();
    }
  }, BROWSER_TIMEOUT_MS);
});
