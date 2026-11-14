import { describe, expect, it } from "vitest";
import { contrastRatio, oklchToRgb, parseOklch } from "./color";

describe("color", () => {
  it("parses oklch with and without alpha", () => {
    expect(parseOklch("oklch(62% 0.19 265)")).toEqual({ l: 0.62, c: 0.19, h: 265, alpha: 1 });
    expect(parseOklch("oklch(100% 0 0 / 0.55)")).toEqual({ l: 1, c: 0, h: 0, alpha: 0.55 });
    expect(parseOklch("#fff")).toBeNull();
  });

  it("converts white and black", () => {
    expect(oklchToRgb(1, 0, 0)).toEqual([255, 255, 255]);
    expect(oklchToRgb(0, 0, 0)).toEqual([0, 0, 0]);
  });

  it("computes contrast", () => {
    expect(contrastRatio([0, 0, 0], [255, 255, 255])).toBeCloseTo(21, 0);
    expect(contrastRatio([255, 255, 255], [255, 255, 255])).toBeCloseTo(1, 5);
  });
});
