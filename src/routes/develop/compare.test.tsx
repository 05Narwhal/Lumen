import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { DEFAULT_DEVELOP_SOURCE, developSourceStore } from "@/lib/develop-source";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { DevelopRoute } from "./gui";

const calls = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);
const stage = () => document.querySelector(".develop-canvas") as HTMLElement;

describe("before and after", () => {
  afterEach(() => vi.restoreAllMocks());
  beforeEach(() => {
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1200);
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(800);
    invokeMock.mockClear();
    developSourceStore.set(DEFAULT_DEVELOP_SOURCE);
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("develop_preview", () => ({ token: "after", width: 300, height: 200, superseded: false, histogram: null }));
    mockCommand("develop_before", () => ({ token: "before", width: 300, height: 200, superseded: false, histogram: null }));
    mockCommand("develop_region", (a) => ({ token: a?.before ? "region-before" : "region-after", width: 100, height: 80, superseded: false, histogram: null }));
    mockCommand("develop_release_full", () => null);
    mockCommand("develop_base_info", () => ({ width: 6000, height: 4000, raw: false }));
    mockCommand("strip_page", () => ({ items: [], nextCursor: null }));
    mockCommand("strip_ids", () => [5]);
    mockCommand("list_presets", () => []);
    mockCommand("list_history", () => []);
  });

  const ready = async () => {
    render(
      <Providers>
        <MemoryRouter initialEntries={["/develop/5"]}>
          <Routes>
            <Route path="/develop/:id?" element={<DevelopRoute />} />
          </Routes>
        </MemoryRouter>
      </Providers>,
    );
    await screen.findByRole("img", { name: "The photo as developed" });
    await waitFor(() => expect(calls("develop_base_info")).toHaveLength(1));
  };

  it("switches between toggle, split and side by side from its menu", async () => {
    await ready();
    const pick = async (name: string) => {
      await userEvent.click(screen.getByRole("button", { name: "Before and after" }));
      await userEvent.click(await screen.findByRole("menuitem", { name }));
    };
    await pick("Split");
    expect(stage().dataset.compare).toBe("split");
    // Its alt is empty: a failed load never shows alt text (Round 24 §4).
    await waitFor(() => expect(document.querySelector(".develop-canvas__layer--before img")).toHaveAttribute("src", expect.stringContaining("before")));
    expect(document.querySelector(".develop-canvas__layer--before img")).toHaveAttribute("alt", "");
    expect(screen.getByRole("separator", { name: "Divider between before and after" })).toBeInTheDocument();
    await pick("Side by side");
    expect(stage().dataset.compare).toBe("side");
    expect(document.querySelectorAll(".develop-canvas__pane")).toHaveLength(2);
    expect(screen.getAllByText("Before").length).toBeGreaterThan(0);
    await pick("Toggle");
    expect(stage().dataset.compare).toBe("toggle");
    await waitFor(() => expect(calls("develop_preview").some((a) => a.before === true)).toBe(true));
    await pick("Off");
    expect(stage().dataset.compare).toBe("off");
    // One before for the photo, however often the modes changed.
    expect(calls("develop_before")).toHaveLength(1);
  });

  it("cycles split, side by side and off with Y", async () => {
    await ready();
    fireEvent.keyDown(window, { key: "y" });
    expect(stage().dataset.compare).toBe("split");
    fireEvent.keyDown(window, { key: "y" });
    expect(stage().dataset.compare).toBe("side");
    fireEvent.keyDown(window, { key: "y" });
    expect(stage().dataset.compare).toBe("off");
  });

  it("moves the divider by dragging it, without panning the photo", async () => {
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "Zoom to 100 %" }));
    fireEvent.keyDown(window, { key: "y" });
    const divider = await screen.findByRole("separator", { name: "Divider between before and after" });
    expect(divider).toHaveAttribute("aria-valuenow", "50");
    const frame = () => (document.querySelector(".develop-canvas__frame") as HTMLElement).style.transform;
    const was = frame();
    fireEvent.pointerDown(divider, { clientX: 600, clientY: 400, pointerId: 1, button: 0 });
    fireEvent.pointerMove(divider, { clientX: 300, clientY: 400, pointerId: 1 });
    fireEvent.pointerUp(divider, { pointerId: 1 });
    expect(divider).toHaveAttribute("aria-valuenow", "25");
    expect(frame()).toBe(was);
    // The zoomed before asks for its own region beside the after's.
    await waitFor(() => expect(calls("develop_region").some((a) => a.before === true)).toBe(true));
    await waitFor(() => expect(document.querySelector(".develop-canvas__layer--before .develop-canvas__region")?.getAttribute("src")).toContain("region-before"));
  });
});
