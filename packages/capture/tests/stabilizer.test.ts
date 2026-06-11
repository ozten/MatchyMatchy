import { describe, it, expect } from "vitest";
import { shouldRetryWithoutFreeze } from "../src/stabilizer.js";
import type { DeterminismRecord } from "../src/schema.js";

/** A DeterminismRecord with all steps "ran" — used as base for mutations. */
function allRan(): Pick<
  DeterminismRecord,
  | "animationsDisabled"
  | "reducedMotion"
  | "timeFrozen"
  | "randomStubbed"
  | "fontsReady"
  | "imagesDecoded"
  | "lazyLoadPass"
  | "settled"
> {
  return {
    animationsDisabled: "ran",
    reducedMotion: "ran",
    timeFrozen: "ran",
    randomStubbed: "ran",
    fontsReady: "ran",
    imagesDecoded: "ran",
    lazyLoadPass: "ran",
    settled: "ran",
  };
}

describe("shouldRetryWithoutFreeze", () => {
  it("returns true when a step failed, clock was installed, and not yet retried", () => {
    const det = { ...allRan(), settled: "failed" as const };
    expect(shouldRetryWithoutFreeze(det, /* clockInstalled */ true, /* alreadyRetried */ false)).toBe(true);
  });

  it("returns false when a step failed but clock was NOT installed", () => {
    const det = { ...allRan(), lazyLoadPass: "failed" as const };
    expect(shouldRetryWithoutFreeze(det, /* clockInstalled */ false, /* alreadyRetried */ false)).toBe(false);
  });

  it("returns false when already retried (even if step failed + clock installed)", () => {
    const det = { ...allRan(), fontsReady: "failed" as const };
    expect(shouldRetryWithoutFreeze(det, /* clockInstalled */ true, /* alreadyRetried */ true)).toBe(false);
  });

  it("returns false when all steps ran (no failure)", () => {
    const det = allRan();
    expect(shouldRetryWithoutFreeze(det, /* clockInstalled */ true, /* alreadyRetried */ false)).toBe(false);
  });

  it("returns true when timeFrozen step itself failed (clock was being installed)", () => {
    const det = { ...allRan(), timeFrozen: "failed" as const };
    expect(shouldRetryWithoutFreeze(det, /* clockInstalled */ true, /* alreadyRetried */ false)).toBe(true);
  });

  it("returns false when all steps skipped and no failure", () => {
    const det = {
      animationsDisabled: "skipped" as const,
      reducedMotion: "skipped" as const,
      timeFrozen: "skipped" as const,
      randomStubbed: "skipped" as const,
      fontsReady: "skipped" as const,
      imagesDecoded: "skipped" as const,
      lazyLoadPass: "skipped" as const,
      settled: "skipped" as const,
    };
    expect(shouldRetryWithoutFreeze(det, true, false)).toBe(false);
  });
});
