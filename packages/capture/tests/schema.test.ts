import { describe, it, expect } from "vitest";
import { CaptureBundleSchema } from "../src/schema.js";

/**
 * A minimal known-good CaptureBundle sample for testing schema validation.
 */
const KNOWN_GOOD_BUNDLE = {
  schemaVersion: "1.0" as const,
  capturedAt: "2026-01-01T00:00:00.000Z",
  viewport: {
    name: "desktop",
    width: 1440,
    height: 1000,
    dsf: 1,
  },
  environment: {
    os: "linux",
    chromiumBuild: "Chromium/130.0.6723.116",
    playwright: "1.60.0",
    dsf: 1,
  },
  determinism: {
    animationsDisabled: "ran" as const,
    reducedMotion: "ran" as const,
    timeFrozen: "ran" as const,
    randomStubbed: "ran" as const,
    fontsReady: "ran" as const,
    imagesDecoded: "ran" as const,
    lazyLoadPass: "ran" as const,
    settled: "ran" as const,
    clicked: [],
    hidden: [],
    masked: [],
    retriedWithoutTimeFreeze: false,
  },
  page: {
    url: "http://localhost:3000/",
    finalUrl: "http://localhost:3000/",
    redirectChain: [],
    statusCode: 200,
    title: "Test Page",
    metaDescription: "A test page",
    canonical: null,
    lang: "en",
    pageHeight: 1000,
    nodes: [
      {
        id: "node_0",
        kind: "heading" as const,
        role: "heading",
        text: "Welcome",
        accName: "Welcome",
        href: null,
        imageAlt: null,
        bbox: [0, 0, 200, 40] as [number, number, number, number],
        seqIndex: 0,
        anchors: {
          text: "Welcome",
          role: "heading",
          href: null,
          alt: null,
          ariaLabel: null,
          nearestHeading: null,
          landmark: "main",
          ordinalInLandmark: 1,
        },
        cssSelector: "h1:nth-of-type(1)",
        rawHref: null,
        src: null,
        naturalWidth: null,
        naturalHeight: null,
        loaded: null,
        headingLevel: 1,
      },
    ],
    landmarks: ["main"],
    network: {
      requests: [
        {
          url: "http://localhost:3000/",
          status: 200,
          type: "document",
          failed: false,
        },
      ],
    },
    console: [],
    a11y: {
      violations: [],
    },
    linkProbes: [],
  },
  computedStyles: {},
  screenshots: {
    fullPage: "desktop/old.png",
    viewport: "desktop/old-vp.png",
  },
};

