import { z } from "zod";

// ─── Determinism step status ───────────────────────────────────────────────
export const DeterminismStepSchema = z.enum(["ran", "failed", "skipped"]);
export type DeterminismStep = z.infer<typeof DeterminismStepSchema>;

// ─── Viewport ─────────────────────────────────────────────────────────────
export const ViewportConfigSchema = z.object({
  name: z.string(),
  width: z.number().int().positive(),
  height: z.number().int().positive(),
  dsf: z.number().positive(),
});
export type ViewportConfig = z.infer<typeof ViewportConfigSchema>;

// ─── Stabilization config ─────────────────────────────────────────────────
export const StabilizationConfigSchema = z.object({
  freezeTime: z.boolean().default(true),
  fixedTime: z.string().default("2026-01-01T00:00:00.000Z"),
  stubRandom: z.boolean().default(true),
  randomSeed: z.number().int().default(1337),
  networkIdleTimeoutMs: z.number().int().positive().default(15000),
  settleMs: z.number().int().nonnegative().default(1000),
  lazyScrollStepPx: z.number().int().positive().default(800),
});
export type StabilizationConfig = z.infer<typeof StabilizationConfigSchema>;

// ─── CaptureConfig ────────────────────────────────────────────────────────
export const CaptureConfigSchema = z.object({
  mode: z.enum(["capture", "doctor"]),
  url: z.string().optional(),
  outDir: z.string().optional(),
  prefix: z.enum(["old", "new"]).optional(),
  viewport: ViewportConfigSchema.default({
    name: "desktop",
    width: 1440,
    height: 1000,
    dsf: 1,
  }),
  stabilization: StabilizationConfigSchema.default({}),
  hideSelectors: z.array(z.string()).default([]),
  maskSelectors: z.array(z.string()).default([]),
  clickBeforeCapture: z.array(z.string()).default([]),
  maxTextLength: z.number().int().positive().default(500),
  redactParams: z
    .array(z.string())
    .default([
      "token",
      "sig",
      "signature",
      "key",
      "auth",
      "apikey",
      "access_token",
    ]),
  probeLinks: z.boolean().default(false),
});
export type CaptureConfig = z.infer<typeof CaptureConfigSchema>;

// ─── LinkProbe ────────────────────────────────────────────────────────────
export const LinkProbeSkippedReasonSchema = z.enum([
  "scheme",
  "external-scope",
  "private-address",
  "cap-exceeded",
  "redirect-blocked",
]);
export type LinkProbeSkippedReason = z.infer<typeof LinkProbeSkippedReasonSchema>;

export const LinkProbeSchema = z.object({
  url: z.string(),
  redirectChain: z.array(z.string()),
  finalUrl: z.string().nullable(),
  status: z.number().int().nullable(),
  skipped: LinkProbeSkippedReasonSchema.nullable(),
  error: z.string().nullable(),
});
export type LinkProbe = z.infer<typeof LinkProbeSchema>;

// ─── SemanticNode anchors ─────────────────────────────────────────────────
export const AnchorsSchema = z.object({
  text: z.string().nullable(),
  role: z.string().nullable(),
  href: z.string().nullable(),
  alt: z.string().nullable(),
  ariaLabel: z.string().nullable(),
  nearestHeading: z.string().nullable(),
  landmark: z.string().nullable(),
  ordinalInLandmark: z.number().int().nullable(),
});
export type Anchors = z.infer<typeof AnchorsSchema>;

// ─── SemanticNode ─────────────────────────────────────────────────────────
export const NodeKindSchema = z.enum([
  "heading",
  "text",
  "link",
  "button",
  "image",
  "form",
  "field",
  "landmark",
  "generic",
]);
export type NodeKind = z.infer<typeof NodeKindSchema>;

export const SemanticNodeSchema = z.object({
  id: z.string(),
  kind: NodeKindSchema,
  role: z.string().nullable(),
  text: z.string().nullable(),
  accName: z.string().nullable(),
  href: z.string().nullable(),
  imageAlt: z.string().nullable(),
  bbox: z.tuple([z.number(), z.number(), z.number(), z.number()]),
  seqIndex: z.number().int().nonnegative(),
  anchors: AnchorsSchema,
  cssSelector: z.string().nullable(),
  // M3 fields
  rawHref: z.string().nullable(),
  src: z.string().nullable(),
  naturalWidth: z.number().int().nonnegative().nullable(),
  naturalHeight: z.number().int().nonnegative().nullable(),
  loaded: z.boolean().nullable(),
  headingLevel: z.number().int().min(1).max(6).nullable(),
});
export type SemanticNode = z.infer<typeof SemanticNodeSchema>;

// ─── NetworkRequest ───────────────────────────────────────────────────────
export const NetworkRequestSchema = z.object({
  url: z.string(),
  status: z.number().int(),
  type: z.string(),
  failed: z.boolean(),
});
export type NetworkRequest = z.infer<typeof NetworkRequestSchema>;

