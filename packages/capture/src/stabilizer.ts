import type { BrowserContext, Page, Response } from "playwright";
import type {
  StabilizationConfig,
  DeterminismRecord,
  IntegrityInventory,
} from "./schema.js";

export interface StabilizationResult {
  determinism: DeterminismRecord;
  mainResponse: Response | null;
  redirectChain: string[];
}

/**
 * LCG Math.random seed script injected into page context.
 * Uses a simple multiplicative LCG seeded from config.
 */
function buildRandomSeedScript(seed: number): string {
  return `
    (function() {
      let s = ${seed >>> 0};
      Math.random = function() {
        s = (s * 1664525 + 1013904223) & 0xFFFFFFFF;
        return (s >>> 0) / 4294967296;
      };
    })();
  `;
}

/**
 * Animation kill CSS.
 */
const ANIMATION_KILL_CSS = `
  *,*::before,*::after {
    animation: none !important;
    transition: none !important;
  }
  html {
    scroll-behavior: auto !important;
    caret-color: transparent;
  }
`;

/**
 * Pure decision helper: should the stabilizer retry without the time freeze?
 *
 * Returns true when ALL of:
 *  - at least one step value is "failed"
 *  - the clock was installed (clockInstalled = true)
 *  - no retry has happened yet (alreadyRetried = false)
 */
export function shouldRetryWithoutFreeze(
  det: Pick<DeterminismRecord, "animationsDisabled" | "reducedMotion" | "timeFrozen" | "randomStubbed" | "fontsReady" | "imagesDecoded" | "lazyLoadPass" | "settled">,
  clockInstalled: boolean,
  alreadyRetried: boolean
): boolean {
  if (alreadyRetried) return false;
  if (!clockInstalled) return false;
  const steps: DeterminismRecord["animationsDisabled"][] = [
    det.animationsDisabled,
    det.reducedMotion,
    det.timeFrozen,
    det.randomStubbed,
    det.fontsReady,
    det.imagesDecoded,
    det.lazyLoadPass,
    det.settled,
  ];
  return steps.some((s) => s === "failed");
}

/**
 * Capture a page inventory snapshot: heading, image, and landmark counts.
 * Wrapped in try/catch: returns undefined on any failure so inventory
 * problems never fail a capture.
 */
async function takeInventory(page: Page): Promise<IntegrityInventory["pre"] | undefined> {
  try {
    return await page.evaluate((): { headingCount: number; imageCount: number; landmarkCount: number } => {
      return {
        headingCount: document.querySelectorAll("h1,h2,h3").length,
        imageCount: document.querySelectorAll("img").length,
        landmarkCount: document.querySelectorAll(
          "header,nav,main,footer,aside,[role=banner],[role=navigation],[role=main],[role=contentinfo],[role=complementary]"
        ).length,
      };
    });
  } catch {
    return undefined;
  }
}

/**
 * Run the 13-step stabilization sequence on a new page.
 * Returns determinism record and the main page response.
 */
