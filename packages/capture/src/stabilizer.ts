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
 *
 * Port-parity U12: the evolved settle stage's `settle` step (only ever set
 * when `settleMode === "full"`; absent under "legacy"/"off") joins this SAME
 * trigger set — a settle failure is a pipeline-step failure exactly like any
 * other, so it must participate here rather than being silently swallowed.
 */
export function shouldRetryWithoutFreeze(
  det: Pick<DeterminismRecord, "animationsDisabled" | "reducedMotion" | "timeFrozen" | "randomStubbed" | "fontsReady" | "imagesDecoded" | "lazyLoadPass" | "settled" | "settle">,
  clockInstalled: boolean,
  alreadyRetried: boolean
): boolean {
  if (alreadyRetried) return false;
  if (!clockInstalled) return false;
  const steps: (DeterminismRecord["animationsDisabled"] | undefined)[] = [
    det.animationsDisabled,
    det.reducedMotion,
    det.timeFrozen,
    det.randomStubbed,
    det.fontsReady,
    det.imagesDecoded,
    det.lazyLoadPass,
    det.settled,
    det.settle,
  ];
  return steps.some((s) => s === "failed");
}

/**
 * Node-side deadline wrapper: races `fn` against a real-clock setTimeout.
 * Unaffected by page.clock; prevents any page.evaluate from hanging forever.
 * Module-level (not a stabilize() closure) so the settle-stage helpers below
 * can share it.
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

// ---------------------------------------------------------------------------
// Port-parity U12: the evolved "full" settle stage.
//
// Frozen defaults for the optional StabilizationConfig knobs (the zod schema
// leaves these `.optional()` — no runtime default — per schema.ts's comment
// "stabilizer applies its own default when absent"). Deferred to
// implementation per plan §"Deferred to Implementation"; calibrated on the
// scroll-reveal fixture (v24) and this unit's browser tests.
// ---------------------------------------------------------------------------
const DEFAULT_SETTLE_DWELL_MS = 200;
const DEFAULT_QUIESCENCE_WINDOW_MS = 500;
const DEFAULT_QUIESCENCE_TIMEOUT_MS = 5000;
const DEFAULT_MAX_SETTLE_STEPS = 60;

/** Result of one {@link awaitAllImagesLoadOrError} phase. */
interface ImageAwaitResult {
  /** True when the 10s Node-side deadline elapsed before every image settled. */
  timedOut: boolean;
  /** Count of images that were not yet `complete` when the phase began (0 when timedOut is false). */
  pendingCount: number;
}

/**
 * Await every `<img>` currently in the document to either load or error,
 * bounded by the existing 10s node-side deadline pattern. Re-queries the DOM
 * at call time so images inserted mid-scroll are included.
 *
 * An image whose `complete === true` is ALREADY settled — this covers both a
 * successful load and a failed one (a 404'd image has `complete === true`
 * and `naturalWidth === 0`) — and is never made to wait on an event that has
 * already fired and will not fire again.
 *
 * For a still-pending image, race its `decode()` call against fresh `load`/
 * `error` listeners: `decode()` resolves once the image is successfully
 * decoded and rejects on a load error, but per real-world observation (an
 * `<img>` sitting inside a collapsed/hidden subtree — e.g. an unopened
 * dropdown menu — can be `naturalWidth > 0` yet permanently NOT `complete`,
 * because Chromium never runs the decode/layout pass for a non-rendered
 * image) `decode()` alone can hang forever even though the element already
 * fired its terminal event in the past. Racing both catches whichever the
 * browser actually delivers; whichever fires first is a terminal outcome
 * (a decode() rejection counts as settled, exactly like a successful load).
 *
 * BUG FIX: the 10s Node-side deadline used to THROW on expiry, which — for a
 * page carrying any image that can never settle by the above race (the
 * hidden-subtree case is common and not fixable client-side) — hard-failed
 * the whole settle stage before it ever reached the quiescence wait. The
 * deadline now degrades gracefully: it returns `{ timedOut: true,
 * pendingCount }` instead of throwing. A timed-out image phase is a
 * mutation-pending signal, not a step failure — the caller proceeds to the
 * quiescence wait, which observes any genuinely-late settling honestly.
 */
