import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { ContextMenuHost } from "@/components/context-menu";
import { Toaster } from "@/components/toast";
import { ZERO_ADJUST } from "@/lib/develop";
import { DEFAULT_DEVELOP_SOURCE, developSourceStore } from "@/lib/develop-source";
import { maskUi } from "@/lib/develop-masks";
import { DevelopRoute } from "@/routes/develop/gui";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { Mask } from "@/types/generated/Mask";
import type { Recipe } from "@/types/generated/Recipe";

const calls = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);
const lastMasks = () => ((calls("develop_preview").at(-1)?.recipe as Recipe).develop?.masks ?? []) as Mask[];
const steps = () => calls("push_history").map((a) => a.label);
const mask = (id: number): Mask => ({ id, name: `Mask ${id}`, shapes: [{ kind: "radial", add: true, center: [0.5, 0.5], radius: [0.2, 0.2], angle: 0, feather: 0.5, inside: true }], invert: false, opacity: 1, adjust: ZERO_ADJUST, visible: true });

function mount(saved: Recipe | null = null) {
  mockCommand("get_develop_draft", () => saved);
  render(
    <Providers>
      <MemoryRouter initialEntries={["/develop/5"]}>
        <Routes>
          <Route
            path="/develop/:id?"
            element={
              <>
                <DevelopRoute />
                <Toaster />
                <ContextMenuHost />
              </>
            }
          />
        </Routes>
      </MemoryRouter>
    </Providers>,
  );
}

