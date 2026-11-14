import { act, render, screen, waitFor } from "@testing-library/react";
import { MotionGlobalConfig } from "motion/react";
import { DEFAULT_DEVELOP_SOURCE, developSourceStore } from "@/lib/develop-source";
import { pageKey } from "./root-layout";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { routes } from "./router";

const settings = {
  version: 1,
  appearance: { theme: "gallery", scheme: "auto", tileSize: "m", reduceMotion: false, sidebarCollapsed: false },
  import: { duplicates: "skip", promptOnVolume: true },
  playback: { loopVideos: false, muteByDefault: false },
  map: { online: true, style: "map" as const },
  window: { width: 1280, height: 800, x: null, y: null, maximized: false },
};

function mount(path: string, current: unknown) {
  mockCommand("get_settings", () => settings);
  mockLibraryDefaults();
  mockCommand("get_current_library", () => current);
  const router = createMemoryRouter(routes, { initialEntries: [path] });
  render(
    <Providers>
      <RouterProvider router={router} />
    </Providers>,
  );
  return router;
}

describe("router", () => {
  afterEach(() => {
    MotionGlobalConfig.skipAnimations = false;
  });

  it("redirects to /setup when no library is open", async () => {
    const r = mount("/library", null);
    await waitFor(() => expect(r.state.location.pathname).toBe("/setup"));
  });

  it("renders the library empty state when a library is open", async () => {
    mount("/library", { name: "Lumen Library", path: "/x", schemaVersion: 1 });
    expect(await screen.findByRole("heading", { level: 1, name: "Library" })).toBeInTheDocument();
    expect(await screen.findByRole("status")).toHaveTextContent(/import/i);
  });

  // Round 4A: stepping through Develop's filmstrip changes /develop/:id. The page transition
  // was keyed by the whole path, so every step faded the screen out and mounted a new one.
  it("moves between photos in Develop without remounting the page", async () => {
    // Exits finish at once, as they do in the app after 180 ms; in jsdom they otherwise never
    // end, and the old page would hide a remount.
    MotionGlobalConfig.skipAnimations = true;
    developSourceStore.set({ ...DEFAULT_DEVELOP_SOURCE, only: [1, 2], label: "Selection" });
    mockCommand("strip_ids", () => [1, 2]);
    mockCommand("strip_page", () => ({ items: [], nextCursor: null }));
    mockCommand("develop_preview", (a) => ({ token: String(a?.assetId), width: 300, height: 200, superseded: false, histogram: null }));
    const r = mount("/develop/1", { name: "Lumen Library", path: "/x", schemaVersion: 1 });
    const screenOf = () => document.querySelector(".route-transition .develop");
    const before = await waitFor(() => {
      const el = screenOf();
      expect(el).not.toBeNull();
      return el;
    });
    // Develop scrolls inside its own panes: the route container neither scrolls nor keeps a gutter.
    expect(document.querySelector(".route-transition")!.classList.contains("route-transition--fixed")).toBe(true);
    await act(async () => r.navigate("/develop/2"));
    // Settled: one page on screen, rendering photo 2.
    await waitFor(() => {
      expect(document.querySelectorAll(".route-transition")).toHaveLength(1);
      expect(invokeMock).toHaveBeenCalledWith("develop_preview", expect.objectContaining({ assetId: 2 }));
    });
    expect(screenOf()).toBe(before);
  });

  // Round 20: the Slideshows and Pets lists are tabs of Memories and People & Pets.
  it("sends the old Slideshows and Pets lists to their tabs", async () => {
    const lib = { name: "Lumen Library", path: "/x", schemaVersion: 1 };
    const r = mount("/slideshows", lib);
    await waitFor(() => expect(r.state.location.pathname + r.state.location.search).toBe("/memories?tab=slideshows"));
    await act(async () => r.navigate("/pets"));
    await waitFor(() => expect(r.state.location.pathname + r.state.location.search).toBe("/people?show=pets"));
  });

  it("keys a page by its screen, not by the photo inside it", () => {
    expect(pageKey("/develop/5")).toBe(pageKey("/develop/6"));
    expect(pageKey("/develop")).toBe(pageKey("/develop/6"));
    expect(pageKey("/albums/3")).not.toBe(pageKey("/albums/4"));
    expect(pageKey("/library")).not.toBe(pageKey("/develop"));
  });
});
