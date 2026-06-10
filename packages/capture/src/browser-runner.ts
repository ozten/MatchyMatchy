import { chromium, Browser, BrowserContext } from "playwright";
import type { ViewportConfig } from "./schema.js";

/**
 * Launch a Chromium browser in headless mode.
 * Never passes --no-sandbox; runs as an unprivileged user.
 */
export async function launchBrowser(): Promise<Browser> {
  return chromium.launch({
    headless: true,
    // Never pass --no-sandbox
    args: [],
  });
}

/**
 * Create an isolated BrowserContext with deterministic settings.
 */
export async function createContext(
  browser: Browser,
  viewport: ViewportConfig
): Promise<BrowserContext> {
  return browser.newContext({
    viewport: {
      width: viewport.width,
      height: viewport.height,
    },
    deviceScaleFactor: viewport.dsf,
    locale: "en-US",
    timezoneId: "UTC",
    colorScheme: "light",
    // Fixed UA suffix for identification
    userAgent: `Mozilla/5.0 (compatible; matchy/0.1 capture)`,
  });
}
