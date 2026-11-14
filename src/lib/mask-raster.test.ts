import { describe, expect, it } from "vitest";
import type { Mask } from "@/types/generated/Mask";
import type { Recipe } from "@/types/generated/Recipe";
import golden from "./mask-golden.json";
import { frameMap, fromFrame, quantise, rasterize, StrokeTint, toFrame, type AiRasters } from "./mask-raster";

interface Case {
  name: string;
  base: [number, number];
  recipe: Recipe;
  mask: Mask;
  width: number;
  height: number;
  weights: number[];
}

describe("mask-raster", () => {
  const { rasters } = golden as unknown as { rasters: AiRasters };

  it("covers the AI shapes", () => {
    expect((golden as unknown as { cases: Case[] }).cases.filter((c) => c.mask.shapes.some((s) => s.kind === "ai")).length).toBeGreaterThanOrEqual(3);
  });

  it("gives an AI shape with no raster no weight", () => {
    const shape = { kind: "ai", add: true, target: "sky", instance: 0, raster: "missing" } as const;
    const mask = { id: 1, name: "Sky", shapes: [shape], invert: false, opacity: 1, adjust: {}, visible: true } as unknown as Mask;
    const got = rasterize(mask, frameMap(8, 6, { flip_h: false, flip_v: false, rotate: 0, straighten: 0, crop: null }), 8, 6, rasters);
    expect(got.every((v) => v === 0)).toBe(true);
  });

  for (const c of (golden as unknown as { cases: Case[] }).cases) {
    it(`matches Rust's weights for ${c.name} within 1/255`, () => {
      const f = frameMap(c.base[0], c.base[1], c.recipe);
      const got = rasterize(c.mask, f, c.width, c.height, rasters);
      expect(got.length).toBe(c.weights.length);
      let worst = 0;
      for (let i = 0; i < got.length; i++) worst = Math.max(worst, Math.abs(got[i] - c.weights[i]));
      expect(worst).toBeLessThanOrEqual(1 / 255);
    });
  }

  it("maps a frame point back to the unedited photo", () => {
    const r = { rotate: 90, flip_h: true, flip_v: false, straighten: 7, crop: { x: 0.1, y: 0.2, w: 0.6, h: 0.7 } } as Recipe;
    const f = frameMap(400, 300, r);
    for (const p of [[0.3, 0.4], [0.75, 0.6]] as [number, number][]) {
      const q = fromFrame(f, toFrame(f, p[0], p[1]));
      expect(q[0]).toBeCloseTo(p[0], 5);
      expect(q[1]).toBeCloseTo(p[1], 5);
    }
  });

  it("paints a stroke dab by dab to the same weights as rasterising the whole mask", () => {
    const c = (golden as unknown as { cases: Case[] }).cases.find((x) => x.name === "combined")!;
    const f = frameMap(c.base[0], c.base[1], c.recipe);
    const before = { ...c.mask, shapes: c.mask.shapes.slice(0, 2) };
    const brush = { radius: 0.06, feather: 0.4, flow: 0.8, add: true };
    const points: [number, number][] = [[0.1, 0.2], [0.3, 0.35], [0.6, 0.3], [0.8, 0.7]];
    const tint = new StrokeTint(before, f, c.width, c.height, brush);
    for (let i = 1; i <= points.length; i++) tint.extend(points.slice(0, i).map(quantise));
    const whole = rasterize({ ...before, shapes: [...before.shapes, { kind: "brush", ...brush, points: points.map(quantise) }] }, f, c.width, c.height);
    let worst = 0;
    for (let i = 0; i < whole.length; i++) worst = Math.max(worst, Math.abs(whole[i] - tint.weights[i]));
    expect(worst).toBeLessThan(1e-4);
    expect(tint.dirty).not.toBeNull();
  });

  it("quantises points to the 4096 grid and keeps them inside it", () => {
    expect(quantise([0.5, 1.2])).toEqual([2048, 4096]);
    expect(quantise([-0.1, 0.25])).toEqual([0, 1024]);
  });
});
