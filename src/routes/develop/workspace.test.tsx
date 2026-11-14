import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { ContextMenuHost } from "@/components/context-menu";
import { exportSheetStore } from "@/components/export-sheet";
import { Toaster } from "@/components/toast";
import { toastStore } from "@/components/toast/store";
import { nextSelection } from "@/components/filmstrip";
import { requestAuto } from "@/components/develop-panels";
import { DEFAULT_DEVELOP, ZERO_ADJUST, developSelection } from "@/lib/develop";
import type { Mask } from "@/types/generated/Mask";
import type { Recipe } from "@/types/generated/Recipe";
import { DEFAULT_DEVELOP_SOURCE, developSourceStore, stripHiddenStore } from "@/lib/develop-source";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { DevelopRoute } from "./gui";

const item = (id: number, flag = 0, colorLabel = 0) => ({ id, filename: `IMG_${id}.jpg`, flag, colorLabel, rating: 0, edited: false, width: 300, height: 200 });
const ITEMS = [item(1, 1), item(2), item(3, -1, 3), item(4)];

function Where() {
  return <p data-testid="where">{useLocation().pathname}</p>;
}

function mount(id = 2) {
  render(
    <Providers>
      <MemoryRouter initialEntries={[`/develop/${id}`]}>
        <Routes>
          <Route
            path="/develop/:id?"
            element={
              <>
                <Where />
                <DevelopRoute />
                <ContextMenuHost />
                <Toaster />
              </>
            }
          />
        </Routes>
      </MemoryRouter>
    </Providers>,
  );
}

const calls = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);

describe("the filmstrip's selection", () => {
  it("replaces on a click, toggles on Ctrl, runs on Shift", () => {
    const ids = [1, 2, 3, 4];
    expect(nextSelection(ids, new Set([1]), 1, 3, { ctrl: false, shift: false })).toEqual({ ids: new Set(), anchor: 3 });
    expect(nextSelection(ids, new Set([1]), 1, 3, { ctrl: true, shift: false })).toEqual({ ids: new Set([1, 3]), anchor: 3 });
    expect(nextSelection(ids, new Set([1, 3]), 3, 3, { ctrl: true, shift: false }).ids).toEqual(new Set([1]));
    expect(nextSelection(ids, new Set(), 2, 4, { ctrl: false, shift: true }).ids).toEqual(new Set([2, 3, 4]));
  });
});