async function awaitAllImagesLoadOrError(pg: Page): Promise<ImageAwaitResult> {
  const pendingCount = await pg.evaluate(
    () => Array.from(document.querySelectorAll<HTMLImageElement>("img")).filter((img) => !img.complete).length
  );

  const settleAll = pg.evaluate(async () => {
    const imgs = Array.from(document.querySelectorAll<HTMLImageElement>("img"));
    await Promise.allSettled(
      imgs.map((img) => {
        if (img.complete) return Promise.resolve();
        return new Promise<void>((resolve) => {
          const done = () => resolve();
          img.addEventListener("load", done, { once: true });
          img.addEventListener("error", done, { once: true });
          // decode() rejection (e.g. a broken image) counts as settled too —
          // whichever of decode()/load/error fires first wins the race.
          img.decode().then(done, done);
        });
      })
    );
  });

  try {
    await withDeadline(settleAll, 10_000);
    return { timedOut: false, pendingCount: 0 };
  } catch {
    return { timedOut: true, pendingCount };
  }
}

/**
 * Wait for DOM quiescence: install a MutationObserver on documentElement
 * (childList + subtree + attributes + characterData) that IGNORES mutations
 * whose target sits inside any hide/mask-selector-matched subtree, then poll
 * a `lastMutation` timestamp in a loop that interleaves a virtual
 * `clock.runFor(100)` (when the clock is installed — this is what lets
 * rAF/timer-driven animation progress deterministically under a frozen
 * wall-clock) with a REAL `waitForTimeout(25)` each iteration (real-async
 * work — fetches, hydration — needs actual wall time to land; a clock-only
 * loop would declare quiescence in near-zero wall time). Quiescence is
 * reached once `windowMs` has elapsed (per the page's own `Date.now()`,
 * which is virtual when the clock is installed) since the last counted
 * mutation. Hard-bounded by `timeoutMs` of accumulated virtual+wall budget.
 */
async function waitForQuiescence(
  pg: Page,
  clockInstalled: boolean,
  ignoredSelectors: string[],
  windowMs: number,
  timeoutMs: number
): Promise<"reached" | "timeout"> {
  await pg.evaluate((selectors: string[]) => {
    const w = window as unknown as Record<string, unknown>;
    w["__mmLastMutation"] = Date.now();

    function insideIgnored(node: Node): boolean {
      let el: Element | null = node.nodeType === 1 ? (node as Element) : node.parentElement;
      while (el) {
        for (const sel of selectors) {
          try {
            if (el.matches(sel)) return true;
          } catch {
            // Invalid/unsupported selector: never treat as a match.
          }
        }
        el = el.parentElement;
      }
      return false;
    }

    const observer = new MutationObserver((mutations) => {
      for (const m of mutations) {
        if (!insideIgnored(m.target)) {
          w["__mmLastMutation"] = Date.now();
          break;
        }
      }
    });
    observer.observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      characterData: true,
    });
    w["__mmSettleObserver"] = observer;
  }, ignoredSelectors);

  let budget = 0;
  let status: "reached" | "timeout" = "timeout";

  while (budget < timeoutMs) {
    if (clockInstalled) {
      await pg.clock.runFor(100);
      budget += 100;
    }
    await pg.waitForTimeout(25);
    budget += 25;

    const { now, last } = await pg.evaluate(() => ({
      now: Date.now(),
      last: (window as unknown as Record<string, unknown>)["__mmLastMutation"] as number,
    }));
    if (now - last >= windowMs) {
      status = "reached";
      break;
    }
  }

  await pg.evaluate(() => {
    const w = window as unknown as Record<string, unknown>;
    (w["__mmSettleObserver"] as MutationObserver | undefined)?.disconnect();
  });

  return status;
}

