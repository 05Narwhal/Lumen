import { describe, expect, it } from "vitest";
import { IDENTITY } from "@/hooks/use-edit-session";
import { editLayout, flipShown, nearestSpin, rotateBy, straightenZoom } from "./edit-geometry";

const close = (a: number, b: number, eps = 1e-6) => Math.abs(a - b) < eps;

describe("rotateBy", () => {
  it("turns the crop with the picture", () => {
    const r = rotateBy({ ...IDENTITY, crop: { x: 0.1, y: 0.2, w: 0.3, h: 0.4 } }, 1);
    expect(r.rotate).toBe(90);
    // Clockwise: the crop's top-left corner lands at (1 − y − h, x).
    expect(r.crop).toEqual({ x: 1 - 0.2 - 0.4, y: 0.1, w: 0.4, h: 0.3 });
    const back = rotateBy(r, -1);
    expect(back.rotate).toBe(0);
    expect(back.crop!.x).toBeCloseTo(0.1);
    expect(back.crop!.y).toBeCloseTo(0.2);
  });

  it("wraps the angle", () => {
    expect(rotateBy(IDENTITY, -1).rotate).toBe(270);
    expect(rotateBy({ ...IDENTITY, rotate: 270 }, 1).rotate).toBe(0);
  });
});

describe("flipShown", () => {
  it("flips what is on screen, whichever way the photo is turned", () => {
    expect(flipShown(IDENTITY, "h").flip_h).toBe(true);
    // Turned a quarter, a left-right flip on screen is an up-down flip of the source.
    const turned = flipShown({ ...IDENTITY, rotate: 90 }, "h");
    expect(turned.flip_v).toBe(true);
    expect(turned.flip_h).toBe(false);
  });

  it("mirrors the crop and the straighten angle", () => {
    const r = flipShown({ ...IDENTITY, straighten: 3, crop: { x: 0.1, y: 0.2, w: 0.3, h: 0.4 } }, "h");
    expect(r.straighten).toBe(-3);
    expect(r.crop!.x).toBeCloseTo(0.6);
    expect(r.crop!.y).toBeCloseTo(0.2);
    const v = flipShown({ ...IDENTITY, crop: { x: 0.1, y: 0.2, w: 0.3, h: 0.4 } }, "v");
    expect(v.crop!.y).toBeCloseTo(0.4);
  });
});

describe("nearestSpin", () => {
  it("keeps turning the short way", () => {
    expect(nearestSpin(0, 270)).toBe(-90);
    expect(nearestSpin(360, 90)).toBe(450);
    expect(nearestSpin(-90, 180)).toBe(-180);
  });
});

describe("straightenZoom", () => {
  it("matches the renderer's zoom", () => {
    expect(straightenZoom(0, 1.5)).toBe(1);
    const t = (10 * Math.PI) / 180;
    expect(close(straightenZoom(10, 1.5), Math.cos(t) + Math.sin(t) * 1.5)).toBe(true);
    expect(close(straightenZoom(-10, 1 / 1.5), straightenZoom(10, 1.5))).toBe(true);
  });
});

describe("editLayout", () => {
  const stage = { width: 1000, height: 600 };

  it("fits the whole frame while cropping", () => {
    const l = editLayout({ imageAspect: 1.5, recipe: IDENTITY, spin: 0, cropping: true, stage });
    expect(l.viewport).toEqual({ width: 900, height: 600 });
    expect(l.frame).toEqual({ width: 900, height: 600, left: 0, top: 0 });
    expect(l.image).toMatchObject({ width: 900, height: 600, left: 0, top: 0 });
  });

  it("swaps the frame on a quarter turn and keeps the image unrotated underneath", () => {
    const l = editLayout({ imageAspect: 1.5, recipe: { ...IDENTITY, rotate: 90 }, spin: 90, cropping: true, stage });
    expect(l.viewport).toEqual({ width: 400, height: 600 });
    expect(l.image).toMatchObject({ width: 600, height: 400, left: -100, top: 100 });
    expect(l.transform).toContain("rotate(90deg)");
  });

  it("shows only the crop outside crop mode, with the frame behind it offset", () => {
    const crop = { x: 0.5, y: 0, w: 0.5, h: 1 };
    const l = editLayout({ imageAspect: 1.5, recipe: { ...IDENTITY, crop }, spin: 0, cropping: false, stage });
    // Viewport aspect 0.75 fits 600 tall.
    expect(l.viewport).toEqual({ width: 450, height: 600 });
    expect(l.frame).toEqual({ width: 900, height: 600, left: -450, top: 0 });
  });

  it("puts flips before the turn", () => {
    const l = editLayout({ imageAspect: 1, recipe: { ...IDENTITY, flip_h: true, straighten: 5 }, spin: 0, cropping: false, stage });
    expect(l.transform).toMatch(/^rotate\(5deg\) scale\([\d.]+\) scale\(-1, 1\)$/);
  });
});
