import { fireEvent, render, screen } from "@testing-library/react";
import type { ReactElement } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { DEFAULT_DEVELOP, ZERO_ADJUST } from "@/lib/develop";
import { maskUi } from "@/lib/develop-masks";
import { frameMap, toFrame } from "@/lib/mask-raster";
import type { Develop } from "@/types/generated/Develop";
import type { Mask } from "@/types/generated/Mask";
import type { Recipe } from "@/types/generated/Recipe";
import { MaskOverlay } from "./mask-overlay";

/** A session that records what the overlay asks of it. */
function fakeSession(recipe: Recipe) {
  const state = { draft: recipe, commits: [] as string[], updates: 0 };
  const session = {
    get draft() {
      return state.draft;
    },
    loaded: true,
    update: (change: (d: Develop) => Develop) => {
      state.updates += 1;
      state.draft = { ...state.draft, develop: change(state.draft.develop ?? DEFAULT_DEVELOP) };
    },
    commit: (label: string) => state.commits.push(label),
  } as unknown as DevelopSession;
  return { session, state };
}

const radial: Mask = {
  id: 1,
  name: "Mask 1",
  shapes: [{ kind: "radial", add: true, center: [0.5, 0.5], radius: [0.2, 0.1], angle: 0, feather: 0.5, inside: true }],
  invert: false,
  opacity: 1,
  adjust: ZERO_ADJUST,
  visible: true,
};
const withMask = (m: Mask, rotate = 0): Recipe => ({ version: 3, crop: null, rotate, straighten: 0, flip_h: false, flip_v: false, develop: { ...DEFAULT_DEVELOP, masks: [m] }, trim: null });

