import { renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { DevelopSession } from "@/hooks/use-develop-session";
import type { Develop } from "@/types/generated/Develop";
import type { Mask } from "@/types/generated/Mask";
import { DEFAULT_DEVELOP, ZERO_ADJUST } from "./develop";
import { addShape, adjustChanged, BRUSH_MAX_RADIUS, brushDrag, brushMinRadius, brushPixels, brushPosition, brushRadiusOf, brushSizeAt, stepBrushRadius, adjustFrom, adjustView, appendStroke, duplicateMask, holdMaskActive, maskSession, maskUi, moveMask, newMask, overlayAfter, radialResize, removeMask, setAdjust, updateMask, useMaskOverlayRule } from "./develop-masks";
import { frameMap, toFrame } from "./mask-raster";

const mask = (id: number, name = `Mask ${id}`): Mask => ({ id, name, shapes: [], invert: false, opacity: 1, adjust: ZERO_ADJUST, visible: true });

describe("develop-masks", () => {
  it("shows the overlay by default", () => {
    expect(maskUi.get().overlay).toBe(true);
    expect(maskUi.get().active).toBe(false);
  });

  it("is active from a press until the pointer is released anywhere", () => {
    holdMaskActive();
    expect(maskUi.get().active).toBe(true);
    window.dispatchEvent(new Event("pointerup"));
    expect(maskUi.get().active).toBe(false);
    holdMaskActive();
    window.dispatchEvent(new Event("pointercancel"));
    expect(maskUi.get().active).toBe(false);
  });

  describe("the overlay rule (Round 8B)", () => {
    const shaped = { ...mask(1), shapes: [{ kind: "radial", add: true, center: [0.5, 0.5], radius: [0.2, 0.2], angle: 0, feather: 0.5 }] } as unknown as Mask;
    const moved = { ...shaped, shapes: [{ ...(shaped.shapes[0] as object), center: [0.6, 0.5] }] } as unknown as Mask;

    it("shows the overlay when a shape moves, resizes or is painted, even if it was off", () => {
      expect(overlayAfter(shaped, moved, false)).toBe(true);
      expect(overlayAfter(mask(1), appendStroke([mask(1)], 1, { radius: 0.05, feather: 0.5, flow: 1, add: true }, [[0.1, 0.1]])[0], false)).toBe(true);
    });

    it("hides it when the mask's photo adjustments change, so the result can be judged", () => {
      expect(overlayAfter(shaped, setAdjust([shaped], 1, { exposure: 0.4 })[0], true)).toBe(false);
    });

    it("leaves it as the user set it otherwise: another mask, or nothing that changed", () => {
      expect(overlayAfter(shaped, shaped, false)).toBe(false);
      expect(overlayAfter(shaped, shaped, true)).toBe(true);
      expect(overlayAfter(shaped, { ...moved, id: 2 }, false)).toBe(false);
      expect(overlayAfter(undefined, shaped, false)).toBe(false);
    });

    it("applies the rule to the shared state as the selected mask changes; O still toggles", () => {
      maskUi.set((u) => ({ ...u, overlay: false }));
      const { rerender } = renderHook(({ m }) => useMaskOverlayRule(m), { initialProps: { m: shaped as Mask | undefined } });
      expect(maskUi.get().overlay).toBe(false);
      rerender({ m: moved });
      expect(maskUi.get().overlay).toBe(true);
      rerender({ m: setAdjust([moved], 1, { contrast: 20 })[0] });
      expect(maskUi.get().overlay).toBe(false);
      maskUi.set((u) => ({ ...u, overlay: true }));
      rerender({ m: setAdjust([moved], 1, { contrast: 20 })[0] });
      expect(maskUi.get().overlay, "the same adjustments again change nothing").toBe(true);
    });
  });

  it("numbers new masks after the highest id and gives gradients a starting shape", () => {
    const radial = newMask([mask(1), mask(4)], "radial");
    expect(radial.id).toBe(5);
    expect(radial.name).toBe("Mask 5");
    expect(radial.shapes).toEqual([expect.objectContaining({ kind: "radial", add: true, center: [0.5, 0.5] })]);
    expect(newMask([], "brush").shapes).toEqual([]);
    expect(newMask([], "linear").shapes[0]).toMatchObject({ kind: "linear", add: true });
  });

  it("edits one mask and leaves the others", () => {
    const masks = [mask(1), mask(2)];
    const next = setAdjust(masks, 2, { exposure: 0.7 });
    expect(next[1].adjust.exposure).toBe(0.7);
    expect(next[0]).toBe(masks[0]);
    expect(updateMask(next, 1, { invert: true })[0].invert).toBe(true);
    expect(removeMask(next, 1).map((m) => m.id)).toEqual([2]);
  });

  it("duplicates under a new id and name", () => {
    const masks = [{ ...mask(3, "Sky"), adjust: { ...ZERO_ADJUST, exposure: -1 } }];
    const copy = duplicateMask(masks, 3);
    expect(copy.map((m) => m.id)).toEqual([3, 4]);
    expect(copy[1]).toMatchObject({ name: "Sky copy", adjust: { exposure: -1 } });
  });

  it("moves a mask to an index, clamped at the ends", () => {
    const masks = [mask(1), mask(2), mask(3)];
    const ids = (m: Mask[]) => m.map((x) => x.id);
    expect(ids(moveMask(masks, 1, 2))).toEqual([2, 3, 1]);
    expect(ids(moveMask(masks, 3, 0))).toEqual([3, 1, 2]);
    expect(ids(moveMask(masks, 2, 9))).toEqual([1, 3, 2]);
    expect(ids(moveMask(masks, 2, -4))).toEqual([2, 1, 3]);
    // Same place and an unknown id hand back the very same list.
    expect(moveMask(masks, 2, 1)).toBe(masks);
    expect(moveMask(masks, 99, 0)).toBe(masks);
  });

  it("adds and subtracts shapes, and strokes become quantised brushes", () => {
    let masks = [mask(1)];
    masks = addShape(masks, 1, "radial", false);
    expect(masks[0].shapes[0]).toMatchObject({ kind: "radial", add: false });
    masks = appendStroke(masks, 1, { add: true, radius: 0.05, feather: 0.5, flow: 1 }, [[0.25, 0.5], [0.5001, 0.75]]);
    expect(masks[0].shapes[1]).toEqual({ kind: "brush", add: true, radius: 0.05, feather: 0.5, flow: 1, points: [[1024, 2048], [2048, 3072]] });
  });

  it("resizes a radial along its own axis in the unedited photo, whatever the turn", () => {
    const shape = { kind: "radial" as const, add: true, center: [0.5, 0.5] as [number, number], radius: [0.2, 0.1] as [number, number], angle: 0, feather: 0.5, inside: true };
    // Turned a quarter: the photo's x axis runs down the screen.
    const f = frameMap(400, 300, { rotate: 90, flip_h: false, flip_v: false, straighten: 0, crop: null });
    // Drag the x handle to 0.3 of the width from the centre: it lands at unedited (0.8, 0.5).
    const p = toFrame(f, 0.8, 0.5);
    const grown = radialResize(shape, "x", f, p);
    expect(grown.radius[0]).toBeCloseTo(0.3, 5);
    expect(grown.radius[1]).toBe(0.1);
  });
});

describe("maskSession (Round 19)", () => {
  const first: Mask = { ...newMask([], "radial"), adjust: { ...ZERO_ADJUST, exposure: 0.5 } };
  const second: Mask = { ...newMask([first], "radial"), adjust: ZERO_ADJUST };
  const photo: Develop = { ...DEFAULT_DEVELOP, basic: { ...DEFAULT_DEVELOP.basic, exposure: 1 }, masks: [first, second] };
  const fake = () => {
    const calls: { kind: string; label?: string; next: Develop }[] = [];
    const session = {
      develop: photo,
      update: (c: (d: Develop) => Develop) => calls.push({ kind: "update", next: c(photo) }),
      apply: (c: (d: Develop) => Develop, label: string) => calls.push({ kind: "apply", label, next: c(photo) }),
    } as unknown as DevelopSession;
    return { session, calls };
  };

  it("reads the mask's adjustments, not the photo's", () => {
    const { session } = fake();
    expect(maskSession(session, first.id).develop.basic.exposure).toBe(0.5);
    expect(maskSession(session, second.id).develop.basic.exposure).toBe(0);
  });

  it("writes into that mask only, as one recorded step", () => {
    const { session, calls } = fake();
    maskSession(session, second.id).apply((d) => ({ ...d, basic: { ...d.basic, vibrance: 0.3 }, curve: { ...d.curve, master: [[0.5, 0.6]] } }), "Mask vibrance");
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ kind: "apply", label: "Mask vibrance" });
    const next = calls[0].next;
    expect(next.basic.exposure).toBe(1);
    expect(next.masks?.[0].adjust.exposure).toBe(0.5);
    expect(next.masks?.[1].adjust.vibrance).toBe(0.3);
    expect(next.masks?.[1].adjust.curve?.master).toEqual([[0.5, 0.6]]);
  });

  it("leaves values at default out, so an untouched mask writes back as it was", () => {
    const { session, calls } = fake();
    maskSession(session, second.id).update((d) => d);
    expect(calls[0].next.masks?.[1].adjust).toEqual(ZERO_ADJUST);
  });

  it("round-trips every tool, and says whether a mask does anything", () => {
    const a = { ...ZERO_ADJUST, vibrance: 0.2, sharpen: { radius: 2, detail: 0.5, masking: 0.1 }, grain: { amount: 0.4, size: 0.5 }, glow: { amount: 0.3, size: 0.6 }, motion_blur: { amount: 0.2, angle: 30 }, halation: { amount: 0.1, size: 0.5 } };
    expect(adjustFrom(adjustView(a))).toEqual(a);
    expect(adjustChanged(a)).toBe(true);
    expect(adjustChanged(ZERO_ADJUST)).toBe(false);
  });
});