/** Result of one full-settle pass, applied by the caller onto `det`. */
interface FullSettleResult {
  scrollIneffective: boolean;
  growthCapped: boolean;
  quiescence: "reached" | "timeout";
}

/**
 * The evolved settle stage (port-parity U12, "full" mode):
 *   1. Scroll-through in viewport-height steps, re-reading scrollHeight each
 *      step; growth-cap guard against infinitely-growing pages.
 *   2. Fixed dwell per step (clock.runFor when installed, else a real wait).
 *   3. Lazy-image await (load-or-error) at the bottom.
 *   4. Return to top, re-await fonts, lazy-image await again.
 *   5. Quiescence wait (MutationObserver, hide/mask-aware, hard-bounded).
 *   6. Scroll-ineffective detection (transform-scroll sites): if scrollY is
 *      unchanged after the first two scroll steps, stop scrolling further
 *      but still run image-await + quiescence.
 *
 * Throws on any unexpected failure — the caller wraps this in the same
 * `step()` helper every other pipeline step uses, so a throw here is
 * recorded as `determinism.settle = "failed"` and joins the existing
 * shouldRetryWithoutFreeze trigger set exactly like any other step failure.
 *
 * BUG FIX: the body runs inside a try/finally. A settle failure must never
 * leave extraction running against a scrolled page (the documented WP-H
 * lesson — log-and-continue must not silently corrupt capture), so on ANY
 * exit — success or throw — scroll position is unconditionally forced back
 * to (0, 0), and fonts are best-effort re-awaited on their own short bound
 * (a stuck finally-path `fonts.ready` must not itself hang the pipeline).
 * The scroll restore is attempted even if that best-effort font re-await
 * fails, and vice versa; neither failure replaces/masks the original error.
 */
