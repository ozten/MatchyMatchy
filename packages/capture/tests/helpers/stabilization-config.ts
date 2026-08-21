import type { StabilizationConfig } from "../../src/schema.js";

/**
 * A fully-populated `StabilizationConfig` (every zod-defaulted field spelled
 * out explicitly) for tests that call `stabilize()` directly rather than
 * going through `CaptureConfigSchema.parse()`. `overrides` wins.
 */
export function baseStabilizationConfig(
  overrides: Partial<StabilizationConfig> = {}
): StabilizationConfig {
  return {
    freezeTime: true,
    fixedTime: "2026-01-01T00:00:00.000Z",
    stubRandom: true,
    randomSeed: 1337,
    networkIdleTimeoutMs: 15000,
    settleMs: 1000,
    lazyScrollStepPx: 800,
    settleMode: "legacy",
    ...overrides,
  };
}