describe("MaskOverlay", () => {
  afterEach(() => vi.restoreAllMocks());
  beforeEach(() => {
    // Turned a quarter, the 400 × 300 photo shows as 300 × 400, at one screen pixel each.
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ x: 0, y: 0, left: 0, top: 0, width: 300, height: 400, right: 300, bottom: 400, toJSON: () => ({}) });
    maskUi.set((u) => ({ ...u, selected: 1, painting: null, overlay: false, active: false, brush: { radius: 0.05, feather: 0.5, flow: 1 } }));
  });

  it("maps a size handle's drag back to the unedited photo on a turned photo", () => {
    const { session, state } = fakeSession(withMask(radial, 90));
    render(<MaskOverlay session={session} base={{ w: 400, h: 300 }} box={{ width: 300, height: 400 }} />);
    const handle = screen.getAllByRole("button", { name: "Radial width" })[0];
    const target = toFrame(frameMap(400, 300, { rotate: 90, flip_h: false, flip_v: false, straighten: 0, crop: null }), 0.8, 0.5);
    fireEvent.pointerDown(handle, { clientX: 0, clientY: 0, pointerId: 1 });
    fireEvent.pointerMove(handle, { clientX: target[0], clientY: target[1], pointerId: 1 });
    fireEvent.pointerUp(handle, { clientX: target[0], clientY: target[1], pointerId: 1 });
    const shape = state.draft.develop!.masks![0].shapes[0];
    expect(shape.kind === "radial" && shape.radius[0]).toBeCloseTo(0.3, 3);
    expect(state.commits).toEqual(["Radial gradient"]);
  });

  it("paints a stroke as quantised points and records it once", () => {
    const { session, state } = fakeSession(withMask({ ...radial, shapes: [] }));
    maskUi.set((u) => ({ ...u, painting: { add: true } }));
    render(<MaskOverlay session={session} base={{ w: 300, h: 400 }} box={{ width: 300, height: 400 }} />);
    const layer = screen.getByLabelText("Paint the mask");
    fireEvent.pointerDown(layer, { clientX: 30, clientY: 40, pointerId: 1 });
    for (let i = 1; i <= 60; i++) fireEvent.pointerMove(layer, { clientX: 30 + i * 3, clientY: 40 + i * 2, pointerId: 1 });
    fireEvent.pointerUp(layer, { clientX: 210, clientY: 160, pointerId: 1 });
    const shapes = state.draft.develop!.masks![0].shapes;
    expect(shapes).toHaveLength(1);
    const s = shapes[0];
    expect(s.kind).toBe("brush");
    if (s.kind !== "brush") return;
    expect(s.add).toBe(true);
    expect(s.points[0]).toEqual([410, 410]);
    expect(s.points.every(([x, y]) => Number.isInteger(x) && Number.isInteger(y) && x >= 0 && x <= 4096 && y <= 4096)).toBe(true);
    expect(state.commits).toEqual(["Brush"]);
    // The overlay paints every move itself; the session (and the preview) hears of the stroke
    // a few times a second, not sixty.
    expect(state.updates).toBeLessThanOrEqual(4);
  });

  // A real session re-renders all of Develop on each update (60–80 ms in dev). Sending whenever
  // 100 ms had passed since the last send then sent on every move, the moves queued behind the
  // renders, and a long stroke froze the page until the watchdog reloaded it.
  it("sends a stroke on a timer, not on every move of a slow page", () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "requestAnimationFrame", "cancelAnimationFrame"] });
    let now = 0;
    vi.spyOn(performance, "now").mockImplementation(() => (now += 150));
    const { session, state } = fakeSession(withMask({ ...radial, shapes: [] }));
    maskUi.set((u) => ({ ...u, painting: { add: false } }));
    render(<MaskOverlay session={session} base={{ w: 300, h: 400 }} box={{ width: 300, height: 400 }} />);
    const layer = screen.getByLabelText("Paint the mask");
    fireEvent.pointerDown(layer, { clientX: 30, clientY: 40, pointerId: 1 });
    for (let i = 1; i <= 60; i++) fireEvent.pointerMove(layer, { clientX: 30 + i * 3, clientY: 40 + i * 2, pointerId: 1 });
    expect(state.updates).toBe(1);
    // A frame for the first send to be drawn, then 100 ms.
    vi.advanceTimersByTime(100);
    expect(state.updates).toBe(1);
    vi.advanceTimersByTime(50);
    expect(state.updates).toBe(2);
    fireEvent.pointerUp(layer, { clientX: 210, clientY: 160, pointerId: 1 });
    vi.advanceTimersByTime(1000);
    expect(state.updates).toBe(3);
    const s = state.draft.develop!.masks![0].shapes[0];
    expect(s.kind === "brush" && !s.add && s.points.length > 10).toBe(true);
    expect(state.commits).toEqual(["Brush"]);
    vi.useRealTimers();
  });

  // Round 5: a drag "yeeted" the mask away — every move added the whole distance since
  // pointer-down to a shape that had already moved. The real session re-renders after each
  // `update`, so these drags re-render between moves.
  describe("a drag over several moves lands under the pointer", () => {
    const unturned = { x: 0, y: 0, left: 0, top: 0, width: 400, height: 300, right: 400, bottom: 300, toJSON: () => ({}) };
    const dragInSteps = (name: string, from: [number, number], steps: [number, number][], view: () => ReactElement, rerender: (ui: ReactElement) => void) => {
      const at = () => screen.getByRole("button", { name });
      fireEvent.pointerDown(at(), { clientX: from[0], clientY: from[1], pointerId: 1 });
      for (const [x, y] of steps) {
        fireEvent.pointerMove(at(), { clientX: x, clientY: y, pointerId: 1 });
        rerender(view());
      }
      const [x, y] = steps[steps.length - 1];
      fireEvent.pointerUp(at(), { clientX: x, clientY: y, pointerId: 1 });
    };

    it("moves a radial by exactly the pointer's travel", () => {
      vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(unturned);
      const { session, state } = fakeSession(withMask(radial));
      const view = () => <MaskOverlay session={session} base={{ w: 400, h: 300 }} box={{ width: 400, height: 300 }} />;
      const { rerender } = render(view());
      dragInSteps("Radial centre", [200, 150], [[230, 150], [260, 150], [260, 180]], view, rerender);
      const s = state.draft.develop!.masks![0].shapes[0];
      if (s.kind !== "radial") throw new Error("radial expected");
      expect(s.center[0]).toBeCloseTo(0.5 + 60 / 400, 4);
      expect(s.center[1]).toBeCloseTo(0.5 + 30 / 300, 4);
      expect(state.commits).toEqual(["Radial gradient"]);
    });

    it("moves a linear gradient by exactly the pointer's travel", () => {
      vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(unturned);
      const linear: Mask = { ...radial, shapes: [{ kind: "linear", add: true, from: [0.5, 0.2], to: [0.5, 0.6] }] };
      const { session, state } = fakeSession(withMask(linear));
      const view = () => <MaskOverlay session={session} base={{ w: 400, h: 300 }} box={{ width: 400, height: 300 }} />;
      const { rerender } = render(view());
      dragInSteps("Linear centre", [200, 120], [[210, 120], [220, 120], [240, 120]], view, rerender);
      const s = state.draft.develop!.masks![0].shapes[0];
      if (s.kind !== "linear") throw new Error("linear expected");
      expect(s.from[0]).toBeCloseTo(0.5 + 40 / 400, 4);
      expect(s.to[0]).toBeCloseTo(0.5 + 40 / 400, 4);
    });

    it("moves a radial on a turned photo by exactly the pointer's travel", () => {
      const { session, state } = fakeSession(withMask(radial, 90));
      const view = () => <MaskOverlay session={session} base={{ w: 400, h: 300 }} box={{ width: 300, height: 400 }} />;
      const { rerender } = render(view());
      const map = frameMap(400, 300, { rotate: 90, flip_h: false, flip_v: false, straighten: 0, crop: null });
      const c = toFrame(map, 0.5, 0.5);
      const end = toFrame(map, 0.6, 0.55);
      const mid: [number, number] = [(c[0] + end[0]) / 2, (c[1] + end[1]) / 2];
      dragInSteps("Radial centre", c, [mid, end], view, rerender);
      const s = state.draft.develop!.masks![0].shapes[0];
      if (s.kind !== "radial") throw new Error("radial expected");
      expect(s.center[0]).toBeCloseTo(0.6, 4);
      expect(s.center[1]).toBeCloseTo(0.55, 4);
    });
  });

  describe("the tint", () => {
    const tint = (c: HTMLElement) => c.querySelector(".mask-overlay__tint");

    it("stays while a handle is held and moved, and comes on when it was off (Round 8B)", () => {
      maskUi.set((u) => ({ ...u, overlay: false, active: false, painting: null }));
      const { session } = fakeSession(withMask(radial, 90));
      const view = () => <MaskOverlay session={session} base={{ w: 400, h: 300 }} box={{ width: 300, height: 400 }} />;
      const { container, rerender } = render(view());
      expect(tint(container)).toBeNull();
      const handle = screen.getByRole("button", { name: "Radial centre" });
      fireEvent.pointerDown(handle, { clientX: 150, clientY: 200, pointerId: 1 });
      fireEvent.pointerMove(handle, { clientX: 170, clientY: 210, pointerId: 1 });
      rerender(view());
      expect(tint(container), "moving the mask shows where it is").not.toBeNull();
      fireEvent.pointerUp(handle, { clientX: 170, clientY: 210, pointerId: 1 });
      window.dispatchEvent(new Event("pointerup"));
      rerender(view());
      expect(tint(container)).not.toBeNull();
    });

    it("stays hidden when the user turned it off, even while painting", () => {
      maskUi.set((u) => ({ ...u, overlay: false, active: false, painting: { add: true } }));
      const { session } = fakeSession(withMask(radial));
      const { container } = render(<MaskOverlay session={session} base={{ w: 300, h: 400 }} box={{ width: 300, height: 400 }} />);
      expect(tint(container)).toBeNull();
    });

    it("hides while a slider elsewhere is dragged", () => {
      maskUi.set((u) => ({ ...u, overlay: true, active: true }));
      const { session } = fakeSession(withMask(radial));
      const { container } = render(<MaskOverlay session={session} base={{ w: 300, h: 400 }} box={{ width: 300, height: 400 }} />);
      expect(tint(container)).toBeNull();
      maskUi.set((u) => ({ ...u, active: false }));
    });
  });

  it("erases with Alt held", () => {
    const { session, state } = fakeSession(withMask({ ...radial, shapes: [] }));
    maskUi.set((u) => ({ ...u, painting: { add: true } }));
    render(<MaskOverlay session={session} base={{ w: 300, h: 400 }} box={{ width: 300, height: 400 }} />);
    const layer = screen.getByLabelText("Paint the mask");
    fireEvent.pointerDown(layer, { clientX: 30, clientY: 40, pointerId: 1, altKey: true });
    fireEvent.pointerUp(layer, { clientX: 30, clientY: 40, pointerId: 1, altKey: true });
    const s = state.draft.develop!.masks![0].shapes[0];
    expect(s.add).toBe(false);
  });
});
