import { describe, expect, it } from "vitest";
import { clampPan, clampScale, maxScale, oneToOneScale, zoomAround } from "./zoom-math";

const box = { width: 1000, height: 600 };
const fitted = { width: 800, height: 600 };

describe("zoom math", () => {
  it("bounds the scale between fit and a sane maximum", () => {
    const one = oneToOneScale({ width: 4000, height: 3000 }, fitted); // 5×
    expect(one).toBe(5);
    expect(maxScale(one)).toBe(10);
    expect(clampScale(0.2, one)).toBe(1);
    expect(clampScale(50, one)).toBe(10);
    expect(oneToOneScale(null, fitted)).toBe(2);
  });

  it("keeps the image inside the box", () => {
    expect(clampPan({ scale: 1, x: 300, y: 300 }, box, fitted)).toEqual({ scale: 1, x: 0, y: 0 });
    // 2×: image is 1600×1200, box 1000×600 → max offset 300 / 300
    expect(clampPan({ scale: 2, x: 900, y: -900 }, box, fitted)).toEqual({ scale: 2, x: 300, y: -300 });
    expect(clampPan({ scale: 2, x: 100, y: 50 }, box, fitted)).toEqual({ scale: 2, x: 100, y: 50 });
  });

  it("zooms around the cursor and snaps back to fit", () => {
    const z = zoomAround({ scale: 1, x: 0, y: 0 }, 2, 200, 100, box, fitted, 5);
    expect(z.scale).toBe(2);
    // The point under the cursor stays put: cx - (cx - x) * k → 200 - 200*2 = -200, clamped to -200 (max 300)
    expect(z.x).toBe(-200);
    expect(z.y).toBe(-100);
    const back = zoomAround(z, 0.1, 0, 0, box, fitted, 5);
    expect(back).toEqual({ scale: 1, x: 0, y: 0 });
  });
});
