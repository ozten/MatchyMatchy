/**
 * Link-probing module for M2 egress policy (spec §10.3).
 *
 * Exports:
 *  - decideEgress: pure, unit-testable egress gate function
 *  - probeLinks: async link-probing with concurrency, redirect following, and cap
 */
import * as dns from "dns";
import { getDomain } from "tldts";
import { redactUrl } from "./normalize.js";
import type { LinkProbe, SemanticNode } from "./schema.js";

// ─── Constants ───────────────────────────────────────────────────────────────

export const MAX_LINK_PROBES = 50;
const MAX_REDIRECT_HOPS = 5;
const PROBE_CONCURRENCY = 4;
const PROBE_TIMEOUT_MS = 5000;

// ─── Private-range detection ─────────────────────────────────────────────────

/**
 * Returns true when the given IPv4 address (dotted-decimal string) falls in a
 * private / loopback / link-local / metadata range.
 */
function isPrivateIPv4(addr: string): boolean {
  const parts = addr.split(".").map(Number);
  if (parts.length !== 4 || parts.some((p) => isNaN(p) || p < 0 || p > 255)) {
    return false;
  }
  const [a, b] = parts as [number, number, number, number];
  // 10.0.0.0/8
  if (a === 10) return true;
  // 172.16.0.0/12
  if (a === 172 && b >= 16 && b <= 31) return true;
  // 192.168.0.0/16
  if (a === 192 && b === 168) return true;
  // 169.254.0.0/16 (link-local / cloud metadata)
  if (a === 169 && b === 254) return true;
  // 127.0.0.0/8 (loopback)
  if (a === 127) return true;
  return false;
}

/**
 * Returns true when the given IPv6 address string is in a private range:
 * ::1, fd00::/8, or ::ffff:<private-ipv4> (IPv4-mapped).
 */
function isPrivateIPv6(addr: string): boolean {
  const lower = addr.toLowerCase();
  // Loopback
  if (lower === "::1") return true;
  // fd00::/8 — ULA (unique local addresses)
  if (lower.startsWith("fd")) return true;
  // ::ffff:a.b.c.d  IPv4-mapped — check the embedded IPv4
  const mapped4 = lower.match(/^::ffff:(\d+\.\d+\.\d+\.\d+)$/);
  if (mapped4?.[1]) return isPrivateIPv4(mapped4[1]);
  // ::ffff:aabb:ccdd hex form
  const mappedHex = lower.match(/^::ffff:([0-9a-f]{1,4}):([0-9a-f]{1,4})$/);
  if (mappedHex?.[1] && mappedHex?.[2]) {
    const hi = parseInt(mappedHex[1], 16);
    const lo = parseInt(mappedHex[2], 16);
    const ipv4 = `${(hi >> 8) & 0xff}.${hi & 0xff}.${(lo >> 8) & 0xff}.${lo & 0xff}`;
    return isPrivateIPv4(ipv4);
  }
  return false;
}

function isPrivateAddress(addr: string): boolean {
  // Simple heuristic: presence of ':' → IPv6
  return addr.includes(":") ? isPrivateIPv6(addr) : isPrivateIPv4(addr);
}

// ─── decideEgress ─────────────────────────────────────────────────────────────

export type EgressDecision = "scheme" | "external-scope" | "private-address";

export interface EgressContext {
  /** Hostname of the input page URL (already resolved and used for comparison) */
  inputHost: string;
  /** Registrable domain of the input page URL (e.g. "example.com"), or null for IPs/localhost */
  inputRegistrableDomain: string | null;
  /** True when the input page URL's own host resolves to a private address */
  inputResolvesPrivate: boolean;
  /**
   * Resolved addresses for the target host (from DNS lookup), or null when we
   * do not have DNS information (pure-function path in tests or defensive guard).
   */
  targetAddresses: string[] | null;
}

/**
 * Pure egress gate — returns null (allow) or a refusal reason.
 *
 * Checks in order:
 *  1. scheme: only http / https pass
 *  2. external-scope: registrable domain must match input (fallback to hostname equality)
 *  3. private-address: any resolved address in a private range is refused
 *     UNLESS ctx.inputResolvesPrivate is true (local fixture serving exception)
 */
