/**
 * Unit tests for probe.ts:
 *  - decideEgress: pure egress gate
 *  - probeLinks: with mocked fetch and DNS
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// Top-level mock for the dns module — hoisted by vitest
vi.mock("dns", () => ({
  promises: {
    lookup: vi.fn(),
  },
}));

import { decideEgress, probeLinks, MAX_LINK_PROBES } from "../src/probe.js";
import type { EgressContext } from "../src/probe.js";
import type { SemanticNode } from "../src/schema.js";
import * as dns from "dns";

// ─── decideEgress ─────────────────────────────────────────────────────────────

describe("decideEgress — scheme check", () => {
  const ctx: EgressContext = {
    inputHost: "example.com",
    inputRegistrableDomain: "example.com",
    inputResolvesPrivate: false,
    targetAddresses: null,
  };

  it("allows http:", () => {
    expect(decideEgress(new URL("http://example.com/path"), ctx)).toBe(null);
  });

  it("allows https:", () => {
    expect(decideEgress(new URL("https://example.com/path"), ctx)).toBe(null);
  });

  it("refuses file:", () => {
    expect(decideEgress(new URL("file:///etc/passwd"), ctx)).toBe("scheme");
  });

  it("refuses ftp:", () => {
    expect(decideEgress(new URL("ftp://example.com/file.txt"), ctx)).toBe("scheme");
  });

  it("refuses data: (non-http/https scheme)", () => {
    expect(decideEgress(new URL("data:text/html,hello"), ctx)).toBe("scheme");
  });
});

describe("decideEgress — scope check (registrable domain)", () => {
  const ctx: EgressContext = {
    inputHost: "www.example.com",
    inputRegistrableDomain: "example.com",
    inputResolvesPrivate: false,
    targetAddresses: null,
  };

  it("allows same registrable domain (apex)", () => {
    expect(decideEgress(new URL("https://example.com/"), ctx)).toBe(null);
  });

  it("allows subdomain of same registrable domain", () => {
    expect(decideEgress(new URL("https://blog.example.com/post"), ctx)).toBe(null);
  });

  it("allows same host (www)", () => {
    expect(decideEgress(new URL("https://www.example.com/about"), ctx)).toBe(null);
  });

  it("refuses different registrable domain", () => {
    expect(decideEgress(new URL("https://other.com/path"), ctx)).toBe("external-scope");
  });

  it("refuses different TLD same SLD", () => {
    expect(decideEgress(new URL("https://example.org/path"), ctx)).toBe("external-scope");
  });
});

describe("decideEgress — hostname fallback (localhost / IP literals)", () => {
  it("localhost input, localhost target — passes (exact hostname match)", () => {
    const ctx: EgressContext = {
      inputHost: "localhost",
      inputRegistrableDomain: null, // localhost has no registrable domain
      inputResolvesPrivate: true,
      targetAddresses: null,
    };
    expect(decideEgress(new URL("http://localhost:3000/page"), ctx)).toBe(null);
  });

  it("localhost input, 127.0.0.1 target — refused (hostnames differ)", () => {
    const ctx: EgressContext = {
      inputHost: "localhost",
      inputRegistrableDomain: null,
      inputResolvesPrivate: true,
      targetAddresses: null,
    };
    expect(decideEgress(new URL("http://127.0.0.1:3000/page"), ctx)).toBe("external-scope");
  });

  it("127.0.0.1 input, 127.0.0.1 target — passes (exact hostname match)", () => {
    const ctx: EgressContext = {
      inputHost: "127.0.0.1",
      inputRegistrableDomain: null,
      inputResolvesPrivate: true,
      targetAddresses: null,
    };
    expect(decideEgress(new URL("http://127.0.0.1/path"), ctx)).toBe(null);
  });

  it("IP input, different IP target — refused", () => {
    const ctx: EgressContext = {
      inputHost: "192.168.1.1",
      inputRegistrableDomain: null,
      inputResolvesPrivate: true,
      targetAddresses: null,
    };
    expect(decideEgress(new URL("http://192.168.1.2/path"), ctx)).toBe("external-scope");
  });
});

describe("decideEgress — private-address check", () => {
  const baseCtx: Omit<EgressContext, "targetAddresses"> = {
    inputHost: "example.com",
    inputRegistrableDomain: "example.com",
    inputResolvesPrivate: false,
  };

  it("refuses 10.x.x.x", () => {
    const ctx: EgressContext = { ...baseCtx, targetAddresses: ["10.0.0.1"] };
    expect(decideEgress(new URL("https://example.com/api"), ctx)).toBe("private-address");
  });

  it("refuses 172.16.x.x", () => {
    const ctx: EgressContext = { ...baseCtx, targetAddresses: ["172.16.0.1"] };
    expect(decideEgress(new URL("https://example.com/api"), ctx)).toBe("private-address");
  });

  it("refuses 172.31.x.x (still in 172.16/12)", () => {
    const ctx: EgressContext = { ...baseCtx, targetAddresses: ["172.31.255.254"] };
    expect(decideEgress(new URL("https://example.com/api"), ctx)).toBe("private-address");
  });

  it("allows 172.15.x.x (outside 172.16/12)", () => {
    const ctx: EgressContext = { ...baseCtx, targetAddresses: ["172.15.0.1"] };
    expect(decideEgress(new URL("https://example.com/api"), ctx)).toBe(null);
  });

  it("refuses 192.168.x.x", () => {
    const ctx: EgressContext = { ...baseCtx, targetAddresses: ["192.168.0.1"] };
    expect(decideEgress(new URL("https://example.com/api"), ctx)).toBe("private-address");
  });

  it("refuses 169.254.169.254 (cloud metadata)", () => {
    const ctx: EgressContext = { ...baseCtx, targetAddresses: ["169.254.169.254"] };
    expect(decideEgress(new URL("https://example.com/api"), ctx)).toBe("private-address");
  });

  it("refuses 127.0.0.1 (loopback)", () => {
    const ctx: EgressContext = { ...baseCtx, targetAddresses: ["127.0.0.1"] };
    expect(decideEgress(new URL("https://example.com/api"), ctx)).toBe("private-address");
  });

  it("refuses ::1 (IPv6 loopback)", () => {
    const ctx: EgressContext = { ...baseCtx, targetAddresses: ["::1"] };
    expect(decideEgress(new URL("https://example.com/api"), ctx)).toBe("private-address");
  });

  it("refuses fd00::1 (ULA)", () => {
    const ctx: EgressContext = { ...baseCtx, targetAddresses: ["fd00::1"] };
    expect(decideEgress(new URL("https://example.com/api"), ctx)).toBe("private-address");
  });

  it("refuses ::ffff:10.0.0.1 (IPv4-mapped private)", () => {
    const ctx: EgressContext = { ...baseCtx, targetAddresses: ["::ffff:10.0.0.1"] };
    expect(decideEgress(new URL("https://example.com/api"), ctx)).toBe("private-address");
  });

  it("allows a public address", () => {
    const ctx: EgressContext = { ...baseCtx, targetAddresses: ["8.8.8.8"] };
    expect(decideEgress(new URL("https://example.com/api"), ctx)).toBe(null);
  });

  it("allows private address when inputResolvesPrivate is true", () => {
    const ctx: EgressContext = {
      ...baseCtx,
      inputResolvesPrivate: true,
      targetAddresses: ["10.0.0.1"],
    };
    expect(decideEgress(new URL("https://example.com/api"), ctx)).toBe(null);
  });

  it("allows private address when targetAddresses is null (no DNS info)", () => {
    const ctx: EgressContext = { ...baseCtx, targetAddresses: null };
    // Without DNS info we cannot make a private-address decision
    expect(decideEgress(new URL("https://example.com/api"), ctx)).toBe(null);
  });
});

// ─── probeLinks helpers ───────────────────────────────────────────────────────

/**
 * Build a minimal SemanticNode with just enough fields for probeLinks.
 */
