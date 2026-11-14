import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { DEFAULT_DEVELOP_SOURCE, developSourceStore } from "@/lib/develop-source";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { Recipe } from "@/types/generated/Recipe";
import { DevelopRoute } from "./gui";

const calls = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);
const lastPreview = () => calls("develop_preview").at(-1)?.recipe as Recipe;
const steps = () => calls("push_history").map((a) => a.label);

function mount() {
  render(
    <Providers>
      <MemoryRouter initialEntries={["/develop/5"]}>
        <Routes>
          <Route path="/develop/:id?" element={<DevelopRoute />} />
        </Routes>
      </MemoryRouter>
    </Providers>,
  );
}

describe("Crop in Develop", () => {
  afterEach(() => vi.restoreAllMocks());
  beforeEach(() => {
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1200);
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(800);
    // The crop overlay turns pointer pixels into frame units with its own box.
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ x: 0, y: 0, left: 0, top: 0, width: 600, height: 400, right: 600, bottom: 400, toJSON: () => ({}) });
    invokeMock.mockClear();
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

  const open = async () => {
    mount();
    await screen.findByRole("img", { name: "The photo as developed" });
    fireEvent.keyDown(window, { key: "r" });
    return screen.findByRole("region", { name: "Crop" });
  };

  it("shows the photo at once when a photo with no geometry opens the crop tool", async () => {
    await open();
    // An identity recipe's one preview already is the uncropped photo: no second render.
    expect(await screen.findByRole("img", { name: "The photo being cropped" })).toBeInTheDocument();
    expect(screen.queryByText("Rendering the whole photo")).toBeNull();
    expect(calls("develop_preview")).toHaveLength(1);
  });

  it("still renders the whole photo again when the saved recipe has a crop", async () => {
    const cropped: Recipe = { version: 3, crop: { x: 0.1, y: 0.1, w: 0.5, h: 0.5 }, rotate: 0, straighten: 0, flip_h: false, flip_v: false, develop: null, trim: null };
    mockCommand("get_develop_draft", () => cropped);
    mount();
    await screen.findByRole("img", { name: "The photo as developed" });
    expect(calls("develop_preview")).toHaveLength(1);
    expect(lastPreview().crop).not.toBeNull();
    fireEvent.keyDown(window, { key: "r" });
    // A cropped recipe's preview is not the whole photo: the geometry-free one is asked for.
    await waitFor(() => expect(calls("develop_preview")).toHaveLength(2));
    expect(lastPreview().crop).toBeNull();
    expect(await screen.findByRole("img", { name: "The photo being cropped" })).toBeInTheDocument();
  });

  it("keeps Auto and Crop side by side in one toolbar, and Reset crop inside the open tool", async () => {
    mount();
    await screen.findByRole("img", { name: "The photo as developed" });
    const bar = screen.getByRole("toolbar", { name: "Develop tools" });
    const crop = within(bar).getByRole("button", { name: /^Crop/ });
    expect(within(bar).getByRole("button", { name: "Auto" })).toBeInTheDocument();
    expect(crop).toHaveAttribute("aria-pressed", "false");
    await userEvent.click(crop);
    const panel = await screen.findByRole("region", { name: "Crop" });
    expect(crop).toHaveAttribute("aria-pressed", "true");
    expect(within(panel).queryByRole("button", { name: "Reset crop" })).toBeNull();
    await userEvent.click(within(panel).getByRole("button", { name: "Rotate right" }));
    await userEvent.click(await within(panel).findByRole("button", { name: "Reset crop" }));
    await waitFor(() => expect(steps()).toContain("Reset crop"));
  });

  it("opens with R and previews the whole photo without its geometry", async () => {
    const panel = await open();
    expect(within(panel).getByRole("button", { name: "Done" })).toBeInTheDocument();
    expect(await screen.findByRole("group", { name: "Crop area; drag to move" })).toBeInTheDocument();
    // Turn it while open: the render stays geometry-free; the canvas draws the turn.
    await userEvent.click(within(panel).getByRole("button", { name: "Rotate right" }));
    await waitFor(() => expect(steps()).toContain("Rotate"));
    expect(lastPreview().rotate).toBe(0);
    expect(lastPreview().crop).toBeNull();
  });

  it("commits one Crop step on Done and renders the new geometry", async () => {
    const panel = await open();
    await userEvent.click(within(panel).getByRole("button", { name: "1:1" }));
    // Drag the top-left corner in: with 1:1 the box stays square in pixels.
    const corner = await screen.findByRole("button", { name: "Crop top-left corner" });
    fireEvent.pointerDown(corner, { clientX: 0, clientY: 0, pointerId: 1 });
    fireEvent.pointerMove(corner, { clientX: 60, clientY: 40, pointerId: 1 });
    fireEvent.pointerUp(corner, { clientX: 60, clientY: 40, pointerId: 1 });
    expect(steps()).not.toContain("Crop");
    await userEvent.click(within(panel).getByRole("button", { name: "Done" }));
    await waitFor(() => expect(steps().filter((s) => s === "Crop")).toHaveLength(1));
    await waitFor(() => expect(lastPreview().crop).not.toBeNull());
    const k = lastPreview().crop!;
    // Square in pixels on a 3:2 photo: w / h = 2 / 3 in frame units.
    expect((k.w * 6000) / (k.h * 4000)).toBeCloseTo(1, 2);
    expect(screen.queryByRole("region", { name: "Crop" })).toBeNull();
  });

  it("puts the old geometry back on Esc", async () => {
    await open();
    const corner = await screen.findByRole("button", { name: "Crop bottom-right corner" });
    fireEvent.pointerDown(corner, { clientX: 600, clientY: 400, pointerId: 1 });
    fireEvent.pointerMove(corner, { clientX: 300, clientY: 200, pointerId: 1 });
    fireEvent.pointerUp(corner, { clientX: 300, clientY: 200, pointerId: 1 });
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("region", { name: "Crop" })).toBeNull());
    await waitFor(() => expect(lastPreview().crop).toBeNull());
    expect(steps()).not.toContain("Crop");
  });

  it("makes each turn and flip a step of its own", async () => {
    const panel = await open();
    await userEvent.click(within(panel).getByRole("button", { name: "Rotate right" }));
    await userEvent.click(within(panel).getByRole("button", { name: "Rotate right" }));
    await userEvent.click(within(panel).getByRole("button", { name: "Flip horizontal" }));
    await waitFor(() => expect(steps()).toEqual(["Rotate", "Rotate", "Flip"]));
    await userEvent.click(within(panel).getByRole("button", { name: "Done" }));
    await waitFor(() => expect(lastPreview().rotate).toBe(180));
    expect(steps()).toEqual(["Rotate", "Rotate", "Flip"]);
  });

  it("levels the photo from a line drawn along the horizon", async () => {
    const panel = await open();
    await userEvent.click(within(panel).getByRole("button", { name: "Draw a level line" }));
    const stage = screen.getByLabelText("Draw along something that should be level");
    fireEvent.pointerDown(stage, { clientX: 100, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(stage, { clientX: 300, clientY: 110, pointerId: 1 });
    fireEvent.pointerUp(stage, { clientX: 300, clientY: 110, pointerId: 1 });
    await waitFor(() => expect(steps()).toContain("Straighten"));
    await userEvent.click(within(panel).getByRole("button", { name: "Done" }));
    // atan(10 / 200) ≈ 2.86°, turned back the other way.
    await waitFor(() => expect(lastPreview().straighten).toBeCloseTo(-2.86, 1));
  });
});
