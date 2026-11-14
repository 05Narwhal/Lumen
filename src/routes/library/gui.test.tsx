import { MotionGlobalConfig } from "motion/react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { EMPTY_SEQ, flatTags } from "@/lib/search-expr";
import type { TagSeq } from "@/types/generated/TagSeq";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { LibraryRoute, YEAR_TILE } from "./gui";

// Exit animations never finish in jsdom: a chip that moved into a group would show twice.
MotionGlobalConfig.skipAnimations = true;

const settings = {
  version: 1,
  appearance: { theme: "gallery", scheme: "auto", tileSize: "m", libraryGroupBy: "month" as const, reduceMotion: false, sidebarCollapsed: false },
  import: { duplicates: "skip", promptOnVolume: true },
  playback: { loopVideos: false, muteByDefault: false },
  map: { online: true, style: "map" as const },
  music: { jamendoClientId: null },
  window: { width: 1280, height: 800, x: null, y: null, maximized: false },
};

function mount(path = "/library") {
  render(
    <Providers>
      <MemoryRouter initialEntries={[path]}>
        <Routes>
          <Route path="/library" element={<LibraryRoute />} />
          <Route path="/library/:deviceId" element={<LibraryRoute />} />
          <Route path="/library/group/:group" element={<LibraryRoute />} />
        </Routes>
      </MemoryRouter>
    </Providers>,
  );
}