function makeNode(href: string | null, seqIndex = 0): SemanticNode {
  return {
    id: `node_${seqIndex}`,
    kind: "link",
    role: "link",
    text: "Link",
    accName: "Link",
    href,
    imageAlt: null,
    bbox: [0, 0, 100, 20],
    seqIndex,
    anchors: {
      text: "Link",
      role: "link",
      href,
      alt: null,
      ariaLabel: null,
      nearestHeading: null,
      landmark: "main",
      ordinalInLandmark: 1,
    },
    cssSelector: "a",
  };
}

/**
 * Build a Response-like object for use in fetch mocks.
 */
function makeResponse(
  status: number,
  headers: Record<string, string> = {}
): Response {
  return {
    status,
    headers: {
      get: (name: string) => headers[name.toLowerCase()] ?? null,
    },
    body: null,
  } as unknown as Response;
}

// DNS lookup mock accessor
const dnsLookupMock = dns.promises.lookup as ReturnType<typeof vi.fn>;

// ─── probeLinks tests ─────────────────────────────────────────────────────────

describe("probeLinks — fragment stripping and dedup", () => {
  beforeEach(() => {
    dnsLookupMock.mockResolvedValue([{ address: "93.184.216.34", family: 4 }]);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(makeResponse(200)));
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    dnsLookupMock.mockReset();
  });

  it("strips fragment from hrefs", async () => {
    const nodes = [makeNode("http://localhost:3000/page#section1")];
    const results = await probeLinks(nodes, "http://localhost:3000/", []);
    expect(results).toHaveLength(1);
    expect(results[0]!.url).toBe("http://localhost:3000/page");
  });

  it("deduplicates same href with different fragments", async () => {
    const nodes = [
      makeNode("http://localhost:3000/page#a", 0),
      makeNode("http://localhost:3000/page#b", 1),
      makeNode("http://localhost:3000/page", 2),
    ];
    const results = await probeLinks(nodes, "http://localhost:3000/", []);
    expect(results).toHaveLength(1);
    expect(results[0]!.url).toBe("http://localhost:3000/page");
  });

  it("sorts output by url byte order", async () => {
    const nodes = [
      makeNode("http://localhost:3000/z", 0),
      makeNode("http://localhost:3000/a", 1),
      makeNode("http://localhost:3000/m", 2),
    ];
    const results = await probeLinks(nodes, "http://localhost:3000/", []);
    const urls = results.map((r) => r.url);
    expect(urls).toEqual([...urls].sort());
  });
});

