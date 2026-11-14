import { describe, expect, it } from "vitest";
import { nextCompare, paneSize, sideBySideAxis, splitClip } from "./compare";

describe("splitClip", () => {
  it("gives the before the left of the divider and the after the right", () => {
    expect(splitClip(0.3, { w: 1000, h: 600 })).toEqual({
      before: { x: 0, y: 0, w: 300, h: 600 },
      after: { x: 300, y: 0, w: 700, h: 600 },
    });
    expect(splitClip(-1, { w: 1000, h: 600 }).before.w).toBe(0);
    expect(splitClip(2, { w: 1000, h: 600 }).after.w).toBe(0);
  });
});

describe("sideBySideAxis", () => {
  it("side_by_side_picks_the_axis_by_shape", () => {
    // A portrait in a landscape window: left and right.
    expect(sideBySideAxis({ w: 3000, h: 4000 }, { w: 1400, h: 800 })).toBe("row");
    // A landscape in a narrow window: top and bottom.
    expect(sideBySideAxis({ w: 6000, h: 4000 }, { w: 700, h: 900 })).toBe("column");
    // A landscape in a wide window still goes side by side when that shows it larger.
    expect(sideBySideAxis({ w: 4000, h: 3000 }, { w: 2400, h: 700 })).toBe("row");
  });

  it("halves the stage along the axis, less the gap", () => {
    expect(paneSize({ w: 1208, h: 800 }, "row", 8)).toEqual({ w: 600, h: 800 });
    expect(paneSize({ w: 1200, h: 808 }, "column", 8)).toEqual({ w: 1200, h: 400 });
  });
});

describe("nextCompare", () => {
  it("cycles split, side by side, off (Y, as in Lightroom)", () => {
    expect(nextCompare("off")).toBe("split");
    expect(nextCompare("toggle")).toBe("split");
    expect(nextCompare("split")).toBe("side");
    expect(nextCompare("side")).toBe("off");
  });
});
