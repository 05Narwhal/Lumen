import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, useLocation } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { albumPickerStore } from "@/components/album-picker/store";
import { ContextMenuHost } from "@/components/context-menu";
import { sequenceNeighbors } from "@/lib/viewer-sequence";
import { mapCalls } from "@/test/maplibre-mock";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { MapRoute } from "./gui";

vi.mock("maplibre-gl", async () => ({ Map: (await import("@/test/maplibre-mock")).FakeMap, setWorkerUrl: () => {} }));

const cluster = (lat: number, lon: number, count: number, id: number) => ({
  lat,
  lon,
  count,
  coverAssetId: id,
  assetId: count === 1 ? id : null,
  bounds: { minLat: lat - 0.1, maxLat: lat + 0.1, minLon: lon - 0.1, maxLon: lon + 0.1 },
});

function SearchProbe() {
  return <p data-testid="search">{useLocation().search}</p>;
}

function mount() {
  render(
    <Providers>
      <MemoryRouter initialEntries={["/map"]}>
        <MapRoute />
      </MemoryRouter>
    </Providers>,
  );
}

describe("MapRoute", () => {
  const props = ["clientWidth", "clientHeight", "offsetWidth", "offsetHeight"] as const;
  beforeEach(() => {
    mapCalls.fitBounds.length = 0;
    mapCalls.flyTo.length = 0;
    mapCalls.setStyle.length = 0;
    for (const p of props) Object.defineProperty(HTMLElement.prototype, p, { configurable: true, get: () => (p.endsWith("Width") ? 900 : 600) });
    // jsdom has no canvas backend; the map only needs the call not to throw.
    HTMLCanvasElement.prototype.getContext = (() => null) as unknown as HTMLCanvasElement["getContext"];
  });
  afterEach(() => {
    for (const p of props) delete (HTMLElement.prototype as unknown as Record<string, unknown>)[p];
  });

  it("explains itself when nothing has a location", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mount();
    expect(await screen.findByRole("status")).toHaveTextContent(/No photos with a location yet/);
  });

  it("asks for clusters in the viewport and counts what is in view", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("map_bounds", () => ({ minLat: 38.0, maxLat: 48.0, minLon: -9.5, maxLon: 12.5 }));
    mockCommand("map_clusters", () => [cluster(38.72, -9.14, 34, 4), cluster(46.06, 14.51, 1, 9)]);
    mount();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("map_clusters", expect.objectContaining({ zoom: expect.any(Number) })));
    expect(await screen.findByText("35 in view")).toBeInTheDocument();
    // A group offers to zoom; a lone photo offers to open.
    expect(await screen.findByRole("button", { name: "Open 34 photos" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open this photo" })).toBeInTheDocument();
  });

  it("searches the bundled atlas and flies to a place", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("map_bounds", () => ({ minLat: 38.0, maxLat: 48.0, minLon: -9.5, maxLon: 12.5 }));
    mockCommand("map_clusters", () => []);
    mockCommand("search_places", () => [{ label: "Lisbon, Portugal", lat: 38.72, lon: -9.14 }]);
    // Offline here: the online geocoder is unreachable, so the bundled atlas answers.
    vi.stubGlobal("fetch", vi.fn(async () => Promise.reject(new TypeError("offline"))));
    mount();
    await userEvent.type(await screen.findByRole("textbox", { name: "Find a place" }), "lisb");
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("search_places", expect.objectContaining({ query: "lisb" })));
    await userEvent.click(await screen.findByRole("button", { name: "Lisbon, Portugal" }));
    expect(mapCalls.flyTo.at(-1)).toMatchObject({ center: [-9.14, 38.72] });
    vi.unstubAllGlobals();
  });

  it("gives a group's pin a menu that hands its photos to the album picker and a slideshow (Round 4G)", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("map_bounds", () => ({ minLat: 38.0, maxLat: 48.0, minLon: -9.5, maxLon: 12.5 }));
    const group = cluster(38.72, -9.14, 3, 42);
    mockCommand("map_clusters", () => [group]);
    mockCommand("assets_in_bounds", (a) => ((a?.limit as number) === 500 ? [42, 41, 40] : [42, 41, 40, 7, 8]));
    render(
      <Providers>
        <MemoryRouter initialEntries={["/map"]}>
          <MapRoute />
          <SearchProbe />
        </MemoryRouter>
        <ContextMenuHost />
      </Providers>,
    );
    const pin = await screen.findByRole("button", { name: "Open 3 photos" });
    await userEvent.pointer({ keys: "[MouseRight]", target: pin });
    await userEvent.click(await screen.findByRole("menuitem", { name: "Add to album…" }, { timeout: 5000 }));
    await waitFor(() => expect(albumPickerStore.get().ids).toEqual([42, 41, 40]));
    await userEvent.pointer({ keys: "[MouseRight]", target: pin });
    await userEvent.click(await screen.findByRole("menuitem", { name: "Start slideshow of these" }, { timeout: 5000 }));
    await waitFor(() => expect(screen.getByTestId("search")).toHaveTextContent("play=1"));
    expect(sequenceNeighbors(42)).toMatchObject({ index: 0, total: 3 });
  });

  it("opens a group as its own set in the viewer and zooms the map to it", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("map_bounds", () => ({ minLat: 38.0, maxLat: 48.0, minLon: -9.5, maxLon: 12.5 }));
    const group = cluster(38.72, -9.14, 3, 42);
    mockCommand("map_clusters", () => [group]);
    mockCommand("assets_in_bounds", (a) => ((a?.limit as number) === 500 ? [42, 41, 40] : [42, 41, 40, 7, 8]));
    render(
      <Providers>
        <MemoryRouter initialEntries={["/map"]}>
          <MapRoute />
          <SearchProbe />
        </MemoryRouter>
      </Providers>,
    );
    await userEvent.click(await screen.findByRole("button", { name: "Open 3 photos" }));
    await waitFor(() => expect(screen.getByTestId("search")).toHaveTextContent("asset=42"));
    expect(invokeMock).toHaveBeenCalledWith("assets_in_bounds", expect.objectContaining({ bounds: group.bounds, limit: 500 }));
    expect(sequenceNeighbors(42)).toMatchObject({ index: 0, total: 3, next: 41 });
    const b = group.bounds;
    expect(mapCalls.fitBounds.at(-1)?.[0]).toEqual([
      [b.minLon, b.minLat],
      [b.maxLon, b.maxLat],
    ]);
  });

  it("frames the library, asks for clusters at the clustering's zoom, and switches to satellite", async () => {
    mockCommand("get_settings", () => ({
      version: 1,
      appearance: { theme: "gallery", scheme: "light", tileSize: "m", reduceMotion: false, sidebarCollapsed: false },
      import: { duplicates: "skip", promptOnVolume: true },
      playback: { loopVideos: false, muteByDefault: false },
      map: { online: true, style: "map" },
      window: { width: 1280, height: 800, x: null, y: null, maximized: false },
    }));
    mockCommand("update_settings", (a) => a?.settings ?? null);
    mockLibraryDefaults();
    mockCommand("map_bounds", () => ({ minLat: 38.0, maxLat: 48.0, minLon: -9.5, maxLon: 12.5 }));
    mockCommand("map_clusters", () => [cluster(38.72, -9.14, 3, 4)]);
    mount();
    await waitFor(() => expect(mapCalls.fitBounds.length).toBeGreaterThan(0));
    // MapLibre's zoom 1 (512px tiles) is the clustering's zoom 2 (256px tiles).
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("map_clusters", expect.objectContaining({ zoom: 2, bounds: { minLat: 30, maxLat: 55, minLon: -20, maxLon: 30 } })),
    );
    await userEvent.click(screen.getByRole("radio", { name: "Satellite" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("update_settings", expect.anything()));
  });
});