describe("probeLinks — external links produce records without fetch", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    dnsLookupMock.mockReset();
  });

  it("marks external-scope links without calling fetch, fetches internal links", async () => {
    // DNS returns public address for internal link
    dnsLookupMock.mockResolvedValue([{ address: "93.184.216.34", family: 4 }]);
    const fetchMock = vi.fn().mockResolvedValue(makeResponse(200));
    vi.stubGlobal("fetch", fetchMock);

    const nodes = [
      makeNode("https://other-domain.com/page", 0),
      makeNode("https://example.com/internal", 1),
    ];

    const results = await probeLinks(nodes, "https://example.com/", []);
    const external = results.find((r) => r.url.includes("other-domain"));
    const internal = results.find((r) => r.url.includes("example.com/internal"));

    expect(external?.skipped).toBe("external-scope");
    expect(external?.finalUrl).toBeNull();
    // fetch must have been called for the internal link
    expect(fetchMock).toHaveBeenCalledWith(
      expect.stringContaining("example.com/internal"),
      expect.anything()
    );
    // fetch must NOT have been called for the external link
    expect(fetchMock).not.toHaveBeenCalledWith(
      expect.stringContaining("other-domain.com"),
      expect.anything()
    );
    expect(internal?.skipped).toBeNull();
  });
});

describe("probeLinks — cap-exceeded after 50", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    dnsLookupMock.mockReset();
  });

  it(`records cap-exceeded for links beyond ${MAX_LINK_PROBES}`, async () => {
    dnsLookupMock.mockResolvedValue([{ address: "127.0.0.1", family: 4 }]);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(makeResponse(200)));

    // Create MAX_LINK_PROBES + 5 unique links
    const nodes = Array.from({ length: MAX_LINK_PROBES + 5 }, (_, i) =>
      makeNode(`http://localhost:3000/page-${String(i).padStart(3, "0")}`, i)
    );

    const results = await probeLinks(nodes, "http://localhost:3000/", []);
    const capExceeded = results.filter((r) => r.skipped === "cap-exceeded");
    const probedOrErrored = results.filter((r) => r.skipped === null || r.skipped === undefined);

    expect(capExceeded).toHaveLength(5);
    expect(probedOrErrored.length).toBeLessThanOrEqual(MAX_LINK_PROBES);
  });
});

describe("probeLinks — 2-hop redirect chain ordering", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    dnsLookupMock.mockReset();
  });

  it("builds a 2-hop redirect chain with correct ordering", async () => {
    // All DNS lookups return loopback (private), but inputResolvesPrivate = true
    // because localhost is the input host → private targets are allowed
    dnsLookupMock.mockResolvedValue([{ address: "127.0.0.1", family: 4 }]);

    // Simulate: /start → 301 /mid → 301 /end → 200
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(
        makeResponse(301, { location: "http://localhost:3017/mid" })
      )
      .mockResolvedValueOnce(
        makeResponse(301, { location: "http://localhost:3017/end" })
      )
      .mockResolvedValueOnce(makeResponse(200));
    vi.stubGlobal("fetch", fetchMock);

    const nodes = [makeNode("http://localhost:3017/start", 0)];
    const results = await probeLinks(nodes, "http://localhost:3017/", []);

    expect(results).toHaveLength(1);
    const probe = results[0]!;
    expect(probe.redirectChain).toEqual([
      "http://localhost:3017/start",
      "http://localhost:3017/mid",
    ]);
    expect(probe.finalUrl).toBe("http://localhost:3017/end");
    expect(probe.status).toBe(200);
    expect(probe.skipped).toBeNull();
    expect(probe.error).toBeNull();
  });
});

