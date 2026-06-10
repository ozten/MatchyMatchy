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
});
