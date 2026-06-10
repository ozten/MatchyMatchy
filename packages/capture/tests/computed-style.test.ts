import { describe, it, expect } from "vitest";
import {
  COMPUTED_STYLE_PROPS,
  COMPUTED_STYLE_VALUE_MAX_LEN,
  capStyleValue,
} from "../src/extract/computed-style.js";

describe("COMPUTED_STYLE_PROPS", () => {
  it("contains all required curated properties", () => {
    const required = [
      "color",
      "background-color",
      "background-image",
      "background",
      "border",
      "border-radius",
      "box-shadow",
      "font-family",
      "font-size",
      "font-weight",
      "line-height",
      "letter-spacing",
      "text-align",
      "padding-top",
      "padding-right",
      "padding-bottom",
      "padding-left",
      "margin-top",
      "margin-right",
      "margin-bottom",
      "margin-left",
      "display",
      "position",
      "opacity",
      "flex-direction",
      "justify-content",
      "align-items",
      "gap",
      "grid-template-columns",
    ];
    for (const prop of required) {
      expect(COMPUTED_STYLE_PROPS).toContain(prop);
    }
  });

  it("has 29 properties total", () => {
    expect(COMPUTED_STYLE_PROPS.length).toBe(29);
  });

  it("includes background shorthand (captured but excluded from diff list in analyze)", () => {
    expect(COMPUTED_STYLE_PROPS).toContain("background");
  });

  it("includes border shorthand (stays in diff list)", () => {
    expect(COMPUTED_STYLE_PROPS).toContain("border");
  });

  it("includes all four padding longhands", () => {
    expect(COMPUTED_STYLE_PROPS).toContain("padding-top");
    expect(COMPUTED_STYLE_PROPS).toContain("padding-right");
    expect(COMPUTED_STYLE_PROPS).toContain("padding-bottom");
    expect(COMPUTED_STYLE_PROPS).toContain("padding-left");
  });

  it("includes all four margin longhands", () => {
    expect(COMPUTED_STYLE_PROPS).toContain("margin-top");
    expect(COMPUTED_STYLE_PROPS).toContain("margin-right");
    expect(COMPUTED_STYLE_PROPS).toContain("margin-bottom");
    expect(COMPUTED_STYLE_PROPS).toContain("margin-left");
  });

  it("includes layout container properties for G1", () => {
    expect(COMPUTED_STYLE_PROPS).toContain("flex-direction");
    expect(COMPUTED_STYLE_PROPS).toContain("justify-content");
    expect(COMPUTED_STYLE_PROPS).toContain("align-items");
    expect(COMPUTED_STYLE_PROPS).toContain("gap");
    expect(COMPUTED_STYLE_PROPS).toContain("grid-template-columns");
  });

  it("is a readonly list with no duplicates", () => {
    const set = new Set(COMPUTED_STYLE_PROPS);
    expect(set.size).toBe(COMPUTED_STYLE_PROPS.length);
  });
});

describe("COMPUTED_STYLE_VALUE_MAX_LEN", () => {
  it("is 1000", () => {
    expect(COMPUTED_STYLE_VALUE_MAX_LEN).toBe(1000);
  });
});

describe("capStyleValue", () => {
  it("returns short values unchanged", () => {
    expect(capStyleValue("rgb(0, 0, 0)")).toBe("rgb(0, 0, 0)");
    expect(capStyleValue("16px")).toBe("16px");
    expect(capStyleValue("")).toBe("");
  });

  it("caps values at 1000 characters", () => {
    const long = "a".repeat(1200);
    const result = capStyleValue(long);
    expect(result.length).toBe(1000);
    expect(result).toBe("a".repeat(1000));
  });

  it("caps at exactly 1000 (boundary)", () => {
    const exact = "x".repeat(1000);
    expect(capStyleValue(exact).length).toBe(1000);

    const oneBeyond = "x".repeat(1001);
    expect(capStyleValue(oneBeyond).length).toBe(1000);
  });

  it("strips C0 control characters", () => {
    // NUL, BEL, BS, VT, FF, shift-in etc.
    expect(capStyleValue("rgb\x00(1,2,3)")).toBe("rgb(1,2,3)");
    expect(capStyleValue("val\x01ue")).toBe("value");
    expect(capStyleValue("val\x08ue")).toBe("value");
    expect(capStyleValue("val\x0bue")).toBe("value");
    expect(capStyleValue("val\x0cue")).toBe("value");
    expect(capStyleValue("val\x1fue")).toBe("value");
    expect(capStyleValue("val\x7fue")).toBe("value");
  });

  it("strips C1 control characters (0x80-0x9F)", () => {
    expect(capStyleValue("val\x80ue")).toBe("value");
    expect(capStyleValue("val\x9fue")).toBe("value");
  });

  it("does not strip normal whitespace (tabs, newlines are legal in CSS values)", () => {
    // Normal spaces are kept
    expect(capStyleValue("linear-gradient(to right, red, blue)")).toBe(
      "linear-gradient(to right, red, blue)"
    );
  });

  it("applies strip before cap (strip then cap on result)", () => {
    // Build a value with many control chars + normal chars to verify order
    const controlChars = "\x00".repeat(200);
    const normalChars = "a".repeat(900);
    const input = controlChars + normalChars;
    // After stripping: 900 chars of 'a'
    const result = capStyleValue(input);
    expect(result).toBe("a".repeat(900));
    expect(result.length).toBe(900);
  });

  it("handles a realistic CSS value", () => {
    const gradient =
      "linear-gradient(90deg, rgb(109, 40, 217) 0%, rgb(37, 99, 235) 100%)";
    expect(capStyleValue(gradient)).toBe(gradient);
  });
});