describe("the Masks panel", () => {
  afterEach(() => vi.restoreAllMocks());
  beforeEach(() => {
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1200);
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(800);
    invokeMock.mockClear();
    maskUi.set((u) => ({ ...u, selected: null, painting: null }));
    developSourceStore.set(DEFAULT_DEVELOP_SOURCE);
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("develop_preview", () => ({ token: "1", width: 300, height: 200, superseded: false, histogram: null }));
    mockCommand("develop_release_full", () => null);
    mockCommand("develop_base_info", () => ({ width: 6000, height: 4000, raw: false }));
    mockCommand("strip_page", () => ({ items: [], nextCursor: null }));
    mockCommand("strip_ids", () => [5]);
    mockCommand("list_presets", () => []);
    mockCommand("list_history", () => []);
    mockCommand("push_history", () => 1);
    mockCommand("save_develop_draft", () => 1);
  });

  const panel = async () => screen.findByRole("region", { name: "Masks" });

  it("creates a radial gradient and selects it", async () => {
    mount();
    const p = await panel();
    await userEvent.click(within(p).getByRole("button", { name: "New mask" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Radial gradient" }));
    await waitFor(() => expect(steps()).toContain("New mask"));
    // The new mask is selected: the column is its (mask mode), with the bar's name and Done.
    expect(await screen.findByRole("button", { name: "Done" })).toBeInTheDocument();
    expect(screen.getByText("Mask 1")).toBeInTheDocument();
    await waitFor(() => expect(lastMasks()).toHaveLength(1));
    expect(lastMasks()[0].shapes[0].kind).toBe("radial");
    // The canvas hands itself to the new mask: its handles lie over the photo.
    expect(await screen.findByRole("button", { name: "Radial centre" })).toBeInTheDocument();
  });

  it("edits only the selected mask with its sliders", async () => {
    const saved: Recipe = { version: 3, crop: null, rotate: 0, straighten: 0, flip_h: false, flip_v: false, develop: { ...(await import("@/lib/develop")).DEFAULT_DEVELOP, masks: [mask(1), mask(2)] }, trim: null };
    mount(saved);
    const p = await panel();
    await userEvent.click(await within(p).findByRole("option", { name: /Mask 2/ }));
    // Mask mode: the Basic panel now edits this mask's values, not the photo's.
    const exposure = await screen.findByRole("slider", { name: "Exposure" });
    fireEvent.keyDown(exposure, { key: "ArrowRight" });
    await waitFor(() => expect(lastMasks()[1].adjust.exposure).toBeGreaterThan(0));
    expect(lastMasks()[0].adjust.exposure).toBe(0);
    expect(calls("develop_preview").at(-1)?.recipe).toMatchObject({ develop: { basic: { exposure: 0 } } });
  });

  it("takes over the column for the selected mask, and gives it back on Done", async () => {
    const saved: Recipe = { version: 3, crop: null, rotate: 0, straighten: 0, flip_h: false, flip_v: false, develop: { ...(await import("@/lib/develop")).DEFAULT_DEVELOP, masks: [mask(1)] }, trim: null };
    mount(saved);
    const p = await panel();
    // The photo's own column: the tools bar (Auto, Crop) and the Vignette are there.
    expect(screen.getByRole("toolbar", { name: "Develop tools" })).toBeInTheDocument();
    expect(screen.getByRole("slider", { name: "Vignette" })).toBeInTheDocument();
    await userEvent.click(await within(p).findByRole("option", { name: /Mask 1/ }));
    expect(await screen.findByRole("button", { name: "Done" })).toBeInTheDocument();
    for (const name of ["Basic", "Tone curve", "Colour", "Colour grading", "Detail", "Effects"]) expect(screen.getByRole("region", { name })).toBeInTheDocument();
    expect(screen.queryByRole("toolbar", { name: "Develop tools" })).toBeNull();
    expect(screen.queryByRole("slider", { name: "Vignette" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(await screen.findByRole("toolbar", { name: "Develop tools" })).toBeInTheDocument();
    expect(screen.getByRole("slider", { name: "Vignette" })).toBeInTheDocument();
  });

  it("puts Invert beside Overlay, not on the opacity row", async () => {
    const saved: Recipe = { version: 3, crop: null, rotate: 0, straighten: 0, flip_h: false, flip_v: false, develop: { ...(await import("@/lib/develop")).DEFAULT_DEVELOP, masks: [mask(1)] }, trim: null };
    mount(saved);
    await userEvent.click(await within(await panel()).findByRole("option", { name: /Mask 1/ }));
    const invert = await screen.findByRole("switch", { name: "Invert" });
    const overlay = screen.getByRole("switch", { name: /Overlay/ });
    expect(invert.closest(".develop-masks__switches")).toBe(overlay.closest(".develop-masks__switches"));
    expect(invert.closest(".develop-masks__bar")).toBeNull();
  });

  it("deletes a mask with an undo toast", async () => {
    const saved: Recipe = { version: 3, crop: null, rotate: 0, straighten: 0, flip_h: false, flip_v: false, develop: { ...(await import("@/lib/develop")).DEFAULT_DEVELOP, masks: [mask(1)] }, trim: null };
    mount(saved);
    const p = await panel();
    await userEvent.click(await within(p).findByRole("option", { name: /Mask 1/ }));
    await userEvent.click(await screen.findByRole("button", { name: "Mask options" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Delete" }));
    await waitFor(() => expect(lastMasks()).toHaveLength(0));
    await screen.findByText("Deleted Mask 1");
    await userEvent.click(screen.getAllByRole("button", { name: "Undo" }).at(-1)!);
    await waitFor(() => expect(lastMasks()).toHaveLength(1));
  });

  it("lists the last-applied mask on top; a row's right-click menu reorders (Round 21 §3)", async () => {
    const saved: Recipe = { version: 3, crop: null, rotate: 0, straighten: 0, flip_h: false, flip_v: false, develop: { ...(await import("@/lib/develop")).DEFAULT_DEVELOP, masks: [mask(1), mask(2), mask(3)] }, trim: null };
    mount(saved);
    const p = await panel();
    const names = () => within(p).getAllByRole("option").map((o) => o.textContent);
    await waitFor(() => expect(names()).toEqual(["Mask 3", "Mask 2", "Mask 1"]));
    fireEvent.contextMenu(within(p).getByRole("option", { name: /Mask 3/ }));
    const items = await screen.findAllByRole("menuitem");
    expect(items.map((i) => i.textContent)).toEqual([
      "Rename", "Duplicate", "Invert", "Hide", "Bring to front", "Bring forward", "Send backward", "Send to back", "Reset adjustments", "Delete",
    ]);
    expect(screen.getByRole("menuitem", { name: "Bring to front" })).toBeDisabled();
    expect(within(p).queryByRole("button", { name: "Mask options" })).toBeNull(); // right-click selected nothing
    await userEvent.click(screen.getByRole("menuitem", { name: "Send to back" }));
    await waitFor(() => expect(lastMasks().map((m) => m.id)).toEqual([3, 1, 2]));
    expect(steps().at(-1)).toBe("Send mask to back");
    await waitFor(() => expect(names()).toEqual(["Mask 2", "Mask 1", "Mask 3"]));
  });

  describe("AI masks (8f)", () => {
    const hash = (c: string) => c.repeat(32);
    const found = { available: true, scanned: true, masks: [{ hash: hash("a"), kind: "sky", instance: 0, model: "facet@1", score: 0.98, width: 2, height: 1 }] };
    const raster = () => new Response(new Uint8Array([2, 0, 0, 0, 1, 0, 0, 0, 255, 0]));
    beforeEach(() => {
      vi.spyOn(globalThis, "fetch").mockImplementation(async (url) => (String(url).includes(hash("a")) ? raster() : new Response(null, { status: 404 })));
    });

    it("hides the AI items while Facet is off", async () => {
      mount();
      const p = await panel();
      await userEvent.click(within(p).getByRole("button", { name: "New mask" }));
      await screen.findByRole("menuitem", { name: "Brush" });
      expect(screen.queryByRole("menuitem", { name: "Sky" })).toBeNull();
    });

    it("finds first, then makes a Sky mask from what Facet found", async () => {
      mockCommand("ai_masks_list", () => ({ available: true, scanned: false, masks: [] }));
      mockCommand("ai_masks_detect", () => found);
      mount();
      const p = await panel();
      await userEvent.click(within(p).getByRole("button", { name: "New mask" }));
      await userEvent.click(await screen.findByRole("menuitem", { name: "Sky" }));
      await waitFor(() => expect(steps()).toContain("New sky mask"));
      expect(calls("ai_masks_detect")[0]).toMatchObject({ assetId: 5, again: false });
      await waitFor(() => expect(lastMasks()[0].shapes[0]).toEqual({ kind: "ai", add: true, target: "sky", instance: 0, raster: hash("a") }));
      // Found once: the menu now says what isn't there (back from the mask's column first).
      await userEvent.click(await screen.findByRole("button", { name: "Done" }));
      await userEvent.click(within(await panel()).getByRole("button", { name: "New mask" }));
      expect(await screen.findByRole("menuitem", { name: "No subject found" })).toBeDisabled();
    });

    it("opens People as a submenu that shares the context menus' dropdown look", async () => {
      const people = { available: true, scanned: true, masks: [0, 1].map((instance) => ({ hash: hash("a"), kind: "person", instance, model: "facet@1", score: 0.9, width: 2, height: 1 })) };
      mockCommand("ai_masks_list", () => people);
      mount();
      const p = await panel();
      await userEvent.click(within(p).getByRole("button", { name: "New mask" }));
      await userEvent.hover(await screen.findByRole("menuitem", { name: "People" }));
      const sub = (await screen.findByRole("menuitem", { name: /Person 1/ })).closest('[role="menu"]');
      expect(sub).toHaveClass("context-menu__dropdown");
    });

    it("says when a pasted mask isn't on this photo, and Detect again points it here", async () => {
      mockCommand("ai_masks_list", () => found);
      const pasted: Mask = { ...mask(1), name: "Sky", shapes: [{ kind: "ai", add: true, target: "sky", instance: 0, raster: hash("b") }] };
      mount({ version: 3, crop: null, rotate: 0, straighten: 0, flip_h: false, flip_v: false, develop: { ...(await import("@/lib/develop")).DEFAULT_DEVELOP, masks: [pasted] }, trim: null });
      const p = await panel();
      await userEvent.click(await within(p).findByRole("option", { name: /Sky/ }));
      expect(await screen.findByText(/Not detected on this photo/)).toBeInTheDocument();
      await userEvent.click(screen.getByRole("button", { name: "Detect again" }));
      await waitFor(() => expect(lastMasks()[0].shapes[0]).toMatchObject({ raster: hash("a") }));
      // What Facet had already found answered; it wasn't run again.
      expect(calls("ai_masks_detect")).toHaveLength(0);
      await waitFor(() => expect(screen.queryByText(/Not detected on this photo/)).toBeNull());
    });
  });
});
