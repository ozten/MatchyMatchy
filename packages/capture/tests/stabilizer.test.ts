import { describe, it, expect, beforeAll, afterAll } from "vitest";
import type { Browser, BrowserContext } from "playwright";
import { shouldRetryWithoutFreeze, stabilize } from "../src/stabilizer.js";
import type { DeterminismRecord, StabilizationConfig } from "../src/schema.js";
import { launchBrowser, createContext } from "../src/browser-runner.js";
import { serveHtml } from "./helpers/serve-html.js";

/** A DeterminismRecord with all steps "ran" — used as base for mutations. */
function allRan(): Pick<
  DeterminismRecord,
  | "animationsDisabled"
  | "reducedMotion"
  | "timeFrozen"
  | "randomStubbed"
  | "fontsReady"
  | "imagesDecoded"
  | "lazyLoadPass"
  | "settled"
> {
  return {
    animationsDisabled: "ran",
    reducedMotion: "ran",
    timeFrozen: "ran",
    randomStubbed: "ran",
    fontsReady: "ran",
    imagesDecoded: "ran",
    lazyLoadPass: "ran",
    settled: "ran",
  };
}

describe("shouldRetryWithoutFreeze", () => {
  it("returns true when a step failed, clock was installed, and not yet retried", () => {
    const det = { ...allRan(), settled: "failed" as const };
    expect(shouldRetryWithoutFreeze(det, /* clockInstalled */ true, /* alreadyRetried */ false)).toBe(true);
  });

  it("returns false when a step failed but clock was NOT installed", () => {
    const det = { ...allRan(), lazyLoadPass: "failed" as const };
    expect(shouldRetryWithoutFreeze(det, /* clockInstalled */ false, /* alreadyRetried */ false)).toBe(false);
  });

  it("returns false when already retried (even if step failed + clock installed)", () => {
    const det = { ...allRan(), fontsReady: "failed" as const };
    expect(shouldRetryWithoutFreeze(det, /* clockInstalled */ true, /* alreadyRetried */ true)).toBe(false);
  });

  it("returns false when all steps ran (no failure)", () => {
    const det = allRan();
    expect(shouldRetryWithoutFreeze(det, /* clockInstalled */ true, /* alreadyRetried */ false)).toBe(false);
  });

  it("returns true when timeFrozen step itself failed (clock was being installed)", () => {
    const det = { ...allRan(), timeFrozen: "failed" as const };
    expect(shouldRetryWithoutFreeze(det, /* clockInstalled */ true, /* alreadyRetried */ false)).toBe(true);
  });

  it("returns false when all steps skipped and no failure", () => {
    const det = {
      animationsDisabled: "skipped" as const,
      reducedMotion: "skipped" as const,
      timeFrozen: "skipped" as const,
      randomStubbed: "skipped" as const,
      fontsReady: "skipped" as const,
      imagesDecoded: "skipped" as const,
      lazyLoadPass: "skipped" as const,
      settled: "skipped" as const,
    };
    expect(shouldRetryWithoutFreeze(det, true, false)).toBe(false);
  });

  // ── Port-parity U12: the new "settle" step joins the same trigger set ────
  it("returns true when the new 'settle' step failed (settle joins the existing trigger set)", () => {
    const det = { ...allRan(), settle: "failed" as const };
    expect(shouldRetryWithoutFreeze(det, /* clockInstalled */ true, /* alreadyRetried */ false)).toBe(true);
  });

  it("ignores an absent/undefined 'settle' field (legacy/off modes never set it)", () => {
    const det = allRan(); // no `settle` key at all
    expect(shouldRetryWithoutFreeze(det, true, false)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Port-parity U12 — characterization of the CURRENT (pre-settle-evolution)
// lazyLoadPass behavior, pinned via real Chromium BEFORE stabilizer.ts's
// step 8 is touched. Every assertion here must remain true after "legacy"
// mode is wired in (settleMode defaults to "legacy" — byte-identical to
// today's unconditional behavior).
// ---------------------------------------------------------------------------
describe("stabilize() step 8 — legacy lazyLoadPass characterization (browser)", () => {
  let browser: Browser;

  beforeAll(async () => {
    browser = await launchBrowser();
  }, 30_000);

  afterAll(async () => {
    await browser?.close();
  }, 30_000);

  function legacyConfig(overrides: Partial<StabilizationConfig> = {}): StabilizationConfig {
    return {
      freezeTime: true,
      fixedTime: "2026-01-01T00:00:00.000Z",
      stubRandom: true,
      randomSeed: 1337,
      networkIdleTimeoutMs: 15000,
      settleMs: 0, // isolate step 8's own clock/scroll contribution
      lazyScrollStepPx: 800,
      settleMode: "legacy",
      ...overrides,
    };
  }

  // ── (a) scroll step sequence: fixed 800px steps, independent of viewport
  //        height, then back-to-top ─────────────────────────────────────────
  it("scrolls in fixed 800px steps (viewport-height independent) then returns to top", async () => {
    const html = `<!doctype html><html><body style="margin:0">
      <script>
        window.__scrollLog = [];
        var origScrollTo = window.scrollTo.bind(window);
        window.scrollTo = function (x, y) {
          window.__scrollLog.push(y);
          return origScrollTo(x, y);
        };
      </script>
      <div style="height:900px"></div>
      <div style="height:900px"></div>
      <div style="height:900px"></div>
    </body></html>`;
    const served = await serveHtml(html);
    const context: BrowserContext = await createContext(browser, {
      name: "desktop",
      width: 1024,
      height: 400,
      dsf: 1,
    });
    try {
      const { page, determinism } = await stabilize(
        context,
        served.url,
        legacyConfig(),
        [],
        [],
        [],
        () => {}
      );
      expect(determinism.lazyLoadPass).toBe("ran");
      // Added once the settleMode dispatch landed (post-characterization):
      // "legacy" mode carries the new fields at their documented values.
      expect(determinism.settle).toBe("skipped");
      expect(determinism.quiescence).toBe("notRun");

      const log = await page.evaluate(
        () => (window as unknown as { __scrollLog: number[] }).__scrollLog
      );
      // Fixed 800px steps over a 2700px page: 800, 1600, 2400, 3200 (overshoot
      // on the final step is expected — the loop condition is checked BEFORE
      // incrementing), then a final scrollTo(0, 0) back-to-top.
      expect(log).toEqual([800, 1600, 2400, 3200, 0]);

      const finalScrollY = await page.evaluate(() => window.scrollY);
      expect(finalScrollY).toBe(0);
      await page.close();
    } finally {
      await context.close();
      await served.close();
    }
  });

  // ── (b) clock advances: at least 100 (scroll) + 100 (back-to-top) + 250
  //        (final) = 450ms of CONTROLLED clock advance for a
  //        single-iteration scroll pass. Playwright's fake clock ticks in
  //        real (wall) time from `clock.install()` until the first explicit
  //        `runFor`/`fastForward`/`pauseAt` call (step 6's `clock.runFor`,
  //        which fires unconditionally once the clock is installed, even
  //        for `settleMs: 0`) — empirically ~40ms of jitter across runs in
  //        this sandbox, almost certainly `waitForLoadState("networkidle")`'s
  //        ~500ms quiet-window heuristic. That real-time leak makes an exact
  //        equality assertion flaky; a lower bound still faithfully pins the
  //        step's deterministic, CONTROLLED contribution (the part this unit
  //        touches) without depending on navigation-timing noise.
  it("advances the frozen clock by at least 450ms for a single-iteration scroll pass", async () => {
    // Short viewport height (100px) so the 400px body is unambiguously taller
    // than the viewport — exactly one 800px scroll step is needed to reach
    // the bottom, regardless of how the browser resolves scrollHeight for a
    // body shorter than a TALL viewport.
    const html = `<!doctype html><html><body style="margin:0"><div style="height:400px"></div></body></html>`;
    const served = await serveHtml(html);
    const context: BrowserContext = await createContext(browser, {
      name: "desktop",
      width: 800,
      height: 100,
      dsf: 1,
    });
    try {
      const config = legacyConfig();
      const { page, determinism } = await stabilize(
        context,
        served.url,
        config,
        [],
        [],
        [],
        () => {}
      );
      expect(determinism.timeFrozen).toBe("ran");
      expect(determinism.lazyLoadPass).toBe("ran");

      const nowMs = await page.evaluate(() => Date.now());
      const baseMs = new Date(config.fixedTime).getTime();
      // Lower bound: the guaranteed controlled advance (100 scroll + 100
      // back-to-top + 250 final). Upper bound: sanity ceiling ruling out a
      // hang or a runaway/duplicated advance.
      expect(nowMs).toBeGreaterThanOrEqual(baseMs + 450);
      expect(nowMs).toBeLessThan(baseMs + 450 + 5000);
      await page.close();
    } finally {
      await context.close();
      await served.close();
    }
  });

  // ── (c) below-fold lazy image loads after the scroll pass. Also pins the
  //        U12 integration requirement: legacy/--no-settle must never
  //        regress below-fold lazy-image loading. ──────────────────────────
  it("loads a below-fold loading=lazy image after the scroll pass", async () => {
    const html = `<!doctype html><html><body style="margin:0">
      <div style="height:1000px"></div>
      <img id="lazyImg" loading="lazy" width="10" height="10"
           src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBTAA7">
    </body></html>`;
    const served = await serveHtml(html);
    const context: BrowserContext = await createContext(browser, {
      name: "desktop",
      width: 800,
      height: 300,
      dsf: 1,
    });
    try {
      const { page, determinism } = await stabilize(
        context,
        served.url,
        legacyConfig({ settleMs: 1000 }),
        [],
        [],
        [],
        () => {}
      );
      expect(determinism.lazyLoadPass).toBe("ran");

      const loaded = await page.evaluate(() => {
        const img = document.getElementById("lazyImg") as HTMLImageElement;
        return img.complete && img.naturalWidth > 0;
      });
      expect(loaded).toBe(true);
      await page.close();
    } finally {
      await context.close();
      await served.close();
    }
  });
});
