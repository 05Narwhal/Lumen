import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { useDevelopSession } from "@/hooks/use-develop-session";
import { DEFAULT_DEVELOP, ZERO_ADJUST } from "@/lib/develop";
import { maskUi } from "@/lib/develop-masks";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { DevelopPreview } from "@/types/generated/DevelopPreview";
import type { Recipe } from "@/types/generated/Recipe";
import { MaskOverlay } from "./mask-overlay";

const calls = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);
const rendered = (token: string): DevelopPreview => ({ token, width: 300, height: 400, superseded: false, histogram: null, cameraRendering: false });

const saved: Recipe = {
  version: 3,
  crop: null,
  rotate: 0,
  straighten: 0,
  flip_h: false,
  flip_v: false,
  develop: { ...DEFAULT_DEVELOP, masks: [{ id: 1, name: "Mask 1", shapes: [], invert: false, opacity: 1, adjust: { ...ZERO_ADJUST, exposure: 1 }, visible: true }] },
  trim: null,
};

/** The real session with the real overlay over a 300 × 400 photo shown at one pixel each. */
function Harness() {
  const session = useDevelopSession(7);
  if (!session.loaded) return null;
  return <MaskOverlay session={session} base={{ w: 300, h: 400 }} box={{ width: 300, height: 400 }} />;
}

describe("painting a mask", () => {
  afterEach(() => vi.restoreAllMocks());
  beforeEach(() => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ x: 0, y: 0, left: 0, top: 0, width: 300, height: 400, right: 300, bottom: 400, toJSON: () => ({}) });
    invokeMock.mockClear();
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_develop_draft", () => saved);
    mockCommand("push_history", () => 1);
    mockCommand("truncate_history", () => 0);
    mockCommand("save_develop_draft", () => 1);
    maskUi.set((u) => ({ ...u, selected: 1, painting: { add: true }, brush: { radius: 0.05, feather: 0.5, flow: 1 } }));
  });

  it("a_stroke_is_one_history_step, with one render in flight and the newest waiting", async () => {
    const replies: Array<(p: DevelopPreview) => void> = [];
    mockCommand("develop_preview", () => new Promise<DevelopPreview>((r) => replies.push(r)));
    render(
      <Providers>
        <Harness />
      </Providers>,
    );
    const layer = await screen.findByLabelText("Paint the mask");
    await waitFor(() => expect(replies).toHaveLength(1));
    fireEvent.pointerDown(layer, { clientX: 20, clientY: 20, pointerId: 1 });
    for (let i = 1; i <= 60; i++) fireEvent.pointerMove(layer, { clientX: 20 + i * 4, clientY: 20 + i * 5, pointerId: 1 });
    fireEvent.pointerUp(layer, { clientX: 260, clientY: 320, pointerId: 1 });
    // Sixty moves, and still only the first render is out.
    expect(replies).toHaveLength(1);
    await waitFor(() => expect(calls("push_history").map((a) => a.label)).toEqual(["Brush"]));
    await act(async () => replies[0](rendered("a")));
    await waitFor(() => expect(replies).toHaveLength(2));
    // The one sent next is the whole stroke.
    const sent = calls("develop_preview")[1].recipe as Recipe;
    const shape = sent.develop!.masks![0].shapes[0];
    expect(shape.kind === "brush" && shape.points.length).toBeGreaterThan(20);
    expect(sent.develop!.masks![0].shapes).toHaveLength(1);
  });

  it("Alt + right-drag sizes and feathers the brush, paints nothing, and shows no context menu", async () => {
    mockCommand("develop_preview", () => Promise.resolve(rendered("a")));
    render(
      <Providers>
        <Harness />
      </Providers>,
    );
    const layer = await screen.findByLabelText("Paint the mask");
    await waitFor(() => expect(calls("develop_preview")).toHaveLength(1));
    const before = calls("develop_preview").length;
    // A plain right-click is not the gesture; the browser's menu is still kept off the photo.
    expect(fireEvent.contextMenu(layer, { clientX: 50, clientY: 50 })).toBe(false);
    fireEvent.pointerDown(layer, { clientX: 100, clientY: 100, pointerId: 1, button: 2, altKey: true });
    // The photo is 400 px on its long edge at one pixel each: 60 px sideways is 0.15 more radius.
    fireEvent.pointerMove(layer, { clientX: 160, clientY: 80, pointerId: 1, buttons: 2, altKey: true });
    expect(maskUi.get().brush.radius).toBeCloseTo(0.2, 6);
    expect(maskUi.get().brush.feather).toBeCloseTo(0.6, 6);
    // The ring stays where the press began and says what it now is: 2 · 0.2 · 400 px.
    expect(screen.getByText("160 px · feather 60")).toBeTruthy();
    expect(layer.querySelector("circle")?.getAttribute("cx")).toBe("100");
    fireEvent.pointerUp(layer, { clientX: 160, clientY: 80, pointerId: 1, button: 2 });
    expect(screen.queryByText("160 px · feather 60")).toBeNull();
    // Nothing was painted: no stroke, no history step, no new render.
    expect(calls("push_history")).toHaveLength(0);
    expect(calls("develop_preview")).toHaveLength(before);
  });

  it("a right-drag without Alt does nothing to the brush", async () => {
    render(
      <Providers>
        <Harness />
      </Providers>,
    );
    const layer = await screen.findByLabelText("Paint the mask");
    fireEvent.pointerDown(layer, { clientX: 100, clientY: 100, pointerId: 1, button: 2 });
    fireEvent.pointerMove(layer, { clientX: 160, clientY: 80, pointerId: 1, buttons: 2 });
    expect(maskUi.get().brush).toEqual({ radius: 0.05, feather: 0.5, flow: 1 });
    expect(calls("push_history")).toHaveLength(0);
  });
});