describe("the Develop workspace", () => {
  afterEach(() => vi.restoreAllMocks());
  beforeEach(() => {
    // jsdom lays nothing out; the filmstrip's virtualiser needs a width to show thumbnails.
    vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(1600);
    vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(104);
    invokeMock.mockClear();
    developSourceStore.set({ ...DEFAULT_DEVELOP_SOURCE, only: null, label: "Library" });
    developSelection.set({ ids: new Set(), anchor: null });
    exportSheetStore.set({ ids: null });
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("develop_preview", (a) => ({ token: String(a?.assetId), width: 300, height: 200, superseded: false, histogram: null }));
    mockCommand("strip_page", () => ({ items: ITEMS, nextCursor: null }));
    mockCommand("strip_ids", (a) => {
      const q = (a?.scope as { query?: { flag?: number | null; colorLabels?: number[] } | null }).query;
      return ITEMS.filter((i) => (q?.flag == null || i.flag === q.flag) && (!q?.colorLabels?.length || q.colorLabels.includes(i.colorLabel))).map((i) => i.id);
    });
    mockCommand("list_presets", () => []);
    mockCommand("list_history", () => []);
    mockCommand("set_flag", (a) => (a?.ids as number[]).map((id) => ({ id, previous: 0 })));
    mockCommand("set_color_label", (a) => (a?.ids as number[]).map((id) => ({ id, previous: 0 })));
  });

  it("shows the strip with its marks, and opens a photo from it", async () => {
    mount();
    const strip = await screen.findByRole("listbox", { name: "Filmstrip" });
    expect(await within(strip).findByRole("option", { name: "IMG_2.jpg" })).toHaveAttribute("aria-current", "true");
    expect(within(strip).getByLabelText("Picked")).toBeInTheDocument();
    expect(within(strip).getByLabelText("Green label")).toBeInTheDocument();
    await userEvent.click(within(strip).getByRole("option", { name: "IMG_4.jpg" }));
    await waitFor(() => expect(screen.getByTestId("where")).toHaveTextContent("/develop/4"));
  });

  it("says when a RAW shows as the camera rendered it, and only then", async () => {
    mockCommand("develop_preview", (a) => ({ token: String(a?.assetId), width: 300, height: 200, superseded: false, histogram: null, cameraRendering: a?.assetId === 2 }));
    mount(2);
    const note = await screen.findByRole("note");
    expect(note).toHaveTextContent("Camera rendering, not RAW");
    // Round 17: it says why, in Settings, and names no camera maker.
    expect(within(note).getByRole("link", { name: "Why?" })).toHaveAttribute("href", "/settings?section=raw-formats");
    expect(note.getAttribute("title")).not.toMatch(/Nikon|HE /);
    await userEvent.click(within(await screen.findByRole("listbox", { name: "Filmstrip" })).getByRole("option", { name: "IMG_4.jpg" }));
    await waitFor(() => expect(screen.getByTestId("where")).toHaveTextContent("/develop/4"));
    await waitFor(() => expect(screen.queryByRole("note")).toBeNull());
  });

  it("selects the picked through Rust, and counts the selection", async () => {
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "Select" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Select picked" }));
    await waitFor(() => expect([...developSelection.get().ids]).toEqual([1]));
    expect(calls("strip_ids").some((a) => (a.scope as { query?: { flag?: number } }).query?.flag === 1)).toBe(true);
    expect(await screen.findByText("1 of 4 photos")).toBeInTheDocument();
    // Two menus and a round trip: under a full parallel run this passes 5 s (6.3 s seen).
  }, 15_000);

  // Round 4J: on a narrow window the mark filter, rating and order fold into one menu.
  it("offers the filters and the order in one Filter menu", async () => {
    mount();
    await screen.findByRole("listbox", { name: "Filmstrip" });
    await userEvent.click(screen.getByRole("button", { name: "Filter" }));
    const menu = await screen.findByRole("menu");
    await userEvent.click(within(menu).getByRole("button", { name: "Show rejected" }));
    await waitFor(() => expect(calls("strip_page").some((a) => (a.scope as { query?: { flag?: number } }).query?.flag === -1)).toBe(true));
    await userEvent.click(within(menu).getByRole("menuitem", { name: "File name" }));
    await waitFor(() => expect(calls("strip_page").some((a) => a.sort === "filename")).toBe(true));
    expect(screen.getByRole("button", { name: "Filter" })).toHaveAttribute("aria-pressed", "true");
  }, 15_000);

  it("narrows the strip with the mark filter and sorts it", async () => {
    mount();
    await screen.findByRole("listbox", { name: "Filmstrip" });
    await userEvent.click(screen.getByRole("button", { name: "Show picked" }));
    await waitFor(() => expect(calls("strip_page").some((a) => (a.scope as { query?: { flag?: number } }).query?.flag === 1)).toBe(true));
    await userEvent.click(screen.getByRole("button", { name: "Sort by Capture time" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Rating" }));
    await waitFor(() => expect(calls("strip_page").some((a) => a.sort === "rating")).toBe(true));
  });

  it("culls with P and Alt+3, on the selection or the open photo", async () => {
    mount();
    await screen.findByRole("listbox", { name: "Filmstrip" });
    fireEvent.keyDown(window, { key: "p" });
    await waitFor(() => expect(calls("set_flag")[0]).toEqual({ ids: [2], flag: 1 }));
    developSelection.set({ ids: new Set([3, 4]), anchor: 3 });
    fireEvent.keyDown(window, { key: "3", code: "Digit3", altKey: true });
    await waitFor(() => expect(calls("set_color_label")[0]).toEqual({ ids: [3, 4], label: 3 }));
  });

  it("moves through the strip with the arrows", async () => {
    mount();
    await screen.findByRole("option", { name: "IMG_3.jpg" });
    fireEvent.keyDown(window, { key: "ArrowRight" });
    await waitFor(() => expect(screen.getByTestId("where")).toHaveTextContent("/develop/3"));
  });

  it("folds the filmstrip away and back, with F6 too", async () => {
    stripHiddenStore.set(false);
    mount();
    await screen.findByRole("option", { name: "IMG_3.jpg" });
    const root = document.querySelector(".develop")!;
    await userEvent.click(screen.getByRole("button", { name: "Hide filmstrip" }));
    expect(root).toHaveAttribute("data-strip", "hidden");
    expect(screen.queryByRole("option", { name: "IMG_3.jpg" }), "out of reach while folded").toBeNull();
    fireEvent.keyDown(window, { key: "F6" });
    expect(root).toHaveAttribute("data-strip", "shown");
    expect(screen.getByRole("button", { name: "Hide filmstrip" })).toBeInTheDocument();
  });

  it("runs Auto on Ctrl+U and from the palette's request", async () => {
    mockCommand("develop_auto", () => ({ basic: { ...DEFAULT_DEVELOP.basic, exposure: 0.5 }, usedModel: true, scene: null }));
    mockCommand("push_history", () => 1);
    mockCommand("truncate_history", () => 0);
    mockCommand("save_develop_draft", () => 1);
    mount();
    await screen.findByRole("option", { name: "IMG_3.jpg" });
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("develop_preview", expect.anything()));
    fireEvent.keyDown(window, { key: "u", ctrlKey: true });
    await waitFor(() => expect(invokeMock.mock.calls.filter(([c]) => c === "develop_auto")).toHaveLength(1));
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.message).toBe("Auto"));
    act(() => requestAuto());
    await waitFor(() => expect(invokeMock.mock.calls.filter(([c]) => c === "develop_auto")).toHaveLength(2));
  });

  it("exports exactly the selection", async () => {
    developSelection.set({ ids: new Set([1, 3]), anchor: 1 });
    mount();
    await userEvent.click(await screen.findByRole("button", { name: /Export 2/ }));
    await waitFor(() => expect(exportSheetStore.get().ids).toEqual([1, 3]));
  });

  it("clears the filmstrip back to choosing photos, and Undo brings it back", async () => {
    toastStore.set({ toasts: [] });
    developSourceStore.set({ ...DEFAULT_DEVELOP_SOURCE, only: [1, 2, 3, 4], label: "Selection" });
    mount();
    await screen.findByRole("listbox", { name: "Filmstrip" });
    await userEvent.click(await screen.findByRole("button", { name: "Select" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Clear filmstrip" }));
    await waitFor(() => expect(screen.getByTestId("where")).toHaveTextContent(/^\/develop$/));
    expect(await screen.findByText("Choose photos to develop")).toBeInTheDocument();
    await userEvent.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() => expect(screen.getByTestId("where")).toHaveTextContent("/develop/2"));
    expect(developSourceStore.get().only).toEqual([1, 2, 3, 4]);
  }, 15_000);

  it("removes the open photo from the strip and opens the next one", async () => {
    toastStore.set({ toasts: [] });
    developSourceStore.set({ ...DEFAULT_DEVELOP_SOURCE, only: [1, 2, 3, 4], label: "Selection" });
    mount();
    const strip = await screen.findByRole("listbox", { name: "Filmstrip" });
    fireEvent.contextMenu(await within(strip).findByRole("option", { name: "IMG_2.jpg" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Remove from filmstrip" }));
    await waitFor(() => expect(developSourceStore.get().only).toEqual([1, 3, 4]));
    await waitFor(() => expect(screen.getByTestId("where")).toHaveTextContent("/develop/3"));
    expect(await screen.findByText("Removed from the filmstrip")).toBeInTheDocument();
  });

  // Round 22 A1: a hovered preset looks exactly like the applied one.
  describe("a preset", () => {
    const bw = { id: -4, key: "builtin:4", source: "builtin", name: "B&W", develop: { ...DEFAULT_DEVELOP, basic: { ...DEFAULT_DEVELOP.basic, saturation: -1, contrast: 0.2, clarity: 0.15 } }, panels: ["basic"], builtin: true };
    const blurred: Mask = {
      id: 1,
      name: "Background",
      shapes: [{ kind: "radial", add: true, center: [0.5, 0.5], radius: [0.3, 0.3], angle: 0, feather: 0, inside: true }],
      invert: true,
      opacity: 1,
      adjust: { ...ZERO_ADJUST, motion_blur: { amount: 0.5, angle: 0 } },
      visible: true,
    };
    const saved: Recipe = { version: 3, crop: null, rotate: 0, straighten: 0, flip_h: false, flip_v: false, develop: { ...DEFAULT_DEVELOP, masks: [blurred] }, trim: null };
    const lastRecipe = () => calls("develop_preview").at(-1)?.recipe as Recipe;

    beforeEach(() => {
      mockCommand("get_develop_draft", () => saved);
      mockCommand("list_presets", () => [bw]);
      mockCommand("develop_release_full", () => null);
      mockCommand("develop_base_info", () => ({ width: 6000, height: 4000, raw: false }));
      mockCommand("push_history", () => 1);
      mockCommand("save_develop_draft", () => 1);
      mockCommand("truncate_history", () => 0);
    });

    it("hovering sends the whole recipe, and leaving sends the draft again", async () => {
      mount();
      const row = await screen.findByRole("button", { name: "B&W" });
      await waitFor(() => expect(lastRecipe()?.develop?.masks).toHaveLength(1));
      await userEvent.hover(row);
      await waitFor(() => expect(lastRecipe().develop?.basic.saturation).toBe(-1));
      expect(lastRecipe().develop?.masks).toEqual([blurred]);
      await userEvent.unhover(row);
      await waitFor(() => expect(lastRecipe()).toEqual(saved));
    });

    it("a click commits the very recipe the hover showed", async () => {
      mount();
      const row = await screen.findByRole("button", { name: "B&W" });
      await waitFor(() => expect(lastRecipe()?.develop?.masks).toHaveLength(1));
      await userEvent.hover(row);
      await waitFor(() => expect(lastRecipe().develop?.basic.saturation).toBe(-1));
      const hovered = lastRecipe();
      await userEvent.click(row);
      await waitFor(() => expect(calls("push_history").map((a) => a.label)).toContain("Preset: B&W"));
      await waitFor(() => expect(calls("save_develop_draft").at(-1)?.recipe).toEqual(hovered));
    });
  });
});
