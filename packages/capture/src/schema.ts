import { z } from "zod";
import { DEFAULT_REDACT_PARAMS } from "./normalize.js";

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

// ─── M9: settle mode vocabulary ───────────────────────────────────────────
// Vocabulary emitted/consumed by the Rust orchestrator's build_capture_config
// (packages/analyze/src/orchestrate.rs). New values require extending this
// enum + the vocabulary-guard tests in tests/schema.test.ts (self-check lesson:
// contract CI does not cover this config seam).
export const SettleModeSchema = z.enum(["full", "legacy", "off"]);
export type SettleMode = z.infer<typeof SettleModeSchema>;

// ─── Stabilization config ─────────────────────────────────────────────────
export const StabilizationConfigSchema = z.object({
  freezeTime: z.boolean().default(true),
  fixedTime: z.string().default("2026-01-01T00:00:00.000Z"),
  stubRandom: z.boolean().default(true),
  randomSeed: z.number().int().default(1337),
  networkIdleTimeoutMs: z.number().int().positive().default(15000),
  settleMs: z.number().int().nonnegative().default(1000),
  lazyScrollStepPx: z.number().int().positive().default(800),
  /**
   * M9: settle stage behavior. "legacy" (default, current behavior) keeps step
   * 8 as the original scroll-steps + clock dwell + image-await lazyLoadPass;
   * "full" is the evolved settle stage (viewport-height steps + quiescence
   * wait) — the default flips to "full" in a later commit, not this one;
   * "off" skips step 8 entirely (config-file only; not CLI-exposed).
   */
  settleMode: SettleModeSchema.default("legacy"),
  /** M9: fixed dwell per settle scroll step, in ms. Optional; stabilizer applies its own default when absent. */
  settleDwellMs: z.number().int().nonnegative().optional(),
  /** M9: no-mutation window required to declare quiescence reached, in ms. */
  quiescenceWindowMs: z.number().int().nonnegative().optional(),
  /** M9: hard timeout bounding the quiescence wait, in ms. */
  quiescenceTimeoutMs: z.number().int().nonnegative().optional(),
  /** M9: max settle scroll steps before the growth cap kicks in. */
  maxSettleSteps: z.number().int().positive().optional(),
});
export type StabilizationConfig = z.infer<typeof StabilizationConfigSchema>;

// ─── CaptureConfig ────────────────────────────────────────────────────────
export const CaptureConfigSchema = z.object({
  mode: z.enum(["capture", "doctor"]),
  url: z.string().optional(),
  outDir: z.string().optional(),
  // Vocabulary emitted by the Rust runner's build_capture_config call sites
  // (packages/analyze/src/bin/matchy.rs, run_self_check): "old", "new", "old-selfcheck".
  // New Rust-side prefixes require extending this enum + the vocabulary test in tests/schema.test.ts.
  prefix: z.enum(["old", "new", "old-selfcheck"]).optional(),
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
  redactParams: z.array(z.string()).default(DEFAULT_REDACT_PARAMS),
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

// ─── IntegrityCounts ──────────────────────────────────────────────────────
export const IntegrityCountsSchema = z.object({
  headingCount: z.number().int().nonnegative(),
  imageCount: z.number().int().nonnegative(),
  landmarkCount: z.number().int().nonnegative(),
});
export type IntegrityCounts = z.infer<typeof IntegrityCountsSchema>;

// ─── IntegrityInventory ───────────────────────────────────────────────────
export const IntegrityInventorySchema = z.object({
  pre: IntegrityCountsSchema,
  post: IntegrityCountsSchema,
});
export type IntegrityInventory = z.infer<typeof IntegrityInventorySchema>;

// ─── M9: quiescence status ────────────────────────────────────────────────
export const QuiescenceStatusSchema = z.enum(["reached", "timeout", "notRun"]);
export type QuiescenceStatus = z.infer<typeof QuiescenceStatusSchema>;

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
  /** M9: status of the evolved settle stage. Optional; absent pre-1.1 and when the stage did not run. */
  settle: DeterminismStepSchema.optional(),
  /** M9: status of the per-node clickable-area hit-test probe. Optional; absent pre-1.1 and when the probe did not run. */
  hitTestProbe: DeterminismStepSchema.optional(),
  /** M9: outcome of the settle stage's quiescence wait. Optional; absent pre-1.1. */
  quiescence: QuiescenceStatusSchema.optional(),
  /** M9: true when the settle stage's scroll steps never moved scrollY. */
  settleScrollIneffective: z.boolean().optional(),
  /** M9: true when the settle stage's page-growth cap was hit. */
  settleGrowthCapped: z.boolean().optional(),
  clicked: z.array(z.string()),
  hidden: z.array(z.string()),
  masked: z.array(z.string()),
  retriedWithoutTimeFreeze: z.boolean(),
  /** Pre/post stabilization page inventory. Optional; absent when the evaluate failed. */
  integrity: IntegrityInventorySchema.optional(),
});
export type DeterminismRecord = z.infer<typeof DeterminismRecordSchema>;

// ─── LandmarkRect ─────────────────────────────────────────────────────────
export const LandmarkRectSchema = z.object({
  path: z.string(),
  role: z.string(),
  heading: z.string().nullable(),
  bbox: z.tuple([z.number().int(), z.number().int(), z.number().int(), z.number().int()]),
});
export type LandmarkRect = z.infer<typeof LandmarkRectSchema>;

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
  /** WP-G: geometry for landmark elements and main's children. Absent in old bundles. */
  landmarkRects: z.array(LandmarkRectSchema).optional(),
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

