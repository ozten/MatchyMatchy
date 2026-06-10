/**
 * Capture entry point. Reads one CaptureConfig JSON from stdin,
 * runs the capture or doctor mode, writes artifacts and prints
 * exactly one JSON line to stdout.
 */
import * as fs from "fs";
import * as path from "path";
import * as readline from "readline";
import type { Browser, BrowserContext, Page, Request, ConsoleMessage as PwConsoleMessage } from "playwright";
import { CaptureConfigSchema } from "./schema.js";
import type { CaptureBundle, NetworkRequest, ConsoleMessage } from "./schema.js";
import { launchBrowser, createContext } from "./browser-runner.js";
import { stabilize } from "./stabilizer.js";
import { extractPageModel } from "./extract/page-model.js";
import { redactUrl, normalizeText } from "./normalize.js";
import { probeLinks } from "./probe.js";

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
  let chromiumOk = false;
  let chromiumVersion = "";
  let browser: Browser | null = null;
  try {
    browser = await launchBrowser();
    chromiumVersion = browser.version();
    chromiumOk = true;
  } catch (err) {
    log(`[doctor] Chromium launch failed: ${err}`);
  } finally {
    if (browser) {
      try { await browser.close(); } catch { /* ignore */ }
    }
  }

  if (chromiumOk) {
    printOk({
      node: process.version,
      playwright: pwVersion,
      chromium: { ok: true, version: chromiumVersion },
    });
    process.exit(0);
  } else {
    printOk({
      node: process.version,
      playwright: pwVersion,
      chromium: { ok: false, version: "" },
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

    context.on("requestfinished", (req: Request) => {
      const reqUrl = redactUrl(req.url(), redactParams);
      const response = req.response();
      // Scan from end for last matching entry (backwards-compat with ES2022 target)
      let idx = -1;
      for (let i = networkRequests.length - 1; i >= 0; i--) {
        const r = networkRequests[i];
        if (r && (r.url === reqUrl || r.url === req.url())) { idx = i; break; }
      }
      if (idx !== -1) {
        void (async () => {
          try {
            const resp = await response;
            if (resp) {
              const entry = networkRequests[idx];
              if (entry) (entry as { status: number }).status = resp.status();
            }
          } catch { /* ignore */ }
        })();
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

    // Run stabilization
    const { page, determinism, mainResponse, redirectChain } = await stabilize(
      context,
      url,
      stabilization,
      hideSelectors,
      maskSelectors,
      clickBeforeCapture,
      log
    );

    // Attach console listener after page is created by stabilize
    page.on("console", (msg: PwConsoleMessage) => {
      consoleMessages.push({
        level: msg.type(),
        text: normalizeText(msg.text(), maxTextLength),
      });
    });

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

    // Allow async requestfinished handlers a beat to complete
    await new Promise<void>((r) => setTimeout(r, 100));

    // Build the bundle
    const bundle: CaptureBundle = {
      schemaVersion: "1.0",
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
        })),
        landmarks: pageModelRaw.landmarks,
        network: { requests: networkRequests },
        console: consoleMessages,
        a11y: { violations: [] },
        linkProbes: linkProbeResults,
      },
      computedStyles: {},
      screenshots: {
        fullPage: `${viewport.name}/${prefix}.png`,
        viewport: `${viewport.name}/${prefix}-vp.png`,
      },
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
