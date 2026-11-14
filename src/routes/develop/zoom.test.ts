import { describe, expect, it } from "vitest";
import { clampPan, LEVELS, levelOf, nextLevel, scaleOf, sourceFor, visibleRect, wheelFactor, zoomAround, type Pan, type ZoomState } from "./zoom";

const photo = { w: 6000, h: 4000 };
const view = { w: 1200, h: 800 };
const fit: ZoomState = { mode: "fit" };

/** Where a photo point lands in the view, in CSS pixels. */
const onScreen = (pt: { x: number; y: number }, z: ZoomState, pan: Pan, dpr = 1) => {
  const s = scaleOf(z, photo, view, dpr);
  return { x: (pt.x - pan.x) * s, y: (pt.y - pan.y) * s };
};

describe("scaleOf", () => {
  it("fits, fills, and shows a level in screen pixels", () => {
    expect(scaleOf(fit, photo, view)).toBeCloseTo(0.2);
    expect(scaleOf({ mode: "fill" }, photo, view)).toBeCloseTo(0.2);
    expect(scaleOf(fit, { w: 4000, h: 4000 }, view)).toBeCloseTo(0.2);
    expect(scaleOf({ mode: "fill" }, { w: 4000, h: 4000 }, view)).toBeCloseTo(0.3);
    expect(scaleOf({ mode: "level", level: 1 }, photo, view)).toBe(1);
    // 100 % is one photo pixel per *device* pixel: half a CSS pixel on a 2× screen.
    expect(scaleOf({ mode: "level", level: 1 }, photo, view, 2)).toBe(0.5);
    expect(levelOf({ mode: "level", level: 2 }, photo, view, 2)).toBe(2);
    expect(levelOf(fit, photo, view, 2)).toBeCloseTo(0.4);
  });
});

describe("zoomAround", () => {
  it("keeps the photo point under the cursor where it was", () => {
    const anchor = { x: 300, y: 200 };
    let z: ZoomState = fit;
    let pan = clampPan({ x: 0, y: 0 }, z, photo, view);
    const s0 = scaleOf(z, photo, view);
    const pt = { x: pan.x + anchor.x / s0, y: pan.y + anchor.y / s0 };
    for (const f of [2, 3, 1.7, 0.6, 1.5]) {
      ({ zoom: z, pan } = zoomAround(z, f, anchor, pan, photo, view));
      const at = onScreen(pt, z, pan);
      expect(Math.abs(at.x - anchor.x)).toBeLessThan(0.5);
      expect(Math.abs(at.y - anchor.y)).toBeLessThan(0.5);
    }
  });

  it("stays between 5 % and 400 %", () => {
    expect(levelOf(zoomAround({ mode: "level", level: 3 }, 10, { x: 0, y: 0 }, { x: 0, y: 0 }, photo, view).zoom, photo, view)).toBe(4);
    expect(levelOf(zoomAround({ mode: "level", level: 0.1 }, 0.01, { x: 0, y: 0 }, { x: 0, y: 0 }, photo, view).zoom, photo, view)).toBe(0.05);
  });

  it("keeps the view inside the photo at its edges, and centres a photo smaller than the view", () => {
    const z: ZoomState = { mode: "level", level: 1 };
    expect(clampPan({ x: -50, y: 9999 }, z, photo, view)).toEqual({ x: 0, y: 3200 });
    expect(clampPan({ x: 10, y: 10 }, { mode: "level", level: 0.1 }, photo, view)).toEqual({ x: -3000, y: -2000 });
    // Zooming in at the corner, the pan cannot run past the edge.
    const r = zoomAround(fit, 20, { x: 0, y: 0 }, clampPan({ x: 0, y: 0 }, fit, photo, view), photo, view);
    expect(r.pan).toEqual({ x: 0, y: 0 });
  });

  it("accumulates a touchpad's small steps smoothly, without jumping by levels", () => {
    const anchor = { x: 600, y: 400 };
    let z: ZoomState = { mode: "level", level: 1 };
    let pan = clampPan({ x: 2400, y: 1600 }, z, photo, view);
    const levels: number[] = [];
    for (let i = 0; i < 100; i++) {
      ({ zoom: z, pan } = zoomAround(z, wheelFactor(-1), anchor, pan, photo, view));
      levels.push(levelOf(z, photo, view));
    }
    // A hundred 1-pixel deltas equal one 100-pixel delta, and every step is tiny.
    expect(levels.at(-1)).toBeCloseTo(wheelFactor(-100), 6);
    for (let i = 1; i < levels.length; i++) expect(levels[i] / levels[i - 1]).toBeLessThan(1.01);
    // A mouse wheel's notch is a clear step.
    expect(wheelFactor(100)).toBeLessThan(0.85);
    expect(wheelFactor(-100)).toBeGreaterThan(1.15);
  });
});

describe("nextLevel", () => {
  it("steps from fit through the levels above it and stops at 400 %", () => {
    expect(LEVELS).toEqual([0.25, 0.5, 1, 2, 4]);
    const seen: number[] = [];
    let z: ZoomState = fit;
    for (let i = 0; i < 7; i++) {
      z = nextLevel(z, 1, photo, view);
      seen.push(levelOf(z, photo, view));
    }
    expect(seen).toEqual([0.25, 0.5, 1, 2, 4, 4, 4]);
    for (const want of [2, 1, 0.5, 0.25]) {
      z = nextLevel(z, -1, photo, view);
      expect(levelOf(z, photo, view)).toBe(want);
    }
    expect(nextLevel(z, -1, photo, view)).toEqual(fit);
    expect(nextLevel(fit, -1, photo, view)).toEqual(fit);
  });

  it("skips levels below fit, and steps from a level in between", () => {
    // A 1600 px photo in a 1200 px view fits at 50 %.
    const small = { w: 1600, h: 1000 };
    expect(nextLevel(fit, 1, small, view)).toEqual({ mode: "level", level: 1 });
    expect(nextLevel({ mode: "level", level: 1.3 }, 1, photo, view)).toEqual({ mode: "level", level: 2 });
    expect(nextLevel({ mode: "level", level: 1.3 }, -1, photo, view)).toEqual({ mode: "level", level: 1 });
  });
});

describe("sourceFor", () => {
  it("uses the preview while the base has the pixels, the region above that", () => {
    // A 6000 px photo on a 2048 px base: 25 % needs 1500 px, 50 % needs 3000.
    expect(sourceFor(0.25, 2048, 6000)).toBe("preview");
    expect(sourceFor(2048 / 6000, 2048, 6000)).toBe("preview");
    expect(sourceFor(0.5, 2048, 6000)).toBe("region");
    expect(sourceFor(4, 2048, 6000)).toBe("region");
    // A photo no bigger than its base never needs the region.
    expect(sourceFor(4, 2048, 2048)).toBe("preview");
  });
});

describe("visibleRect", () => {
  it("names the visible part as fractions of the photo", () => {
    const z: ZoomState = { mode: "level", level: 1 };
    expect(visibleRect({ x: 1500, y: 1000 }, z, photo, view)).toEqual({ x: 0.25, y: 0.25, w: 0.2, h: 0.2 });
    const r = visibleRect({ x: -3000, y: -2000 }, { mode: "level", level: 0.1 }, photo, view);
    expect([r.x, r.y, r.w, r.h]).toEqual([0, 0, 1, 1]);
  });
});