// ─── M9: HitTest ──────────────────────────────────────────────────────────
export const HitTestOutcomeSchema = z.enum(["hit", "miss", "clipped", "offViewport"]);
export type HitTestOutcome = z.infer<typeof HitTestOutcomeSchema>;

/**
 * A single grid-point outcome. Coordinates are never stored — only the
 * outcome (and, for a miss, the winning element's selector). `winner` is
 * required iff `o === "miss"`, enforced below (JSON Schema mirrors this as
 * an if/then; additionalProperties stays permissive at the zod layer to
 * match).
 */
export const HitTestPointSchema = z
  .object({
    o: HitTestOutcomeSchema,
    winner: z.string().optional(),
  })
  .superRefine((val, ctx) => {
    if (val.o === "miss" && val.winner === undefined) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: "winner is required when o is 'miss'",
        path: ["winner"],
      });
    }
  });
export type HitTestPoint = z.infer<typeof HitTestPointSchema>;

export const HitTestSkipReasonSchema = z.enum(["tooSmall", "offDocument", "detached"]);
export type HitTestSkipReason = z.infer<typeof HitTestSkipReasonSchema>;

/**
 * Per-node clickable-area hit-test result. `skipReason` required iff
 * status === "skipped"; `gridSize`/`points` required iff status === "sampled".
 */
export const HitTestEntrySchema = z
  .object({
    status: z.enum(["sampled", "skipped"]),
    skipReason: HitTestSkipReasonSchema.optional(),
    gridSize: z.number().int().positive().optional(),
    points: z.array(HitTestPointSchema).length(25).optional(),
  })
  .superRefine((val, ctx) => {
    if (val.status === "skipped" && val.skipReason === undefined) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: "skipReason is required when status is 'skipped'",
        path: ["skipReason"],
      });
    }
    if (val.status === "sampled") {
      if (val.gridSize === undefined) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: "gridSize is required when status is 'sampled'",
          path: ["gridSize"],
        });
      }
      if (val.points === undefined) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: "points is required when status is 'sampled'",
          path: ["points"],
        });
      }
    }
  });
export type HitTestEntry = z.infer<typeof HitTestEntrySchema>;

// ─── M9: PseudoElements ───────────────────────────────────────────────────
export const PseudoStylesSchema = z.object({
  content: z.string(),
  position: z.string().optional(),
  width: z.string().optional(),
  height: z.string().optional(),
  "background-color": z.string().optional(),
  "background-image": z.string().optional(),
  border: z.string().optional(),
  "border-radius": z.string().optional(),
  top: z.string().optional(),
  right: z.string().optional(),
  bottom: z.string().optional(),
  left: z.string().optional(),
  "z-index": z.string().optional(),
  display: z.string().optional(),
  opacity: z.string().optional(),
  /** Best-effort [x, y, width, height] in page coordinates. Absent when unresolvable. */
  bbox: z.tuple([z.number(), z.number(), z.number(), z.number()]).optional(),
});
export type PseudoStyles = z.infer<typeof PseudoStylesSchema>;

export const PseudoOwnerTierSchema = z.enum(["node", "ancestor", "selector"]);
export type PseudoOwnerTier = z.infer<typeof PseudoOwnerTierSchema>;

/**
 * A captured ::before/::after pair for a single owner. `ownerNodeId` required
 * iff ownerTier === "node"; `ownerSelector` required iff ownerTier is
 * "ancestor" or "selector".
 */
export const PseudoElementEntrySchema = z
  .object({
    ownerTier: PseudoOwnerTierSchema,
    ownerNodeId: z.string().optional(),
    ownerSelector: z.string().optional(),
    landmark: z.string().nullable(),
    before: PseudoStylesSchema.optional(),
    after: PseudoStylesSchema.optional(),
  })
  .superRefine((val, ctx) => {
    if (val.ownerTier === "node" && val.ownerNodeId === undefined) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: "ownerNodeId is required when ownerTier is 'node'",
        path: ["ownerNodeId"],
      });
    }
    if (
      (val.ownerTier === "ancestor" || val.ownerTier === "selector") &&
      val.ownerSelector === undefined
    ) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: "ownerSelector is required when ownerTier is 'ancestor' or 'selector'",
        path: ["ownerSelector"],
      });
    }
  });
export type PseudoElementEntry = z.infer<typeof PseudoElementEntrySchema>;

/** Present only when the per-page pseudo-element budget was exceeded. */
export const PseudoTruncatedSchema = z.object({
  droppedCount: z.number().int().nonnegative(),
});
export type PseudoTruncated = z.infer<typeof PseudoTruncatedSchema>;

// ─── CaptureBundle ────────────────────────────────────────────────────────
export const CaptureBundleSchema = z.object({
  schemaVersion: z.literal("1.1"),
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
  /** M9: per-node hit-test probe results, keyed by SemanticNode id. Absent when the probe did not run. */
  hitTests: z.record(z.string(), HitTestEntrySchema).optional(),
  /** M9: captured ::before/::after entries, keyed by owner key. Absent when the pseudo scan did not run. */
  pseudoElements: z.record(z.string(), PseudoElementEntrySchema).optional(),
  /** M9: present only when the pseudo-element budget was exceeded. */
  pseudoTruncated: PseudoTruncatedSchema.optional(),
});
export type CaptureBundle = z.infer<typeof CaptureBundleSchema>;
