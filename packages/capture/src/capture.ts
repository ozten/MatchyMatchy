/**
 * Capture entry point. Reads one CaptureConfig JSON from stdin,
 * runs the capture or doctor mode, writes artifacts and prints
 * exactly one JSON line to stdout.
 */
import * as fs from "fs";
import * as path from "path";
import * as readline from "readline";
import { chromium } from "playwright";
import type { Browser, BrowserContext, Page, Request, Response, ConsoleMessage as PwConsoleMessage } from "playwright";
import { CaptureConfigSchema } from "./schema.js";
import type { CaptureBundle, NetworkRequest, ConsoleMessage } from "./schema.js";
import { launchBrowser, createContext } from "./browser-runner.js";
import { stabilize } from "./stabilizer.js";
import { extractPageModel } from "./extract/page-model.js";
import { isProbeEligible, runHitTestProbe } from "./extract/hit-test.js";
import type { HitTestProbeInput } from "./extract/hit-test.js";
import { redactUrl, normalizeText } from "./normalize.js";
import { probeLinks } from "./probe.js";
import axeCore from "axe-core";

// axe-core exposes the injectable script as a .source property on the default export
const axeSource: string = (axeCore as unknown as { source: string }).source;

// Get playwright version from package.json
function getPlaywrightVersion(): string {
  try {
    // Try to read from node_modules playwright's package.json
    const dirs = [
      path.join(process.cwd(), "node_modules", "playwright", "package.json"),
      path.join(__dirname, "..", "node_modules", "playwright", "package.json"),
      path.join(__dirname, "..", "..", "node_modules", "playwright", "package.json"),
    ];
    for (const p of dirs) {
      if (fs.existsSync(p)) {
        const pkg = JSON.parse(fs.readFileSync(p, "utf-8")) as { version: string };
        return pkg.version;
      }
    }
  } catch {
    // ignore
  }
  return "1.60.0"; // fallback to pinned version
}

function printOk(data: Record<string, unknown>): void {
  process.stdout.write(JSON.stringify({ ok: true, ...data }) + "\n");
}

function printError(code: string, message: string): void {
  process.stdout.write(
    JSON.stringify({ ok: false, error: { code, message } }) + "\n"
  );
}

function log(msg: string): void {
  process.stderr.write(msg + "\n");
}

async function readStdin(): Promise<string> {
  return new Promise((resolve, reject) => {
    const rl = readline.createInterface({ input: process.stdin });
    const lines: string[] = [];
    rl.on("line", (line) => lines.push(line));
    rl.on("close", () => resolve(lines.join("\n")));
    rl.on("error", reject);
  });
}

async function runDoctor(): Promise<void> {
  const pwVersion = getPlaywrightVersion();
  let launchSucceeded = false;
  let chromiumVersion = "";
  let browser: Browser | null = null;
  let launchError: string | null = null;

  // Get the path Playwright would use for chromium, even before launching.
  let executablePath = "";
  try {
    executablePath = chromium.executablePath();
  } catch (err) {
    log(`[doctor] chromium.executablePath() failed: ${err}`);
  }

  const executableExists = executablePath !== "" && fs.existsSync(executablePath);

  try {
    browser = await launchBrowser();
    chromiumVersion = browser.version();
    launchSucceeded = true;
  } catch (err) {
    launchError = String(err);
    log(`[doctor] Chromium launch failed: ${err}`);
  } finally {
    if (browser) {
      try { await browser.close(); } catch { /* ignore */ }
    }
  }

  // chromium.ok requires both a successful launch AND the executable existing on disk.
  const chromiumOk = launchSucceeded && executableExists;

  // browsersPath: the env var that controls where Playwright looks for browsers.
  const browsersPath: string | null = process.env["PLAYWRIGHT_BROWSERS_PATH"] ?? null;

  const chromiumInfo = {
    ok: chromiumOk,
    version: chromiumVersion,
    executablePath,
    exists: executableExists,
    launchError,
  };

  if (chromiumOk) {
    printOk({
      node: process.version,
      playwright: pwVersion,
      chromium: chromiumInfo,
      browsersPath,
    });
    process.exit(0);
  } else {
    printOk({
      node: process.version,
      playwright: pwVersion,
      chromium: chromiumInfo,
      browsersPath,
    });
    process.exit(1);
  }
}

