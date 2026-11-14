import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { DEFAULT_DEVELOP_SOURCE, developSourceStore } from "@/lib/develop-source";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { DevelopCanvas } from "./canvas";
import { DevelopRoute } from "./gui";
import { clampPan, levelOf, visibleRect, wheelFactor, zoomAround } from "./zoom";

const calls = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);

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

describe("DevelopCanvas", () => {
  afterEach(() => vi.restoreAllMocks());
  beforeEach(() => {
    // jsdom lays nothing out: the stage is 1200 × 800.
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1200);
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(800);
    invokeMock.mockClear();
    developSourceStore.set(DEFAULT_DEVELOP_SOURCE);
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("develop_preview", () => ({ token: "1", width: 300, height: 200, superseded: false, histogram: null }));
    mockCommand("develop_region", () => ({ token: "2", width: 100, height: 80, superseded: false, histogram: null }));
    mockCommand("develop_release_full", () => null);
    mockCommand("develop_base_info", () => ({ width: 6000, height: 4000, raw: true }));
    mockCommand("strip_page", () => ({ items: [], nextCursor: null }));
    mockCommand("strip_ids", () => [5]);
    mockCommand("list_presets", () => []);
    mockCommand("list_history", () => []);
  });

  const level = () => screen.getByRole("button", { name: "Zoom level" });
  const ready = async () => {
    mount();
    await screen.findByRole("img", { name: "The photo as developed" });
    await waitFor(() => expect(calls("develop_base_info")).toHaveLength(1));
  };

  it("shows before with backslash and paints clipping with J", async () => {
    mount();
    await screen.findByRole("img", { name: "The photo as developed" });
    fireEvent.keyDown(window, { key: "\\" });
    await waitFor(() => expect(calls("develop_preview").some((a) => a.before === true)).toBe(true));
    expect(screen.getByText("Before")).toBeInTheDocument();
    fireEvent.keyDown(window, { key: "j" });
    await waitFor(() => expect(calls("develop_preview").some((a) => a.clipping === true)).toBe(true));
  });

  it("asks for the visible part at 100 % and frees the frame when zooming out", async () => {
    await ready();
    expect(level()).toHaveTextContent("Fit");
    fireEvent.keyDown(window, { key: "z" });
    await waitFor(() => expect(level()).toHaveTextContent("100 %"));
    await waitFor(() => expect(calls("develop_region")).toHaveLength(1));
    expect(calls("develop_region")[0]).toMatchObject({ assetId: 5, before: false, scale: 1 });
    fireEvent.keyDown(window, { key: "z" });
    await waitFor(() => expect(calls("develop_release_full").length).toBeGreaterThan(0));
    expect(level()).toHaveTextContent("Fit");
  });

  it("zooms from the bar: fit, fill, 100 %, a level from the menu, and steps", async () => {
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "Zoom to 100 %" }));
    expect(level()).toHaveTextContent("100 %");
    await userEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(level()).toHaveTextContent("200 %");
    await userEvent.click(screen.getByRole("button", { name: "Zoom out" }));
    expect(level()).toHaveTextContent("100 %");
    await userEvent.click(level());
    await userEvent.click(await screen.findByRole("menuitem", { name: "400 %" }));
    expect(level()).toHaveTextContent("400 %");
    await userEvent.click(screen.getByRole("button", { name: "Fill" }));
    expect(level()).toHaveTextContent("Fill");
    await userEvent.click(screen.getByRole("button", { name: "Fit" }));
    expect(level()).toHaveTextContent("Fit");
    // And from the keyboard: Ctrl+= and Ctrl+− step, Ctrl+0 fits.
    fireEvent.keyDown(window, { key: "=", ctrlKey: true });
    expect(level()).toHaveTextContent("25 %");
    fireEvent.keyDown(window, { key: "=", ctrlKey: true });
    expect(level()).toHaveTextContent("50 %");
    fireEvent.keyDown(window, { key: "-", ctrlKey: true });
    expect(level()).toHaveTextContent("25 %");
    fireEvent.keyDown(window, { key: "0", ctrlKey: true });
    expect(level()).toHaveTextContent("Fit");
  });

  it("zooms around the cursor with Ctrl+wheel, and renders what is then in view", async () => {
    await ready();
    const stage = document.querySelector(".develop-canvas") as HTMLElement;
    fireEvent.wheel(stage, { deltaY: -500, ctrlKey: true, clientX: 300, clientY: 200 });
    // Where the maths puts it: fit (inside the stage's 16 px padding), then around (300, 200).
    const photo = { w: 6000, h: 4000 };
    const view = { w: 1200, h: 800 };
    const from = { mode: "level" as const, level: Math.min(1168 / 6000, 768 / 4000) };
    const want = zoomAround(from, wheelFactor(-500), { x: 300, y: 200 }, clampPan({ x: 0, y: 0 }, from, photo, view), photo, view);
    await waitFor(() => expect(calls("develop_region")).toHaveLength(1));
    const asked = calls("develop_region")[0] as { rect: { x: number; y: number }; scale: number };
    const rect = visibleRect(want.pan, want.zoom, photo, view);
    expect(asked.rect.x).toBeCloseTo(rect.x, 4);
    expect(asked.rect.y).toBeCloseTo(rect.y, 4);
    expect(asked.scale).toBeCloseTo(levelOf(want.zoom, photo, view), 4);
    expect(level()).toHaveTextContent(`${Math.round(levelOf(want.zoom, photo, view) * 100)} %`);
  });

  it("a_stale_region_is_never_drawn", async () => {
    const pending: ((v: unknown) => void)[] = [];
    mockCommand("develop_region", () => new Promise((res) => pending.push(res)));
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "Zoom to 100 %" }));
    await waitFor(() => expect(pending).toHaveLength(1));
    await userEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    await waitFor(() => expect(pending).toHaveLength(2));
    const regionSrc = () => document.querySelector(".develop-canvas__region")?.getAttribute("src") ?? null;
    await act(async () => pending[1]({ token: "newer", width: 600, height: 400, superseded: false, histogram: null }));
    await waitFor(() => expect(regionSrc()).toContain("newer"));
    await act(async () => pending[0]({ token: "older", width: 1200, height: 800, superseded: false, histogram: null }));
    expect(regionSrc()).toContain("newer");
  });

  it("shows a navigator only past fit, and pans by dragging in it", async () => {
    await ready();
    expect(screen.queryByRole("img", { name: "Navigator" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Zoom to 100 %" }));
    const nav = await screen.findByRole("img", { name: "Navigator" });
    await waitFor(() => expect(calls("develop_region")).toHaveLength(1));
    // A press at the navigator's top-left corner moves the view there.
    vi.spyOn(nav, "getBoundingClientRect").mockReturnValue({ left: 0, top: 0, width: 120, height: 80, right: 120, bottom: 80, x: 0, y: 0, toJSON: () => ({}) });
    fireEvent.pointerDown(nav, { clientX: 0, clientY: 0, pointerId: 1 });
    fireEvent.pointerUp(nav, { pointerId: 1 });
    await waitFor(() => expect(calls("develop_region")).toHaveLength(2));
    expect(calls("develop_region")[1]).toMatchObject({ rect: { x: 0, y: 0 } });
    await userEvent.click(screen.getByRole("button", { name: "Fit" }));
    expect(screen.queryByRole("img", { name: "Navigator" })).toBeNull();
  });
});

describe("moving between photos", () => {
  const session = (preview: { assetId: number; url: string } | null) =>
    ({
      preview: preview && { ...preview, width: 300, height: 200 },
      draft: { version: 2, crop: null, rotate: 0, straighten: 0, flipH: false, flipV: false, develop: null, trim: null },
      before: false,
      clipping: false,
      rendering: false,
      setBefore: () => {},
      setClipping: () => {},
    }) as unknown as DevelopSession;
  const canvas = (assetId: number, s: DevelopSession) => (
    <Providers>
      <DevelopCanvas session={s} assetId={assetId} marks={null} order={[1, 2, 3]} />
    </Providers>
  );

  beforeEach(() => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("develop_base_info", () => ({ width: 3000, height: 2000, raw: false }));
  });

  it("slides the next photo in at once from its cached rendition, then swaps its render in place", async () => {
    const { container, rerender } = render(canvas(1, session({ assetId: 1, url: "a" })));
    const imgs = () => [...container.querySelectorAll("img.develop-canvas__img")] as HTMLImageElement[];
    const photo = (id: number) => imgs().find((i) => i.dataset.photo === String(id) && !i.classList.contains("develop-canvas__img--leaving"));
    // Photo 2's render is not in yet: its cached 1024 px rendition stands in, so the canvas
    // shows the photo the panels show.
    rerender(canvas(2, session({ assetId: 1, url: "a" })));
    await waitFor(() => expect(photo(2)?.getAttribute("src")).toContain("thumb/2"));
    expect(photo(2)?.dataset.direction).toBe("1");
    // Its render arrives: same photo, swapped in place, no second slide.
    rerender(canvas(2, session({ assetId: 2, url: "b" })));
    expect(photo(2)?.getAttribute("src")).toBe("b");
    // Back again: from the other side.
    rerender(canvas(1, session({ assetId: 2, url: "b" })));
    await waitFor(() => expect(photo(1)?.dataset.direction).toBe("-1"));
  });
});