export async function stabilize(
  context: BrowserContext,
  url: string,
  config: StabilizationConfig,
  hideSelectors: string[],
  maskSelectors: string[],
  clickBeforeCapture: string[],
  log: (msg: string) => void,
  onPageCreated?: (page: Page) => void
): Promise<{ page: Page } & StabilizationResult> {
  // -------------------------------------------------------------------------
  // Outer determinism record — will be re-initialized on retry.
  // -------------------------------------------------------------------------
  let det: DeterminismRecord = {
    animationsDisabled: "skipped",
    reducedMotion: "skipped",
    timeFrozen: "skipped",
    randomStubbed: "skipped",
    fontsReady: "skipped",
    imagesDecoded: "skipped",
    lazyLoadPass: "skipped",
    settled: "skipped",
    clicked: [],
    hidden: [],
    masked: [],
    retriedWithoutTimeFreeze: false,
  };

  /**
   * Node-side deadline wrapper: races `fn` against a real-clock setTimeout.
   * Unaffected by page.clock; prevents any page.evaluate from hanging forever.
   */
  function withDeadline<T>(fn: Promise<T>, ms: number): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(
        () => reject(new Error(`deadline exceeded (${ms}ms)`)),
        ms
      );
      fn.then(
        (v) => { clearTimeout(timer); resolve(v); },
        (e) => { clearTimeout(timer); reject(e); }
      );
    });
  }

  /**
   * Run the post-navigation stabilization pipeline on the given page.
   * Mutates `det` (passed by reference via the outer scope).
   * Returns pre/post inventory snapshots (each may be undefined on failure).
   */
  async function runPipeline(
    pg: Page,
    clockIsInstalled: boolean
  ): Promise<{ preInventory: IntegrityInventory["pre"] | undefined; postInventory: IntegrityInventory["pre"] | undefined }> {
    // Helper: record step result into the current det record.
    async function step(
      name: keyof Pick<
        DeterminismRecord,
        | "animationsDisabled"
        | "reducedMotion"
        | "timeFrozen"
        | "randomStubbed"
        | "fontsReady"
        | "imagesDecoded"
        | "lazyLoadPass"
        | "settled"
      >,
      fn: () => Promise<void>
    ): Promise<void> {
      try {
        await fn();
        det[name] = "ran";
      } catch (err) {
        det[name] = "failed";
        log(`[stabilizer] step ${name} failed: ${err}`);
      }
    }

    // PRE inventory: taken after navigation settles, before lazy-load/scroll steps.
    const preInventory = await takeInventory(pg);

    // Step 4: animation kill CSS
    await step("animationsDisabled", async () => {
      await pg.addStyleTag({ content: ANIMATION_KILL_CSS });
    });

    // Step 5: clickBeforeCapture
    det.clicked = [];
    for (const selector of clickBeforeCapture) {
      try {
        await pg.click(selector, { timeout: 3000 });
        det.clicked.push(selector);
      } catch {
        // absence is not failure per spec
        log(`[stabilizer] click selector not found or click failed: ${selector}`);
      }
    }

    // Step 6: clock.runFor(settleMs) if clock installed
    await step("settled", async () => {
      if (clockIsInstalled) {
        await pg.clock.runFor(config.settleMs);
      } else {
        // If no clock, just wait a bit
        await pg.waitForTimeout(Math.min(config.settleMs, 500));
      }
    });

    // Step 7a: document.fonts.ready (10 s Node-side deadline)
    await step("fontsReady", async () => {
      await withDeadline(pg.evaluate(() => document.fonts.ready), 10_000);
    });

    // Step 7b: decode only IN-VIEWPORT images that have already started loading.
    // checkVisibility() is NOT viewport-bounded — lazy images below the fold pass
    // it but their decode() never resolves (they haven't fetched yet). Restrict to
    // images whose bounding box intersects the current viewport (0,0,innerWidth,innerHeight)
    // and which are either complete or non-lazy, then wrap the whole evaluate in a
    // 10 s Node-side deadline so a stray decode can never hang the process.
    await step("imagesDecoded", async () => {
      await withDeadline(
        pg.evaluate(async () => {
          const vw = window.innerWidth;
          const vh = window.innerHeight;
          const imgs = Array.from(document.querySelectorAll<HTMLImageElement>("img"));
          const candidates = imgs.filter((img) => {
            // Must be visible per CSS
            try {
              if (!img.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })) {
                return false;
              }
            } catch {
              return false;
            }
            // Must intersect the current viewport rectangle
            const r = img.getBoundingClientRect();
            if (r.width <= 0 || r.height <= 0) return false;
            if (r.right <= 0 || r.bottom <= 0 || r.left >= vw || r.top >= vh) return false;
            // Skip lazy images that haven't started fetching (complete === false and loading === "lazy")
            if (!img.complete && img.loading === "lazy") return false;
            return true;
          });
          // Per-image rejection (e.g. 404) is page reality, NOT a step failure
          await Promise.allSettled(candidates.map((img) => img.decode()));
        }),
        10_000
      );
    });

    // Step 8: lazy-load pass
    await step("lazyLoadPass", async () => {
      // Get total height first
      const totalHeight = await pg.evaluate(
        () => document.documentElement.scrollHeight
      );

      // Scroll to bottom in steps from Node side (avoids setTimeout deadlock with frozen clock)
      let current = 0;
      while (current < totalHeight) {
        current += config.lazyScrollStepPx;
        const scrollTo = current;
        await pg.evaluate((y: number) => window.scrollTo(0, y), scrollTo);
        // Advance the clock a bit to allow intersection observers to fire
        if (clockIsInstalled) {
          await pg.clock.runFor(100);
        }
      }

      // Scroll back to top
      await pg.evaluate(() => window.scrollTo(0, 0));
      if (clockIsInstalled) {
        await pg.clock.runFor(100);
      }

      // Re-wait fonts (10 s Node-side deadline)
      await withDeadline(pg.evaluate(() => document.fonts.ready), 10_000);

      // Re-decode: only images that have already finished loading (complete === true).
      // After the scroll pass more images may have loaded; skip any still incomplete.
      // 10 s Node-side deadline guards against any stragglers.
      await withDeadline(
        pg.evaluate(async () => {
          const imgs = Array.from(document.querySelectorAll<HTMLImageElement>("img"));
          const loaded = imgs.filter((img) => img.complete);
          await Promise.allSettled(loaded.map((img) => img.decode()));
        }),
        10_000
      );

      // clock.runFor(250) if clock installed
      if (clockIsInstalled) {
        await pg.clock.runFor(250);
      }
    });

    // Step 9: apply hideSelectors and maskSelectors
    det.hidden = [];
    for (const selector of hideSelectors) {
      try {
        await pg.addStyleTag({
          content: `${selector} { visibility: hidden !important; }`,
        });
        det.hidden.push(selector);
      } catch {
        log(`[stabilizer] hide selector failed: ${selector}`);
      }
    }

    det.masked = [];
    for (const selector of maskSelectors) {
      try {
        await pg.addStyleTag({
          content: `${selector} { background-color: #808080 !important; color: transparent !important; background-image: none !important; }`,
        });
        det.masked.push(selector);
      } catch {
        log(`[stabilizer] mask selector failed: ${selector}`);
      }
    }

    // POST inventory: taken after the full pipeline.
    const postInventory = await takeInventory(pg);

    return { preInventory, postInventory };
  }

  // -------------------------------------------------------------------------
  // Step 1: Create page (context already configured with viewport/dsf/locale/tz/colorScheme)
  // -------------------------------------------------------------------------
  let page = await context.newPage();
  onPageCreated?.(page);

  // Step 2a: emulateMedia reducedMotion
  try {
    await page.emulateMedia({ reducedMotion: "reduce" });
    det.reducedMotion = "ran";
  } catch (err) {
    det.reducedMotion = "failed";
    log(`[stabilizer] step reducedMotion failed: ${err}`);
  }

  // Step 2b: stubRandom via init script
  if (config.stubRandom) {
    try {
      await context.addInitScript(buildRandomSeedScript(config.randomSeed));
      det.randomStubbed = "ran";
    } catch (err) {
      det.randomStubbed = "failed";
      log(`[stabilizer] step randomStubbed failed: ${err}`);
    }
  } else {
    det.randomStubbed = "skipped";
  }

  // Step 2c: clock.install if freezeTime
  let clockInstalled = false;
  if (config.freezeTime) {
    try {
      await page.clock.install({ time: config.fixedTime });
      clockInstalled = true;
      det.timeFrozen = "ran";
    } catch (err) {
      det.timeFrozen = "failed";
      log(`[stabilizer] step timeFrozen failed: ${err}`);
    }
  } else {
    det.timeFrozen = "skipped";
  }

  // -------------------------------------------------------------------------
  // Step 3: goto + networkidle
  // -------------------------------------------------------------------------
  let mainResponse: Response | null = null;
  let redirectChain: string[] = [];
  let navigationSucceeded = false;
  let alreadyRetried = false;

  try {
    mainResponse = await page.goto(url, {
      waitUntil: "load",
      timeout: config.networkIdleTimeoutMs,
    });
    // Record redirect chain
    if (mainResponse) {
      redirectChain = mainResponse
        .request()
        .redirectedFrom()
        ? collectRedirectChain(mainResponse)
        : [];
    }
    // Wait for networkidle
    await page.waitForLoadState("networkidle", {
      timeout: config.networkIdleTimeoutMs,
    });
    navigationSucceeded = true;
  } catch (err) {
    const errMsg = String(err);
    if (clockInstalled && errMsg.includes("timeout")) {
      // networkidle timeout with frozen clock — retry without freeze
      log("[stabilizer] networkidle timeout with freezeTime, retrying without it");
      navigationSucceeded = false;
      // Fall through to the unified retry path below.
    } else {
      log(`[stabilizer] navigation failed: ${err}`);
    }
  }

  // -------------------------------------------------------------------------
  // Run the post-navigation pipeline (first attempt).
  // -------------------------------------------------------------------------
  let preInventory: IntegrityInventory["pre"] | undefined;
  let postInventory: IntegrityInventory["pre"] | undefined;

  if (navigationSucceeded) {
    const inv = await runPipeline(page, clockInstalled);
    preInventory = inv.preInventory;
    postInventory = inv.postInventory;
  }

  // -------------------------------------------------------------------------
  // Retry-without-freeze decision:
  // Triggers when:
  //   (a) networkidle timed out under frozen clock (navigationSucceeded = false), OR
  //   (b) pipeline ran but at least one step failed, AND the clock was installed,
  //       AND we haven't retried yet.
  // -------------------------------------------------------------------------
  const needsRetry =
    !alreadyRetried &&
    clockInstalled &&
    (!navigationSucceeded || shouldRetryWithoutFreeze(det, clockInstalled, alreadyRetried));

  if (needsRetry) {
    alreadyRetried = true;
    log("[stabilizer] retrying without time freeze (step failure or timeout under frozen clock)");

    // Re-initialize the determinism record for the retry pass.
    det = {
      animationsDisabled: "skipped",
      reducedMotion: "skipped",
      timeFrozen: "skipped",   // skipped — no clock on retry
      randomStubbed: det.randomStubbed, // random seed init script already added to context
      fontsReady: "skipped",
      imagesDecoded: "skipped",
      lazyLoadPass: "skipped",
      settled: "skipped",
      clicked: [],
      hidden: [],
      masked: [],
      retriedWithoutTimeFreeze: true,
    };

    await page.close();
    page = await context.newPage();
    onPageCreated?.(page);

    // Re-apply reducedMotion
    try {
      await page.emulateMedia({ reducedMotion: "reduce" });
      det.reducedMotion = "ran";
    } catch { /* ignore */ }

    clockInstalled = false;

    navigationSucceeded = false;
    try {
      mainResponse = await page.goto(url, {
        waitUntil: "load",
        timeout: config.networkIdleTimeoutMs,
      });
      if (mainResponse) {
        redirectChain = collectRedirectChain(mainResponse);
      }
      await page.waitForLoadState("networkidle", {
        timeout: config.networkIdleTimeoutMs,
      });
      navigationSucceeded = true;
    } catch (err2) {
      log(`[stabilizer] navigation retry also failed: ${err2}`);
    }

    if (navigationSucceeded) {
      const inv = await runPipeline(page, false);
      preInventory = inv.preInventory;
      postInventory = inv.postInventory;
    }
  }

  if (!navigationSucceeded) {
    return { page, determinism: det, mainResponse, redirectChain };
  }

  // -------------------------------------------------------------------------
  // Record integrity inventory into det if both snapshots were taken.
  // -------------------------------------------------------------------------
  if (preInventory !== undefined && postInventory !== undefined) {
    det.integrity = { pre: preInventory, post: postInventory };
  }

  return { page, determinism: det, mainResponse, redirectChain };
}

/**
 * Collect the redirect chain from a response.
 */
function collectRedirectChain(response: Response): string[] {
  const chain: string[] = [];
  let req = response.request().redirectedFrom();
  while (req) {
    chain.unshift(req.url());
    req = req.redirectedFrom();
  }
  return chain;
}