// ─── ConsoleMessage ───────────────────────────────────────────────────────
export const ConsoleMessageSchema = z.object({
  level: z.string(),
  text: z.string(),
});
export type ConsoleMessage = z.infer<typeof ConsoleMessageSchema>;

// ─── DeterminismRecord ────────────────────────────────────────────────────
export const DeterminismRecordSchema = z.object({
  animationsDisabled: DeterminismStepSchema,
  reducedMotion: DeterminismStepSchema,
  timeFrozen: DeterminismStepSchema,
  randomStubbed: DeterminismStepSchema,
  fontsReady: DeterminismStepSchema,
  imagesDecoded: DeterminismStepSchema,
  lazyLoadPass: DeterminismStepSchema,
  settled: DeterminismStepSchema,
  clicked: z.array(z.string()),
  hidden: z.array(z.string()),
  masked: z.array(z.string()),
  retriedWithoutTimeFreeze: z.boolean(),
});
export type DeterminismRecord = z.infer<typeof DeterminismRecordSchema>;

// ─── PageModel ────────────────────────────────────────────────────────────
export const PageModelSchema = z.object({
  url: z.string(),
  finalUrl: z.string(),
  redirectChain: z.array(z.string()),
  statusCode: z.number().int(),
  title: z.string(),
  metaDescription: z.string(),
  canonical: z.string().nullable(),
  lang: z.string(),
  pageHeight: z.number().int(),
  nodes: z.array(SemanticNodeSchema),
  landmarks: z.array(z.string()),
  network: z.object({
    requests: z.array(NetworkRequestSchema),
  }),
  console: z.array(ConsoleMessageSchema),
  a11y: z.object({
    violations: z.array(z.unknown()),
  }),
  linkProbes: z.array(LinkProbeSchema),
});
export type PageModel = z.infer<typeof PageModelSchema>;

// ─── EnvironmentFingerprint ───────────────────────────────────────────────
export const EnvironmentFingerprintSchema = z.object({
  os: z.string(),
  chromiumBuild: z.string(),
  playwright: z.string(),
  dsf: z.number(),
});
export type EnvironmentFingerprint = z.infer<typeof EnvironmentFingerprintSchema>;

// ─── M4: AncestorDescriptor ───────────────────────────────────────────────
//
// Anchor set for ancestor elements: role/href/alt/ariaLabel are always null
// (ancestors are container elements, not semantic leaf nodes). text may be
// inherited from the single text-bearing semantic descendant when there is
// exactly one (§4b item 2), otherwise null.
export const AncestorAnchorsSchema = z.object({
  text: z.string().nullable(),
  role: z.null(),
  href: z.null(),
  alt: z.null(),
  ariaLabel: z.null(),
  nearestHeading: z.string().nullable(),
  landmark: z.string().nullable(),
  ordinalInLandmark: z.number().int().positive().nullable(),
});
export type AncestorAnchors = z.infer<typeof AncestorAnchorsSchema>;

export const AncestorDescriptorSchema = z.object({
  id: z.string(),
  tag: z.string(),
  bbox: z.tuple([z.number(), z.number(), z.number(), z.number()]),
  depth: z.number().int().nonnegative(),
  cssSelector: z.string().nullable(),
  anchors: AncestorAnchorsSchema,
});
export type AncestorDescriptor = z.infer<typeof AncestorDescriptorSchema>;

// ─── M4: StyleCandidates ──────────────────────────────────────────────────
export const StyleCandidatesSchema = z.object({
  /** True ancestor descriptors (not SemanticNode elements), sorted by document order. */
  ancestors: z.array(AncestorDescriptorSchema),
  /**
   * Per node: ordered list of ancestor ids (node_N or anc_N), nearest first.
   * Entries for dropped ancestors are omitted. Keys in node-id order.
   */
  chains: z.record(z.string(), z.array(z.string())),
  /** Maximum number of computedStyles entries allowed (always 2000). */
  budget: z.number().int().positive(),
  /** True if the ancestor set was truncated due to budget overflow. */
  truncated: z.boolean(),
  /** Number of ancestor entries dropped due to budget overflow. */
  droppedCount: z.number().int().nonnegative(),
});
export type StyleCandidates = z.infer<typeof StyleCandidatesSchema>;

// ─── CaptureBundle ────────────────────────────────────────────────────────
export const CaptureBundleSchema = z.object({
  schemaVersion: z.literal("1.0"),
  capturedAt: z.string(),
  viewport: ViewportConfigSchema,
  environment: EnvironmentFingerprintSchema,
  determinism: DeterminismRecordSchema,
  page: PageModelSchema,
  computedStyles: z.record(z.string(), z.record(z.string(), z.string())),
  styleCandidates: StyleCandidatesSchema,
  screenshots: z.object({
    fullPage: z.string(),
    viewport: z.string(),
  }),
});
export type CaptureBundle = z.infer<typeof CaptureBundleSchema>;