describe("LibraryRoute", () => {
  const props = ["clientWidth", "clientHeight", "offsetWidth", "offsetHeight"] as const;
  beforeEach(() => {
    for (const p of props) Object.defineProperty(HTMLElement.prototype, p, { configurable: true, get: () => (p.endsWith("Width") ? 1000 : 800) });
  });
  afterEach(() => {
    for (const p of props) delete (HTMLElement.prototype as unknown as Record<string, unknown>)[p];
  });

  const searchTags = (scope: unknown) => flatTags((scope as { query?: { tagExpr?: TagSeq } }).query?.tagExpr ?? EMPTY_SEQ).map((t) => t.label);
  const lastScope = () => {
    const calls = invokeMock.mock.calls.filter(([cmd]) => cmd === "get_layout");
    return (calls[calls.length - 1]?.[1] as { scope: unknown }).scope;
  };

  it("turns a search in the URL into a tag, counts the matches, and Clear removes it", async () => {
    mockCommand("get_settings", () => settings);
    mockLibraryDefaults();
    mockCommand("get_library_stats", () => ({ items: 12, videos: 1, bytes: 1000 }));
    mockCommand("count_scope", () => 3);
    mount("/library?q=beach");
    await waitFor(() => expect(searchTags(lastScope())).toEqual(["beach"]));
    expect(await screen.findByText("3 match")).toBeInTheDocument();
    expect(screen.getByRole("listitem")).toHaveTextContent("“beach”");
    await userEvent.click(screen.getByRole("button", { name: "Clear" }));
    await waitFor(() => expect(screen.queryByText("3 match")).toBeNull());
    await waitFor(() => expect(searchTags(lastScope())).toEqual([]));
  });

  it("adds tags from the field's suggestions, OR within a field, and Backspace takes the last", async () => {
    mockCommand("get_settings", () => settings);
    mockLibraryDefaults();
    mockCommand("get_library_stats", () => ({
      items: 12,
      videos: 1,
      bytes: 1000,
    }));
    mockCommand("suggest_tags", (a) => {
      const input = String((a as { input: string }).input).toLowerCase();
      const all = [
        {
          tag: { label: "Videos", value: { type: "media", media: 1 } },
          field: "Kind",
          count: 1,
        },
        {
          tag: { label: "Photos", value: { type: "media", media: 0 } },
          field: "Kind",
          count: 11,
        },
        {
          tag: { label: "Anna Berg", value: { type: "person", id: 4 } },
          field: "Person",
          count: 5,
        },
      ];
      return all.filter((s) => s.tag.label.toLowerCase().startsWith(input));
    });
    mount();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_layout", expect.anything()));
    const field = screen.getByRole("combobox", { name: "Search" });

    await userEvent.type(field, "vid");
    const option = await screen.findByRole("option", { name: /Videos/ });
    expect(option).toHaveTextContent("1");
    expect(screen.getByRole("option", { name: /“vid”/ })).toBeInTheDocument();
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(searchTags(lastScope())).toEqual(["Videos"]));
    expect(field).toHaveValue("");

    await userEvent.type(field, "pho");
    await screen.findByRole("option", { name: /Photos/ });
    await userEvent.keyboard("{Tab}");
    await waitFor(() => expect(searchTags(lastScope())).toEqual(["Videos", "Photos"]));
    // The token row shows the `or` between the chips as a button.
    expect(screen.getByRole("button", { name: "or — change how these combine" })).toBeInTheDocument();

    // ↓ moves past the first suggestion; Enter takes the one chosen.
    await userEvent.type(field, "anna");
    await screen.findByRole("option", { name: /Anna Berg/ });
    await userEvent.keyboard("{ArrowDown}{Enter}");
    await waitFor(() => expect(searchTags(lastScope())).toEqual(["Videos", "Photos", "anna"]));

    // Back to tags the grid has shown before: its layout comes from the cache, so read the bar.
    const bar = () => screen.getAllByRole("listitem").map((li) => li.textContent);
    expect(bar()).toEqual(["Videos", "Photos", "“anna”"]);
    await userEvent.keyboard("{Backspace}");
    await waitFor(() => expect(bar()).toEqual(["Videos", "Photos"]));
    await userEvent.click(screen.getByRole("button", { name: "Remove Videos" }));
    await waitFor(() => expect(bar()).toEqual(["Photos"]));
  });

  it("turns an old filter link into tags, and the page is named for what is left", async () => {
    mockCommand("get_settings", () => settings);
    mockLibraryDefaults();
    mockCommand("get_library_stats", () => ({ items: 12, videos: 1, bytes: 1000 }));
    mount("/library?filters=%7B%22minRating%22%3A4%2C%22favorites%22%3Atrue%7D");
    expect(await screen.findByText("4 stars and up")).toBeInTheDocument();
    expect(screen.getByText("Favorites")).toBeInTheDocument();
    await userEvent.click(screen.getByLabelText("Remove 4 stars and up"));
    await waitFor(() => expect(screen.queryByText("4 stars and up")).toBeNull());
    expect(screen.getByLabelText("Remove Favorites")).toBeInTheDocument();
    // What is left is exactly the Favorites collection, so the page says so.
    expect(screen.getByRole("heading", { name: "Favorites" })).toBeInTheDocument();
  });

  it("scopes the grid to a device and shows its name", async () => {
    mockCommand("get_settings", () => settings);
    mockLibraryDefaults();
    mockCommand("get_library_stats", () => ({ items: 12, videos: 1, bytes: 1000 }));
    mockCommand("list_devices", () => [{ id: 3, displayName: "Sony ILCE-7M4", kind: 2, icon: null, itemCount: 7, lastImportAt: 1, hidden: false }]);
    mount("/library/3");
    expect(await screen.findByText("Sony ILCE-7M4")).toBeInTheDocument();
    expect(screen.getByText("7 items")).toBeInTheDocument();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_layout", expect.objectContaining({ scope: expect.objectContaining({ deviceId: 3 }) })));
  });

  it("scopes the grid to every device of a group and names the group", async () => {
    mockCommand("get_settings", () => settings);
    mockLibraryDefaults();
    mockCommand("get_library_stats", () => ({ items: 12, videos: 1, bytes: 1000 }));
    const device = (id: number, group: string, itemCount: number) => ({ id, displayName: `D${id}`, kind: 1, icon: null, itemCount, lastImportAt: 1, hidden: false, group });
    mockCommand("list_devices", () => [device(1, "iphone", 5), device(2, "camera", 9), device(3, "iphone", 6)]);
    mount("/library/group/iphone");
    expect(await screen.findByText("iPhones")).toBeInTheDocument();
    expect(await screen.findByText("11 items")).toBeInTheDocument();
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "get_layout",
        expect.objectContaining({ scope: expect.objectContaining({ deviceId: null, query: expect.objectContaining({ deviceIds: [1, 3] }) }) }),
      ),
    );
  });

  it("shows the empty state when the library has no items", async () => {
    mockCommand("get_settings", () => settings);
    mockLibraryDefaults();
    mount();
    expect(await screen.findByRole("status")).toHaveTextContent(/No photos yet/);
    expect(screen.queryByRole("grid")).toBeNull();
  });

  it("shows the grid, the tag search and tile-size control when items exist", async () => {
    mockCommand("get_settings", () => settings);
    mockLibraryDefaults();
    mockCommand("get_library_stats", () => ({ items: 12, videos: 1, bytes: 1000 }));
    mount();
    expect(await screen.findByRole("grid")).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "Search" })).toBeInTheDocument();
    // The All / Photos / Videos pills are gone: tags say it now (Round 7).
    expect(screen.queryByRole("radio", { name: "Videos" })).toBeNull();
    expect(screen.getByRole("radiogroup", { name: "Tile size" })).toBeInTheDocument();
  });

  describe("grouping", () => {
    const withGroup = (libraryGroupBy: "day" | "month" | "year", tileSize = "m") => ({
      ...settings,
      appearance: { ...settings.appearance, libraryGroupBy, tileSize },
    });

    it("asks for sections by day, month or year", async () => {
      mockCommand("get_settings", () => settings);
      mockLibraryDefaults();
      mockCommand("get_library_stats", () => ({ items: 12, videos: 1, bytes: 1000 }));
      mount();
      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith("get_layout", expect.objectContaining({ groupBy: "month", rowHeight: 200 })),
      );
      await userEvent.click(screen.getByRole("radio", { name: "Days" }));
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_layout", expect.objectContaining({ groupBy: "day" })));
      expect(invokeMock).toHaveBeenCalledWith("update_settings", expect.objectContaining({
        settings: expect.objectContaining({ appearance: expect.objectContaining({ libraryGroupBy: "day" }) }),
      }));
    });

    it("offers S, M and L by day, but only S and M by month", async () => {
      mockCommand("get_settings", () => withGroup("day"));
      mockLibraryDefaults();
      mockCommand("get_library_stats", () => ({ items: 12, videos: 1, bytes: 1000 }));
      mount();
      const sizes = await screen.findByRole("radiogroup", { name: "Tile size" });
      await waitFor(() => expect(sizes.querySelectorAll("button")).toHaveLength(3));
      await userEvent.click(screen.getByRole("radio", { name: "Months" }));
      await waitFor(() => expect(screen.getByRole("radiogroup", { name: "Tile size" }).querySelectorAll("button")).toHaveLength(2));
    });

    it("shows a saved L as M by month, without changing the setting", async () => {
      mockCommand("get_settings", () => withGroup("month", "l"));
      mockLibraryDefaults();
      mockCommand("get_library_stats", () => ({ items: 12, videos: 1, bytes: 1000 }));
      mount();
      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith("get_layout", expect.objectContaining({ groupBy: "month", rowHeight: 200 })),
      );
      expect(screen.getByRole("radio", { name: "M" })).toHaveAttribute("aria-checked", "true");
    });

    it("lays a year out in small squares, with no size to choose", async () => {
      mockCommand("get_settings", () => withGroup("year"));
      mockLibraryDefaults();
      mockCommand("get_library_stats", () => ({ items: 12, videos: 1, bytes: 1000 }));
      mount();
      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith("get_layout", expect.objectContaining({ groupBy: "year", rowHeight: YEAR_TILE })),
      );
      expect(screen.queryByRole("radiogroup", { name: "Tile size" })).toBeNull();
    });

    it("opens the month a photo belongs to when it is clicked in the year view", async () => {
      mockCommand("get_settings", () => withGroup("year"));
      mockLibraryDefaults();
      mockCommand("get_library_stats", () => ({ items: 12, videos: 1, bytes: 1000 }));
      const tile = { id: 7, x: 0, w: 56, h: 56, thumbhash: null, color: null, mediaType: 0, durationMs: null, renderable: true, extension: "jpg", isFavorite: false, trashedAt: null, rating: 0, hdr: 0, raw: false };
      mockCommand("get_layout", () => ({
        totalHeight: 100,
        itemCount: 1,
        rowHeights: [44, 56],
        sections: [{ index: 0, year: 2025, title: "2025", y: 0, height: 100, firstRow: 0, rowCount: 2, itemCount: 1 }],
      }));
      mockCommand("get_layout_rows", () => [
        { index: 0, kind: "header", y: 0, height: 44, section: 0, title: "2025", tiles: [] },
        { index: 1, kind: "tiles", y: 44, height: 56, section: 0, title: null, tiles: [tile] },
      ]);
      mockCommand("find_layout_row", () => 1);
      mount();
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_layout_rows", expect.objectContaining({ groupBy: "year" })));
      await userEvent.click(await screen.findByRole("button", { name: "Item 7" }));
      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith("find_layout_row", expect.objectContaining({ assetId: 7, groupBy: "month" })),
      );
      expect(screen.getByRole("radio", { name: "Months" })).toHaveAttribute("aria-checked", "true");
    });
  });
});
