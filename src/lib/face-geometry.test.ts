import { describe, expect, it } from "vitest";
import type { Recipe } from "@/types/generated/Recipe";
import { rectToOriginal, rectToShown, toOriginal, toShown, visible } from "./face-geometry";

const plain: Recipe = { version: 2, crop: null, rotate: 0, straighten: 0, flip_h: false, flip_v: false, develop: null, trim: null };
const close = (a: number[], b: number[]) => a.forEach((v, i) => expect(v).toBeCloseTo(b[i], 5));

describe("face geometry", () => {
  it("turns clockwise the way the renderer does: the top-left corner goes top-right", () => {
    close(toShown([0, 0], { ...plain, rotate: 90 }, 1.5), [1, 0]);
    close(toShown([0, 0], { ...plain, rotate: 270 }, 1.5), [0, 1]);
    close(toShown([0, 0], { ...plain, rotate: 180 }, 1.5), [1, 1]);
    close(toShown([0.25, 0], { ...plain, flip_h: true }, 1.5), [0.75, 0]);
  });

  it("crops", () => {
    const r = { ...plain, crop: { x: 0.5, y: 0.25, w: 0.5, h: 0.5 } };
    close(toShown([0.75, 0.5], r, 1.5), [0.5, 0.5]);
    expect(visible(rectToShown([0, 0, 0.2, 0.2], r, 1.5))).toBe(false);
  });

  it("goes there and back through every step at once", () => {
    const recipes: Recipe[] = [
      { ...plain, rotate: 90, straighten: 7.5, crop: { x: 0.1, y: 0.05, w: 0.7, h: 0.6 } },
      { ...plain, rotate: 270, flip_h: true, flip_v: true, straighten: -3, crop: { x: 0.2, y: 0.2, w: 0.5, h: 0.5 } },
      { ...plain, rotate: 180, straighten: 12 },
    ];
    for (const r of recipes) {
      for (const p of [
        [0.4, 0.5],
        [0.3, 0.6],
        [0.55, 0.45],
      ] as [number, number][]) {
        close(toOriginal(toShown(p, r, 4 / 3), r, 4 / 3), p);
      }
    }
  });

  it("straightening zooms in: the centre stays, a point off it moves out", () => {
    const r = { ...plain, straighten: 10 };
    close(toShown([0.5, 0.5], r, 1.5), [0.5, 0.5]);
    const [x] = toShown([0.75, 0.5], r, 1.5);
    expect(x).toBeGreaterThan(0.75);
  });

  it("a drawn box maps back into the unedited photo and is clamped to it", () => {
    const r = { ...plain, rotate: 90 };
    close(rectToOriginal([0.5, 0, 0.5, 0.5], r, 2), [0, 0, 0.5, 0.5]);
    close(rectToOriginal([-0.1, -0.1, 0.3, 0.3], null, 1), [0, 0, 0.2, 0.2]);
  });
});