async function runFullSettle(
  pg: Page,
  clockInstalled: boolean,
  config: StabilizationConfig,
  hideSelectors: string[],
  maskSelectors: string[],
  log: (msg: string) => void
): Promise<FullSettleResult> {
  const dwellMs = config.settleDwellMs ?? DEFAULT_SETTLE_DWELL_MS;
  const maxSettleSteps = config.maxSettleSteps ?? DEFAULT_MAX_SETTLE_STEPS;
  const windowMs = config.quiescenceWindowMs ?? DEFAULT_QUIESCENCE_WINDOW_MS;
  const timeoutMs = config.quiescenceTimeoutMs ?? DEFAULT_QUIESCENCE_TIMEOUT_MS;

  try {
    const viewportHeight = await pg.evaluate(() => window.innerHeight);
    let scrollHeight = await pg.evaluate(() => document.documentElement.scrollHeight);
    const initialScrollHeight = scrollHeight;
    const maxSteps = Math.max(
      Math.ceil((3 * initialScrollHeight) / Math.max(viewportHeight, 1)),
      maxSettleSteps
    );

    let current = 0;
    let stepCount = 0;
    let growthCapped = false;
    let scrollIneffective = false;
    const scrollYSamples: number[] = [];

    // NOTE on the loop condition: a genuine `position:fixed; overflow:hidden`
    // transform-scroll site (the real-world pattern that fully disables native
    // document scrolling) typically reports `document.documentElement.scrollHeight`
    // clamped to the viewport height — INDISTINGUISHABLE, by that reading alone,
    // from a page that's simply shorter than the viewport. The only way to tell
    // them apart is to attempt scrolling and observe `scrollY`, so the FIRST TWO
    // steps are always attempted regardless of what `scrollHeight` claims;
    // `current < scrollHeight` only gates steps AFTER the two-sample
    // ineffectiveness check has had its chance to run. (Residual limitation:
    // a legitimately short, non-scrollable page will also read as "ineffective"
    // under this scheme — recorded faithfully rather than silently guessed at.)
    while (scrollYSamples.length < 2 || current < scrollHeight) {
      current += viewportHeight;
      stepCount += 1;
      if (stepCount > maxSteps) {
        growthCapped = true;
        break;
      }

      await pg.evaluate((y: number) => window.scrollTo(0, y), current);
      if (clockInstalled) {
        await pg.clock.runFor(dwellMs);
      } else {
        await pg.waitForTimeout(Math.min(dwellMs, 200));
      }

      const scrollY = await pg.evaluate(() => window.scrollY);
      scrollYSamples.push(scrollY);
      if (scrollYSamples.length === 2 && scrollYSamples[0] === scrollYSamples[1]) {
        scrollIneffective = true;
        break;
      }

      // Re-read scrollHeight each step — the page may have grown (e.g. an
      // infinite feed appending content on scroll).
      scrollHeight = await pg.evaluate(() => document.documentElement.scrollHeight);
    }

    // Lazy-image await at the bottom (or wherever the loop stopped). A phase
    // timeout degrades gracefully (see awaitAllImagesLoadOrError) — it is a
    // mutation-pending signal, not a step failure; the quiescence wait below
    // observes any genuinely-late settling honestly.
    const bottomImageAwait = await awaitAllImagesLoadOrError(pg);
    if (bottomImageAwait.timedOut) {
      log(
        `[stabilizer] settle: image-await (bottom) phase timed out (pendingCount=${bottomImageAwait.pendingCount}); proceeding to quiescence`
      );
    }

    // Return to top, re-await fonts (existing pattern), lazy-image await again.
    await pg.evaluate(() => window.scrollTo(0, 0));
    await withDeadline(pg.evaluate(() => document.fonts.ready), 10_000);
    const topImageAwait = await awaitAllImagesLoadOrError(pg);
    if (topImageAwait.timedOut) {
      log(
        `[stabilizer] settle: image-await (top) phase timed out (pendingCount=${topImageAwait.pendingCount}); proceeding to quiescence`
      );
    }

    // Quiescence wait.
    const quiescence = await waitForQuiescence(
      pg,
      clockInstalled,
      [...hideSelectors, ...maskSelectors],
      windowMs,
      timeoutMs
    );

    return { scrollIneffective, growthCapped, quiescence };
  } finally {
    try {
      await pg.evaluate(() => window.scrollTo(0, 0));
    } catch (err) {
      log(`[stabilizer] settle: failed to restore scroll position: ${err}`);
    }
    try {
      await withDeadline(pg.evaluate(() => document.fonts.ready), 2_000);
    } catch {
      // Best-effort only — a stuck/slow fonts.ready here must not block the
      // caller further. Scroll restoration (the hard requirement) has
      // already been attempted above regardless of this outcome.
    }
  }
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
        | "settle"
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

    // Step 8: settle stage (port-parity U12). Dispatches on
    // config.settleMode — "legacy" is BYTE-FOR-BYTE today's lazyLoadPass
    // (characterized in tests/stabilizer.test.ts before this dispatch was
    // introduced) and stays the zod-schema default in this unit; the
    // default flip to "full" is a separate later commit.
    if (config.settleMode === "off") {
      // Config-file only (no CLI flag maps to this) — skip step 8 entirely.
      det.lazyLoadPass = "skipped";
      det.settle = "skipped";
      det.quiescence = "notRun";
    } else if (config.settleMode === "legacy") {
      det.settle = "skipped";
      det.quiescence = "notRun";
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
    } else {
      // "full": the evolved settle stage.
      det.lazyLoadPass = "skipped";
      det.quiescence = "notRun";
      await step("settle", async () => {
        const result = await runFullSettle(pg, clockIsInstalled, config, hideSelectors, maskSelectors, log);
        if (result.scrollIneffective) det.settleScrollIneffective = true;
        if (result.growthCapped) det.settleGrowthCapped = true;
        det.quiescence = result.quiescence;
      });
    }

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