describe("probeLinks — redirect-blocked when hop target violates egress", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    dnsLookupMock.mockReset();
  });

  it("stops with redirect-blocked when redirect goes to an external domain", async () => {
    // First call: input host DNS check → localhost private → input resolves private
    // Second call: localhost target for /start → allowed (input private)
    // After redirect to evil.example.net: DNS lookup for that host → public IP
    dnsLookupMock
      .mockResolvedValueOnce([{ address: "127.0.0.1", family: 4 }]) // input host check
      .mockResolvedValueOnce([{ address: "127.0.0.1", family: 4 }]) // first fetch target
      .mockResolvedValueOnce([{ address: "1.2.3.4", family: 4 }]); // redirect target

    // /start → 301 to external domain
    const fetchMock = vi.fn().mockResolvedValueOnce(
      makeResponse(301, { location: "https://evil.example.net/steal" })
    );
    vi.stubGlobal("fetch", fetchMock);

    const nodes = [makeNode("http://localhost:3017/start", 0)];
    const results = await probeLinks(nodes, "http://localhost:3017/", []);

    expect(results).toHaveLength(1);
    const probe = results[0]!;
    expect(probe.skipped).toBe("redirect-blocked");
    // Partial chain recorded (pre-redirect URL)
    expect(probe.redirectChain).toEqual(["http://localhost:3017/start"]);
    expect(probe.finalUrl).toBeNull();
    expect(probe.status).toBeNull();
  });
});

describe("probeLinks — too many redirects (hop cap)", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    dnsLookupMock.mockReset();
  });

  it("records error 'too many redirects' after 5 hops", async () => {
    // All DNS lookups return loopback; input is localhost so private targets allowed
    dnsLookupMock.mockResolvedValue([{ address: "127.0.0.1", family: 4 }]);

    let hop = 0;
    const fetchMock = vi.fn().mockImplementation(() => {
      hop++;
      return Promise.resolve(
        makeResponse(301, { location: `http://localhost:3017/hop-${hop}` })
      );
    });
    vi.stubGlobal("fetch", fetchMock);

    const nodes = [makeNode("http://localhost:3017/start", 0)];
    const results = await probeLinks(nodes, "http://localhost:3017/", []);

    expect(results).toHaveLength(1);
    const probe = results[0]!;
    expect(probe.error).toBe("too many redirects");
  });
});

describe("probeLinks — non-http(s) schemes dropped silently", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    dnsLookupMock.mockReset();
  });

  it("does not produce a probe record for mailto: href", async () => {
    dnsLookupMock.mockResolvedValue([{ address: "127.0.0.1", family: 4 }]);
    vi.stubGlobal("fetch", vi.fn());
    const nodes = [makeNode("mailto:someone@example.com", 0)];
    const results = await probeLinks(nodes, "http://localhost:3000/", []);
    expect(results).toHaveLength(0);
  });

  it("does not produce a probe record for tel: href", async () => {
    dnsLookupMock.mockResolvedValue([{ address: "127.0.0.1", family: 4 }]);
    vi.stubGlobal("fetch", vi.fn());
    const nodes = [makeNode("tel:+15555555555", 0)];
    const results = await probeLinks(nodes, "http://localhost:3000/", []);
    expect(results).toHaveLength(0);
  });
});

describe("probeLinks — sorted output regardless of completion order", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    dnsLookupMock.mockReset();
  });

  it("output is sorted by url even with multiple candidates", async () => {
    // All DNS lookups return loopback; input is localhost so private targets allowed
    dnsLookupMock.mockResolvedValue([{ address: "127.0.0.1", family: 4 }]);

    let callCount = 0;
    const fetchMock = vi.fn().mockImplementation(() => {
      callCount++;
      return Promise.resolve(makeResponse(200));
    });
    vi.stubGlobal("fetch", fetchMock);

    const nodes = [
      makeNode("http://localhost:3000/z", 0),
      makeNode("http://localhost:3000/a", 1),
    ];
    const results = await probeLinks(nodes, "http://localhost:3000/", []);
    const urls = results.map((r) => r.url);
    expect(urls).toEqual([...urls].sort());
    expect(callCount).toBe(2);
  });
});