async function runCapture(configRaw: unknown): Promise<void> {
  // Parse and validate config
  const parseResult = CaptureConfigSchema.safeParse(configRaw);
  if (!parseResult.success) {
    printError("INVALID_CONFIG", parseResult.error.message);
    process.exit(1);
  }
  const config = parseResult.data;

  if (!config.url || !config.outDir || !config.prefix) {
    printError("MISSING_REQUIRED", "url, outDir, and prefix are required for capture mode");
    process.exit(1);
  }

  const { url, outDir, prefix, viewport, stabilization, hideSelectors, maskSelectors, clickBeforeCapture, maxTextLength, redactParams, probeLinks: configProbeLinks } = config;

  // Create output directories
  const viewportDir = path.join(outDir, viewport.name);
  fs.mkdirSync(viewportDir, { recursive: true });

  let browser: Browser | null = null;
  let context: BrowserContext | null = null;
  // exitCode is set before the finally block runs, then process.exit fires after cleanup.
  let exitCode = 0;

  try {
    browser = await launchBrowser();
    const pwVersion = getPlaywrightVersion();
    const chromiumBuild = browser.version();

    context = await createContext(browser, viewport);

    // Network request recording via context-level listeners.
    const networkRequests: NetworkRequest[] = [];
    const consoleMessages: ConsoleMessage[] = [];

    context.on("request", (req: Request) => {
      const reqUrl = redactUrl(req.url(), redactParams);
      // Authorization/Cookie headers are never recorded — only URL is stored.
      networkRequests.push({
        url: reqUrl,
        status: 0,
        type: req.resourceType(),
        failed: false,
      });
    });

    // Synchronously record status from the response event (reliable; fires for all
    // completed transactions including 4xx/5xx; requestfailed does NOT fire for these).
    context.on("response", (resp: Response) => {
      const respUrl = redactUrl(resp.url(), redactParams);
      let idx = -1;
      for (let i = networkRequests.length - 1; i >= 0; i--) {
        const r = networkRequests[i];
        if (r && (r.url === respUrl || r.url === resp.url())) { idx = i; break; }
      }
      if (idx !== -1) {
        const entry = networkRequests[idx];
        if (entry) (entry as { status: number }).status = resp.status();
      }
    });

    context.on("requestfailed", (req: Request) => {
      const reqUrl = redactUrl(req.url(), redactParams);
      let idx = -1;
      for (let i = networkRequests.length - 1; i >= 0; i--) {
        const r = networkRequests[i];
        if (r && (r.url === reqUrl || r.url === req.url())) { idx = i; break; }
      }
      if (idx !== -1) {
        const entry = networkRequests[idx];
        if (entry) (entry as { failed: boolean }).failed = true;
      }
    });

    // Build the console-collection callback and pass it into stabilize so that
    // the listener is attached BEFORE page.goto (pre-navigation), capturing
    // load-time console messages. The reset on each call handles the retry path
    // (where stabilize creates a second page) — only the surviving page's
    // messages are kept.
    const onPageCreated = (p: Page): void => {
      consoleMessages.length = 0; // reset so only surviving page's console is kept
      p.on("console", (msg: PwConsoleMessage) => {
        consoleMessages.push({
          level: msg.type(),
          text: normalizeText(msg.text(), maxTextLength),
        });
      });
    };

    // Run stabilization (onPageCreated attaches console listener pre-navigation)
    const { page, determinism, mainResponse, redirectChain } = await stabilize(
      context,
      url,
      stabilization,
      hideSelectors,
      maskSelectors,
      clickBeforeCapture,
      log,
      onPageCreated
    );

    // Extract page model
    const pageModelRaw = await page.evaluate(extractPageModel, maxTextLength);

    // Get page metadata
    const finalUrl = page.url();
    const statusCode = mainResponse?.status() ?? 0;
    const title = await page.evaluate(() => document.title);
    const metaDescription = await page.evaluate(() => {
      const meta = document.querySelector('meta[name="description"]');
      return meta?.getAttribute("content") ?? "";
    });
    const canonical = await page.evaluate(() => {
      const link = document.querySelector('link[rel="canonical"]');
      return link?.getAttribute("href") ?? null;
    });
    const lang = await page.evaluate(() => document.documentElement.lang ?? "");

    // Redact redirect chain
    const redactedRedirectChain = redirectChain.map((u) => redactUrl(u, redactParams));

    // Probe links (new-side only, when enabled) — failures must never fail the capture
    let linkProbeResults: CaptureBundle["page"]["linkProbes"] = [];
    if (configProbeLinks) {
      try {
        linkProbeResults = await probeLinks(
          pageModelRaw.nodes.map((n) => ({
            id: n.id,
            kind: n.kind as CaptureBundle["page"]["nodes"][0]["kind"],
            role: n.role,
            text: n.text,
            accName: n.accName,
            href: n.href,
            imageAlt: n.imageAlt,
            bbox: n.bbox,
            seqIndex: n.seqIndex,
            anchors: n.anchors,
            cssSelector: n.cssSelector,
            rawHref: n.rawHref,
            src: n.src,
            naturalWidth: n.naturalWidth,
            naturalHeight: n.naturalHeight,
            loaded: n.loaded,
            headingLevel: n.headingLevel,
          })),
          url,
          redactParams
        );
      } catch (probeErr) {
        log(`[capture] probeLinks failed (non-fatal): ${probeErr}`);
      }
    }

    // Screenshots
    const fullPagePath = path.join(viewportDir, `${prefix}.png`);
    const viewportPath = path.join(viewportDir, `${prefix}-vp.png`);

    await page.screenshot({ path: fullPagePath, fullPage: true, animations: "disabled" });
    await page.screenshot({ path: viewportPath, fullPage: false, animations: "disabled" });

    // Allow any in-flight response listeners a beat to record statuses
    await new Promise<void>((r) => setTimeout(r, 100));

    // Port-parity U6: capture-time clickable-area hit-test probe. Runs after
    // both screenshots (so scrolling for the probe can never pollute captured
    // visual state) and before the clock-resume/axe step. No config off-switch:
    // this stage always runs. A thrown error is recorded as "failed" and the
    // capture CONTINUES — the probe is never allowed to abort a capture.
    //
    // Deviation from the retry-without-freeze machinery (stabilizer.ts
    // shouldRetryWithoutFreeze / the capture.ts retry path): that machinery is
    // strictly scoped to stabilize()'s own pipeline, which has already
    // returned (and the page has already been used for extraction and both
    // screenshots) by the time this stage runs. Retrying here would require
    // re-navigating and redoing extraction/screenshots, which is out of scope
    // for this unit — a probe failure is recorded as "failed" without
    // triggering a retry.
    let hitTests: CaptureBundle["hitTests"];
    try {
      const eligibleNodes: HitTestProbeInput[] = pageModelRaw.nodes
        .filter((n) => isProbeEligible(n.kind, n.hasOnclick))
        .map((n) => ({ id: n.id, cssSelector: n.cssSelector, bbox: n.bbox }));

      if (eligibleNodes.length > 0) {
        const probeResult = await page.evaluate(runHitTestProbe, eligibleNodes);
        if (Object.keys(probeResult).length > 0) {
          hitTests = probeResult as CaptureBundle["hitTests"];
        }
      }
      determinism.hitTestProbe = "ran";
    } catch (hitTestErr) {
      determinism.hitTestProbe = "failed";
      log(`[capture] hit-test probe failed (non-fatal): ${hitTestErr}`);
    }

    // Run axe-core after screenshots + page-model extraction (determinism: axe runs last
    // so resuming the frozen clock cannot affect already-captured screenshots/styles/model).
    let violations: unknown[] = [];
    try {
      // If the clock was installed (freezeTime path), resume it before running axe.
      // axe's internal async machinery uses setTimeout, which hangs under a frozen clock.
      // resume() restores natural time; this is safe because all visual capture is done.
      if (determinism.timeFrozen === "ran") {
        await page.clock.resume();
      }

      // Inject axe-core source into the page then run it
      const axeRunResult = await new Promise<{ violations: unknown[] }>((resolve, reject) => {
        const timer = setTimeout(
          () => reject(new Error("axe run deadline exceeded (20000ms)")),
          20000
        );
        page.addScriptTag({ content: axeSource })
          .then(() =>
            page.evaluate(async () =>
              (window as unknown as { axe: { run: (doc: Document, opts: Record<string, unknown>) => Promise<{ violations: unknown[] }> } })
                .axe.run(document, { resultTypes: ["violations"] })
            )
          )
          .then((result) => { clearTimeout(timer); resolve(result); })
          .catch((err) => { clearTimeout(timer); reject(err); });
      });

      violations = axeRunResult.violations ?? [];
    } catch (axeErr) {
      log(`[capture] axe failed (non-fatal): ${axeErr}`);
      violations = [];
    }

    // Build the bundle
    const bundle: CaptureBundle = {
      schemaVersion: "1.1",
      capturedAt: new Date().toISOString(),
      viewport: { name: viewport.name, width: viewport.width, height: viewport.height, dsf: viewport.dsf },
      environment: { os: process.platform, chromiumBuild, playwright: pwVersion, dsf: viewport.dsf },
      determinism,
      page: {
        url,
        finalUrl,
        redirectChain: redactedRedirectChain,
        statusCode,
        title: normalizeText(title, maxTextLength),
        metaDescription: normalizeText(metaDescription, maxTextLength),
        canonical: canonical ? normalizeText(canonical, maxTextLength) : null,
        lang: lang ?? "",
        pageHeight: pageModelRaw.pageHeight,
        nodes: pageModelRaw.nodes.map((n) => ({
          id: n.id,
          kind: n.kind as CaptureBundle["page"]["nodes"][0]["kind"],
          role: n.role,
          text: n.text,
          accName: n.accName,
          href: n.href,
          imageAlt: n.imageAlt,
          bbox: n.bbox,
          seqIndex: n.seqIndex,
          anchors: n.anchors,
          cssSelector: n.cssSelector,
          rawHref: n.rawHref,
          src: n.src,
          naturalWidth: n.naturalWidth,
          naturalHeight: n.naturalHeight,
          loaded: n.loaded,
          headingLevel: n.headingLevel,
          hasOnclick: n.hasOnclick,
        })),
        landmarks: pageModelRaw.landmarks,
        landmarkRects: pageModelRaw.landmarkRects,
        network: { requests: networkRequests },
        console: consoleMessages,
        a11y: { violations },
        linkProbes: linkProbeResults,
      },
      computedStyles: pageModelRaw.computedStyles,
      styleCandidates: pageModelRaw.styleCandidates,
      screenshots: {
        fullPage: `${viewport.name}/${prefix}.png`,
        viewport: `${viewport.name}/${prefix}-vp.png`,
      },
      hitTests,
      // Port-parity U9: pseudoElements is omitted (not an empty object) when
      // the scan found no painted ::before/::after entries.
      pseudoElements:
        Object.keys(pageModelRaw.pseudoElements).length > 0 ? pageModelRaw.pseudoElements : undefined,
      pseudoTruncated: pageModelRaw.pseudoTruncated,
    };

    // Zod validation before writing
    const { CaptureBundleSchema: schema } = await import("./schema.js");
    const validationResult = schema.safeParse(bundle);
    if (!validationResult.success) {
      log(`[capture] Bundle validation failed: ${validationResult.error.message}`);
      printError("BUNDLE_INVALID", validationResult.error.message);
      exitCode = 1;
      return; // fall through to finally for cleanup
    }

    // Write bundle
    const bundlePath = path.join(viewportDir, `${prefix}.bundle.json`);
    fs.writeFileSync(bundlePath, JSON.stringify(bundle, null, 2));

    printOk({ bundlePath });
    // exitCode stays 0
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    log(`[capture] Fatal error: ${message}`);
    printError("CAPTURE_FAILED", message);
    exitCode = 1;
  } finally {
    // Always close browser resources before exiting
    if (context) {
      try { await context.close(); } catch { /* ignore */ }
    }
    if (browser) {
      try { await browser.close(); } catch { /* ignore */ }
    }
  }

  process.exit(exitCode);
}

async function main(): Promise<void> {
  const input = await readStdin();
  let configRaw: unknown;
  try {
    configRaw = JSON.parse(input);
  } catch (err) {
    printError("INVALID_JSON", `Failed to parse stdin as JSON: ${err}`);
    process.exit(1);
  }

  const raw = configRaw as Record<string, unknown>;
  const mode = raw["mode"];

  if (mode === "doctor") {
    await runDoctor();
  } else if (mode === "capture") {
    await runCapture(configRaw);
  } else {
    printError("INVALID_MODE", `Unknown mode: ${String(mode)}`);
    process.exit(1);
  }
}

main().catch((err) => {
  const message = err instanceof Error ? err.message : String(err);
  printError("UNHANDLED_ERROR", message);
  process.exit(1);
});
