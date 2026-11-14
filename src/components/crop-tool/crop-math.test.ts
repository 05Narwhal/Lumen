import { describe, expect, it } from "vitest";
import { dragCrop, fitAspect } from "./crop-math";

const full = { x: 0, y: 0, w: 1, h: 1 };

describe("dragCrop", () => {
  it("moves an edge and never leaves the frame", () => {
    expect(dragCrop(full, "e", -0.2, 0, null, 1.5)).toEqual({ x: 0, y: 0, w: 0.8, h: 1 });
    expect(dragCrop(full, "w", -0.3, 0, null, 1.5).x).toBe(0);
  });
  it("keeps a minimum size", () => {
    const r = dragCrop(full, "se", -2, -2, null, 1.5);
    expect(r.w).toBeGreaterThanOrEqual(0.05);
    expect(r.h).toBeGreaterThanOrEqual(0.05);
  });
  it("moves the whole rectangle inside the frame", () => {
    const r = dragCrop({ x: 0.2, y: 0.2, w: 0.5, h: 0.5 }, "move", 0.6, -0.6, null, 1.5);
    expect(r).toEqual({ x: 0.5, y: 0, w: 0.5, h: 0.5 });
  });
  it("holds a pixel aspect on a non-square frame", () => {
    const r = dragCrop(full, "se", -0.4, 0, 1, 1.5); // 1:1 on a 3:2 frame
    expect((r.w * 1.5) / r.h).toBeCloseTo(1, 5);
  });
});

describe("fitAspect", () => {
  it("finds the largest centred square of a 3:2 frame", () => {
    const r = fitAspect(full, 1, 1.5);
    expect(r.h).toBeCloseTo(1);
    expect(r.w).toBeCloseTo(1 / 1.5);
    expect(r.x).toBeCloseTo((1 - 1 / 1.5) / 2);
  });
});