describe("CaptureBundleSchema", () => {
  it("accepts a known-good sample bundle", () => {
    const result = CaptureBundleSchema.safeParse(KNOWN_GOOD_BUNDLE);
    expect(result.success).toBe(true);
  });

  it("rejects a bundle missing environment fingerprint", () => {
    const { environment: _env, ...bundleWithoutEnv } = KNOWN_GOOD_BUNDLE;
    const result = CaptureBundleSchema.safeParse(bundleWithoutEnv);
    expect(result.success).toBe(false);
    if (!result.success) {
      // Should mention environment
      const issues = result.error.issues;
      const hasEnvironmentError = issues.some(
        (i) =>
          i.path.includes("environment") ||
          i.message.toLowerCase().includes("required")
      );
      expect(hasEnvironmentError).toBe(true);
    }
  });

  it("rejects a bundle missing screenshots", () => {
    const { screenshots: _ss, ...bundleWithoutScreenshots } = KNOWN_GOOD_BUNDLE;
    const result = CaptureBundleSchema.safeParse(bundleWithoutScreenshots);
    expect(result.success).toBe(false);
  });

  it("rejects a bundle with wrong schemaVersion", () => {
    const bundle = { ...KNOWN_GOOD_BUNDLE, schemaVersion: "2.0" };
    const result = CaptureBundleSchema.safeParse(bundle);
    expect(result.success).toBe(false);
  });

  it("accepts all determinism step values", () => {
    const bundle = {
      ...KNOWN_GOOD_BUNDLE,
      determinism: {
        ...KNOWN_GOOD_BUNDLE.determinism,
        animationsDisabled: "failed" as const,
        timeFrozen: "skipped" as const,
        lazyLoadPass: "failed" as const,
      },
    };
    const result = CaptureBundleSchema.safeParse(bundle);
    expect(result.success).toBe(true);
  });

  it("rejects invalid determinism step value", () => {
    const bundle = {
      ...KNOWN_GOOD_BUNDLE,
      determinism: {
        ...KNOWN_GOOD_BUNDLE.determinism,
        animationsDisabled: "unknown_value",
      },
    };
    const result = CaptureBundleSchema.safeParse(bundle);
    expect(result.success).toBe(false);
  });

  it("accepts nodes with nullable fields", () => {
    const bundle = {
      ...KNOWN_GOOD_BUNDLE,
      page: {
        ...KNOWN_GOOD_BUNDLE.page,
        nodes: [
          {
            id: "node_0",
            kind: "image" as const,
            role: null,
            text: null,
            accName: null,
            href: null,
            imageAlt: "A logo",
            bbox: [0, 0, 100, 50] as [number, number, number, number],
            seqIndex: 0,
            anchors: {
              text: null,
              role: null,
              href: null,
              alt: "A logo",
              ariaLabel: null,
              nearestHeading: null,
              landmark: null,
              ordinalInLandmark: null,
            },
            cssSelector: null,
            rawHref: null,
            src: "http://localhost:3000/logo.png",
            naturalWidth: 200,
            naturalHeight: 50,
            loaded: true,
            headingLevel: null,
          },
        ],
      },
    };
    const result = CaptureBundleSchema.safeParse(bundle);
    expect(result.success).toBe(true);
  });

  it("accepts computedStyles as an empty object (M1)", () => {
    const bundle = { ...KNOWN_GOOD_BUNDLE, computedStyles: {} };
    const result = CaptureBundleSchema.safeParse(bundle);
    expect(result.success).toBe(true);
  });

  it("accepts computedStyles with node entries (M4 shape)", () => {
    const bundle = {
      ...KNOWN_GOOD_BUNDLE,
      computedStyles: {
        node_0: {
          "font-family": "Arial, sans-serif",
          "color": "rgb(0, 0, 0)",
        },
      },
    };
    const result = CaptureBundleSchema.safeParse(bundle);
    expect(result.success).toBe(true);
  });

  // ── M3 SemanticNode field tests ────────────────────────────────────────────

  it("M3: accepts image node with src, dims, and loaded fields", () => {
    const bundle = {
      ...KNOWN_GOOD_BUNDLE,
      page: {
        ...KNOWN_GOOD_BUNDLE.page,
        nodes: [
          {
            id: "node_0",
            kind: "image" as const,
            role: "img",
            text: null,
            accName: "Performance chart",
            href: null,
            imageAlt: "Performance chart",
            bbox: [0, 100, 600, 400] as [number, number, number, number],
            seqIndex: 0,
            anchors: {
              text: null,
              role: "img",
              href: null,
              alt: "Performance chart",
              ariaLabel: null,
              nearestHeading: "Analytics",
              landmark: "main",
              ordinalInLandmark: 1,
            },
            cssSelector: "img:nth-of-type(1)",
            rawHref: null,
            src: "http://localhost:3001/assets/images/chart.png",
            naturalWidth: 600,
            naturalHeight: 400,
            loaded: true,
            headingLevel: null,
          },
        ],
      },
    };
    const result = CaptureBundleSchema.safeParse(bundle);
    expect(result.success).toBe(true);
  });

  it("M3: accepts link node with rawHref field", () => {
    const bundle = {
      ...KNOWN_GOOD_BUNDLE,
      page: {
        ...KNOWN_GOOD_BUNDLE.page,
        nodes: [
          {
            id: "node_0",
            kind: "link" as const,
            role: "link",
            text: "Get a Demo",
            accName: "Get a Demo",
            href: "http://localhost:3001/demo.html",
            imageAlt: null,
            bbox: [0, 50, 120, 40] as [number, number, number, number],
            seqIndex: 0,
            anchors: {
              text: "Get a Demo",
              role: "link",
              href: "http://localhost:3001/demo.html",
              alt: null,
              ariaLabel: null,
              nearestHeading: "Welcome",
              landmark: "main",
              ordinalInLandmark: 1,
            },
            cssSelector: "a:nth-of-type(1)",
            rawHref: "demo.html",
            src: null,
            naturalWidth: null,
            naturalHeight: null,
            loaded: null,
            headingLevel: null,
          },
        ],
      },
    };
    const result = CaptureBundleSchema.safeParse(bundle);
    expect(result.success).toBe(true);
  });

  it("M3: accepts heading node with headingLevel field", () => {
    const bundle = {
      ...KNOWN_GOOD_BUNDLE,
      page: {
        ...KNOWN_GOOD_BUNDLE.page,
        nodes: [
          {
            id: "node_0",
            kind: "heading" as const,
            role: "heading",
            text: "Main Title",
            accName: "Main Title",
            href: null,
            imageAlt: null,
            bbox: [0, 0, 800, 60] as [number, number, number, number],
            seqIndex: 0,
            anchors: {
              text: "Main Title",
              role: "heading",
              href: null,
              alt: null,
              ariaLabel: null,
              nearestHeading: null,
              landmark: "main",
              ordinalInLandmark: 1,
            },
            cssSelector: "h1:nth-of-type(1)",
            rawHref: null,
            src: null,
            naturalWidth: null,
            naturalHeight: null,
            loaded: null,
            headingLevel: 1,
          },
        ],
      },
    };
    const result = CaptureBundleSchema.safeParse(bundle);
    expect(result.success).toBe(true);
  });

  it("M3: accepts text node with all six new fields null", () => {
    const bundle = {
      ...KNOWN_GOOD_BUNDLE,
      page: {
        ...KNOWN_GOOD_BUNDLE.page,
        nodes: [
          {
            id: "node_0",
            kind: "text" as const,
            role: null,
            text: "Some paragraph text",
            accName: "Some paragraph text",
            href: null,
            imageAlt: null,
            bbox: [0, 200, 600, 80] as [number, number, number, number],
            seqIndex: 0,
            anchors: {
              text: "Some paragraph text",
              role: null,
              href: null,
              alt: null,
              ariaLabel: null,
              nearestHeading: "Main Title",
              landmark: "main",
              ordinalInLandmark: 1,
            },
            cssSelector: "p:nth-of-type(1)",
            rawHref: null,
            src: null,
            naturalWidth: null,
            naturalHeight: null,
            loaded: null,
            headingLevel: null,
          },
        ],
      },
    };
    const result = CaptureBundleSchema.safeParse(bundle);
    expect(result.success).toBe(true);
  });

  it("M3: rejects a node missing the new M3 fields", () => {
    const bundle = {
      ...KNOWN_GOOD_BUNDLE,
      page: {
        ...KNOWN_GOOD_BUNDLE.page,
        nodes: [
          {
            id: "node_0",
            kind: "text" as const,
            role: null,
            text: "Missing fields node",
            accName: "Missing fields node",
            href: null,
            imageAlt: null,
            bbox: [0, 0, 100, 20] as [number, number, number, number],
            seqIndex: 0,
            anchors: {
              text: "Missing fields node",
              role: null,
              href: null,
              alt: null,
              ariaLabel: null,
              nearestHeading: null,
              landmark: null,
              ordinalInLandmark: null,
            },
            cssSelector: null,
            // rawHref, src, naturalWidth, naturalHeight, loaded, headingLevel intentionally omitted
          },
        ],
      },
    };
    const result = CaptureBundleSchema.safeParse(bundle);
    expect(result.success).toBe(false);
  });

  it("M3: rejects headingLevel out of range (0)", () => {
    const bundle = {
      ...KNOWN_GOOD_BUNDLE,
      page: {
        ...KNOWN_GOOD_BUNDLE.page,
        nodes: [
          {
            ...KNOWN_GOOD_BUNDLE.page.nodes[0],
            headingLevel: 0,
          },
        ],
      },
    };
    const result = CaptureBundleSchema.safeParse(bundle);
    expect(result.success).toBe(false);
  });

  it("M3: rejects headingLevel out of range (7)", () => {
    const bundle = {
      ...KNOWN_GOOD_BUNDLE,
      page: {
        ...KNOWN_GOOD_BUNDLE.page,
        nodes: [
          {
            ...KNOWN_GOOD_BUNDLE.page.nodes[0],
            headingLevel: 7,
          },
        ],
      },
    };
    const result = CaptureBundleSchema.safeParse(bundle);
    expect(result.success).toBe(false);
  });

  it("M3: rejects negative naturalWidth", () => {
    const bundle = {
      ...KNOWN_GOOD_BUNDLE,
      page: {
        ...KNOWN_GOOD_BUNDLE.page,
        nodes: [
          {
            ...KNOWN_GOOD_BUNDLE.page.nodes[0],
            naturalWidth: -1,
          },
        ],
      },
    };
    const result = CaptureBundleSchema.safeParse(bundle);
    expect(result.success).toBe(false);
  });
});
