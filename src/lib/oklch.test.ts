import { describe, expect, it } from "vitest";
import { oklchToRgb } from "./oklch";

describe("oklchToRgb", () => {
  it("converts the theme tokens' oklch() to sRGB bytes", () => {
    expect(oklchToRgb("oklch(100% 0 0)")).toEqual([255, 255, 255]);
    expect(oklchToRgb("oklch(0% 0 0)")).toEqual([0, 0, 0]);
    expect(oklchToRgb("oklch(12% 0 0)")).toEqual([6, 6, 6]);
    expect(oklchToRgb("oklch(95% 0.012 260)")).toEqual([234, 239, 247]);
    expect(oklchToRgb(" oklch(20% 0.02 260) ")).toEqual([17, 22, 31]);
  });

  it("accepts a 0–1 lightness and returns null for anything else", () => {
    expect(oklchToRgb("oklch(0.12 0 0)")).toEqual([6, 6, 6]);
    expect(oklchToRgb("#fff")).toBeNull();
    expect(oklchToRgb("")).toBeNull();
  });
});