describe("the brush by dragging and in pixels", () => {
  const start = { radius: 0.05, feather: 0.5, flow: 1 };

  it("brushDrag: sideways moves the ring's edge one to one, up softens", () => {
    // 1000 px long edge on screen: +100 px sideways adds 0.1 of the long edge.
    expect(brushDrag(start, 100, 0, 1000, 0.0001).radius).toBeCloseTo(0.15, 8);
    expect(brushDrag(start, -20, 0, 1000, 0.0001).radius).toBeCloseTo(0.03, 6);
    expect(brushDrag(start, 0, -50, 1000, 0.0001).feather).toBeCloseTo(0.75, 6);
    expect(brushDrag(start, 0, 50, 1000, 0.0001).feather).toBeCloseTo(0.25, 6);
    expect(brushDrag(start, 0, 0, 1000, 0.0001)).toEqual(start);
  });

  it("brushDrag clamps: never under the floor, never over 0.3, feather 0–1, flow kept", () => {
    expect(brushDrag({ ...start, flow: 0.4 }, -5000, 0, 1000, 0.0002)).toEqual({ radius: 0.0002, feather: 0.5, flow: 0.4 });
    expect(brushDrag(start, 5000, 0, 1000, 0.0001).radius).toBe(BRUSH_MAX_RADIUS);
    expect(brushDrag(start, 0, -999, 1000, 0.0001).feather).toBe(1);
    expect(brushDrag(start, 0, 999, 1000, 0.0001).feather).toBe(0);
  });

  it("brushDrag does not divide by a photo of no size", () => {
    expect(brushDrag(start, 10, 0, 0, 0.0001).radius).toBe(0.05);
  });

  it("the size is a diameter in photo pixels, 1 px to 60 % of the long edge", () => {
    expect(brushMinRadius(6000)).toBeCloseTo(0.5 / 6000, 10);
    expect(brushPixels(0.05, 6000)).toBe(600);
    expect(brushRadiusOf(600, 6000)).toBeCloseTo(0.05, 8);
    expect(brushRadiusOf(0.2, 6000)).toBeCloseTo(0.5 / 6000, 10);
    expect(brushRadiusOf(99999, 6000)).toBe(BRUSH_MAX_RADIUS);
  });

  it("the slider's position is logarithmic and the two maps undo each other", () => {
    expect(brushPosition(1, 6000)).toBeCloseTo(0, 8);
    expect(brushPosition(3600, 6000)).toBeCloseTo(1, 8);
    expect(brushSizeAt(0, 6000)).toBeCloseTo(1, 8);
    expect(brushSizeAt(1, 6000)).toBeCloseTo(3600, 6);
    // Halfway along the slider is the geometric mean, not 1800 px.
    expect(brushSizeAt(0.5, 6000)).toBeCloseTo(60, 6);
    for (const px of [1, 7, 24, 250, 3000]) expect(brushSizeAt(brushPosition(px, 6000), 6000)).toBeCloseTo(px, 6);
  });

  it("[ and ] scale by 1.1 and stop at the floor and the ceiling", () => {
    expect(stepBrushRadius(0.05, 1, 6000)).toBeCloseTo(0.055, 8);
    expect(stepBrushRadius(0.055, -1, 6000)).toBeCloseTo(0.05, 8);
    expect(stepBrushRadius(0.5 / 6000, -1, 6000)).toBeCloseTo(0.5 / 6000, 10);
    expect(stepBrushRadius(0.29, 1, 6000)).toBe(BRUSH_MAX_RADIUS);
  });
});
