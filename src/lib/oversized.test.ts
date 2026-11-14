import { describe, expect, it } from "vitest";
import { isOversized, shownPercent } from "./oversized";

describe("oversized images", () => {
  it("knows an oversized photo, and how much of it is shown", () => {
    expect(isOversized(16000, 12000)).toBe(true);
    expect(isOversized(100000, 4000)).toBe(true);
    expect(isOversized(8000, 6000)).toBe(false);
    expect(isOversized(null, 6000)).toBe(false);
    expect(shownPercent({ width: 57920, height: 2317 }, { width: 100000, height: 4000 })).toBe(58);
    expect(shownPercent({ width: 2317, height: 57920 }, { width: 100000, height: 4000 })).toBe(58);
    expect(shownPercent({ width: 16000, height: 12000 }, { width: 16000, height: 12000 })).toBeNull();
  });
});