export function decideEgress(
  targetUrl: URL,
  ctx: EgressContext
): null | EgressDecision {
  // 1. Scheme check
  if (targetUrl.protocol !== "http:" && targetUrl.protocol !== "https:") {
    return "scheme";
  }

  // 2. Scope check
  const targetHost = targetUrl.hostname;
  const targetRegistrable = getDomain(targetHost) ?? null;

  if (
    targetRegistrable !== null &&
    ctx.inputRegistrableDomain !== null
  ) {
    // Both have registrable domains — they must match
    if (targetRegistrable !== ctx.inputRegistrableDomain) {
      return "external-scope";
    }
  } else {
    // One or both lack registrable domains (IP literals, localhost) →
    // fall back to exact hostname equality
    if (targetHost !== ctx.inputHost) {
      return "external-scope";
    }
  }

  // 3. Private-address check (only when we have resolved addresses)
  if (ctx.targetAddresses !== null && !ctx.inputResolvesPrivate) {
    for (const addr of ctx.targetAddresses) {
      if (isPrivateAddress(addr)) {
        return "private-address";
      }
    }
  }

  return null;
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/** Strip fragment and return the URL string. */
function stripFragment(urlStr: string): string {
  try {
    const u = new URL(urlStr);
    u.hash = "";
    return u.toString();
  } catch {
    return urlStr;
  }
}

/** Resolve a potentially-relative URL against a base, stripping fragment. Returns null on error. */
function resolveUrl(href: string, base: string): string | null {
  try {
    return stripFragment(new URL(href, base).toString());
  } catch {
    return null;
  }
}

/**
 * Determine whether the input URL's host resolves to a private address.
 * Returns false on any error (fail-open: don't accidentally block real sites).
 */
async function inputResolvesPrivate(inputUrl: URL): Promise<boolean> {
  const host = inputUrl.hostname;
  // IP literals — check directly without DNS
  try {
    const u = new URL(`http://${host}`);
    if (u.hostname !== host) {
      // Bracketed IPv6 — strip brackets
    }
  } catch {
    // ignore
  }
  // Check if the hostname itself is a private address literal
  if (isPrivateAddress(host)) return true;
  if (host === "localhost") return true;

  try {
    const addrs = await dns.promises.lookup(host, { all: true });
    return addrs.some((a) => isPrivateAddress(a.address));
  } catch {
    return false;
  }
}

// ─── probeLinks ──────────────────────────────────────────────────────────────

/**
 * Probe all same-site links extracted from the page nodes.
 *
 * @param nodes        SemanticNode[] from the page model
 * @param inputUrl     The page's input URL string (used for scope checks)
 * @param redactParams Query parameter names to redact in recorded URLs
 */
export async function probeLinks(
  nodes: SemanticNode[],
  inputUrl: string,
  redactParams: string[]
): Promise<LinkProbe[]> {
  let parsedInput: URL;
  try {
    parsedInput = new URL(inputUrl);
  } catch {
    // Unparseable input URL — return empty
    return [];
  }

  // Determine whether input host resolves to a private address (done once)
  const inputPrivate = await inputResolvesPrivate(parsedInput);

  const inputHost = parsedInput.hostname;
  const inputRegistrableDomain = getDomain(inputHost) ?? null;

  const baseCtx: Omit<EgressContext, "targetAddresses"> = {
    inputHost,
    inputRegistrableDomain,
    inputResolvesPrivate: inputPrivate,
  };

  // Collect, de-dup, and sort candidates
  const seen = new Set<string>();
  const candidates: string[] = [];

  for (const node of nodes) {
    if (!node.href) continue;
    const resolved = resolveUrl(node.href, inputUrl);
    if (!resolved) continue;
    // Drop non-http(s) silently — not probe targets
    try {
      const u = new URL(resolved);
      if (u.protocol !== "http:" && u.protocol !== "https:") continue;
    } catch {
      continue;
    }
    const redacted = redactUrl(resolved, redactParams);
    if (!seen.has(redacted)) {
      seen.add(redacted);
      candidates.push(redacted);
    }
  }

  // Sort ascending (byte order)
  candidates.sort();

  // Partition into external-scope (no fetch), fetch-eligible (up to cap), and cap-exceeded
  const external: LinkProbe[] = [];
  const eligible: string[] = [];
  const capExceeded: string[] = [];

  for (const cand of candidates) {
    let targetUrl: URL;
    try {
      targetUrl = new URL(cand);
    } catch {
      // Malformed after redaction — skip silently
      continue;
    }
    const scopeDecision = decideEgress(targetUrl, {
      ...baseCtx,
      targetAddresses: null, // scope-only check, no DNS yet
    });
    if (scopeDecision === "external-scope") {
      external.push({
        url: cand,
        redirectChain: [],
        finalUrl: null,
        status: null,
        skipped: "external-scope",
        error: null,
      });
    } else if (scopeDecision === "scheme") {
      // Already filtered above, but be defensive
      continue;
    } else {
      if (eligible.length < MAX_LINK_PROBES) {
        eligible.push(cand);
      } else {
        capExceeded.push(cand);
      }
    }
  }

  const capExceededProbes: LinkProbe[] = capExceeded.map((url) => ({
    url,
    redirectChain: [],
    finalUrl: null,
    status: null,
    skipped: "cap-exceeded" as const,
    error: null,
  }));

  // Worker pool over eligible candidates
  const results: LinkProbe[] = await runWorkerPool(
    eligible,
    PROBE_CONCURRENCY,
    (url) => probeOne(url, parsedInput, baseCtx, redactParams)
  );

  // Combine and re-sort by url for deterministic output
  const all = [...external, ...results, ...capExceededProbes];
  all.sort((a, b) => (a.url < b.url ? -1 : a.url > b.url ? 1 : 0));
  return all;
}

/**
 * Simple fixed-concurrency worker pool.
 * Processes `items` with at most `concurrency` in-flight at once.
 * Results array order matches input order (deterministic reassembly).
 */
async function runWorkerPool<T>(
  items: string[],
  concurrency: number,
  worker: (item: string) => Promise<T>
): Promise<T[]> {
  const results: T[] = new Array(items.length);
  let nextIndex = 0;

  async function runWorker(): Promise<void> {
    while (nextIndex < items.length) {
      const i = nextIndex++;
      const item = items[i];
      if (item === undefined) break;
      results[i] = await worker(item);
    }
  }

  const workers: Promise<void>[] = [];
  for (let i = 0; i < Math.min(concurrency, items.length); i++) {
    workers.push(runWorker());
  }
  await Promise.all(workers);

  return results;
}

/**
 * Probe a single URL: DNS check, then fetch with redirect following.
 * Never throws — errors are recorded in the returned LinkProbe.
 */
async function probeOne(
  urlStr: string,
  parsedInput: URL,
  baseCtx: Omit<EgressContext, "targetAddresses">,
  redactParams: string[]
): Promise<LinkProbe> {
  const probe: LinkProbe = {
    url: urlStr,
    redirectChain: [],
    finalUrl: null,
    status: null,
    skipped: null,
    error: null,
  };

  try {
    await followRedirects(urlStr, parsedInput, baseCtx, redactParams, probe);
  } catch (err) {
    probe.error = err instanceof Error ? err.message : String(err);
  }

  return probe;
}

/**
 * Follow redirects from `startUrl`, populating `probe` in place.
 * Applies the full egress gate (DNS + scope + private-address) at each hop.
 */
async function followRedirects(
  startUrl: string,
  parsedInput: URL,
  baseCtx: Omit<EgressContext, "targetAddresses">,
  redactParams: string[],
  probe: LinkProbe
): Promise<void> {
  let currentUrl = startUrl;

  for (let hop = 0; hop < MAX_REDIRECT_HOPS; hop++) {
    let targetUrl: URL;
    try {
      targetUrl = new URL(currentUrl);
    } catch {
      probe.error = `Invalid URL: ${currentUrl}`;
      return;
    }

    // DNS lookup to check for private addresses
    const host = targetUrl.hostname;
    let resolvedAddrs: string[] | null = null;

    // Only look up if not an IP literal (avoid pointless lookup)
    if (!isIpLiteral(host)) {
      try {
        const lookupResult = await dns.promises.lookup(host, { all: true });
        resolvedAddrs = lookupResult.map((r) => r.address);
      } catch (err) {
        probe.error = `DNS lookup failed for ${host}: ${err instanceof Error ? err.message : String(err)}`;
        return;
      }
    } else {
      resolvedAddrs = [host];
    }

    // Full egress gate with addresses
    const decision = decideEgress(targetUrl, {
      ...baseCtx,
      targetAddresses: resolvedAddrs,
    });

    if (decision !== null) {
      if (hop === 0) {
        // First hop — set skipped (not redirect-blocked)
        probe.skipped = decision === "private-address" ? "private-address" : "scheme";
      } else {
        // Mid-redirect violation
        probe.skipped = "redirect-blocked";
      }
      return;
    }

    // Fetch with redirect:"manual"
    let response: Response;
    try {
      response = await fetch(currentUrl, {
        redirect: "manual",
        signal: AbortSignal.timeout(PROBE_TIMEOUT_MS),
      });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      probe.error = msg.includes("timeout") || msg.includes("Timeout")
        ? `timeout fetching ${redactUrl(currentUrl, redactParams)}`
        : msg;
      return;
    } finally {
      // We never want the body — cancel it in a best-effort way
    }

    // Cancel body to free resources (best-effort)
    try {
      await response.body?.cancel();
    } catch {
      // ignore
    }

    const status = response.status;

    if (status >= 300 && status < 400) {
      const location = response.headers.get("Location");
      if (!location) {
        // 3xx with no Location — treat as final
        probe.status = status;
        probe.finalUrl = redactUrl(currentUrl, redactParams);
        return;
      }

      // Push pre-redirect URL onto chain (semantics: one entry per hop, pre-redirect URLs)
      probe.redirectChain.push(redactUrl(currentUrl, redactParams));

      // Resolve the target relative to current
      let nextUrl: string;
      try {
        nextUrl = new URL(location, currentUrl).toString();
      } catch {
        probe.error = `Invalid Location header: ${location}`;
        return;
      }

      currentUrl = nextUrl;
      continue;
    }

    // Non-redirect: this is the final response
    probe.status = status;
    probe.finalUrl = redactUrl(currentUrl, redactParams);
    return;
  }

  // Exhausted MAX_REDIRECT_HOPS
  probe.error = "too many redirects";
}

/**
 * Returns true if `host` appears to be an IP literal (IPv4 dotted-decimal or
 * IPv6 in brackets already stripped by URL parsing).
 */
function isIpLiteral(host: string): boolean {
  // IPv4: four dotted groups
  if (/^\d+\.\d+\.\d+\.\d+$/.test(host)) return true;
  // IPv6: contains colons (URL hostname strips brackets)
  if (host.includes(":")) return true;
  return false;
}
