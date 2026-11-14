import { EMPTY_QUERY } from "@/hooks/use-search";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ConfirmDialogHost } from "@/components/confirm-dialog";
import { ContextMenuHost } from "@/components/context-menu";
import { toastStore } from "@/components/toast/store";
import { Providers } from "@/app";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { AlbumsRoute } from "./gui";

const smartQuery = { text: null, dateFrom: null, dateTo: null, deviceIds: [], mediaType: 1, favorites: false, minRating: 4, hasGps: null, camera: null, lens: null };

const albums = [
  { id: 1, name: "Trip", kind: 0, parentId: null, coverAssetId: 4, itemCount: 12, createdAt: 1, updatedAt: 2, query: null, hidesItems: false, coverStyle: null, previewIds: [4, 6, 7] },
  { id: 2, name: "Family", kind: 0, parentId: null, coverAssetId: null, itemCount: 0, createdAt: 1, updatedAt: 1, query: null, hidesItems: false, coverStyle: null, previewIds: [] },
  { id: 3, name: "Best videos", kind: 1, parentId: null, coverAssetId: 9, itemCount: 7, createdAt: 1, updatedAt: 3, query: smartQuery, hidesItems: false, coverStyle: null, previewIds: [9] },
  { id: 4, name: "School", kind: 0, parentId: null, coverAssetId: 5, itemCount: 30, createdAt: 1, updatedAt: 4, query: null, hidesItems: true, coverStyle: { kind: "icon", shape: "box", icon: "school", color: "#12ab34" }, previewIds: [5] },
];

function mount(path: string) {
  render(
    <Providers>
      <MemoryRouter initialEntries={[path]}>
        <Routes>
          <Route path="/albums" element={<AlbumsRoute />} />
          <Route path="/albums/:id" element={<AlbumsRoute />} />
        </Routes>
      </MemoryRouter>
      <ConfirmDialogHost />
      <ContextMenuHost />
    </Providers>,
  );
}

describe("AlbumsRoute", () => {
  const props = ["clientWidth", "clientHeight", "offsetWidth", "offsetHeight"] as const;
  beforeEach(() => {
    for (const p of props) Object.defineProperty(HTMLElement.prototype, p, { configurable: true, get: () => (p.endsWith("Width") ? 1000 : 800) });
  });
  afterEach(() => {
    for (const p of props) delete (HTMLElement.prototype as unknown as Record<string, unknown>)[p];
  });

  it("lists the collections above the albums and creates a new album", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_albums", () => albums);
    const q = { ...EMPTY_QUERY };
    mockCommand("list_collections", () => [
      { id: "favorites", query: { ...q, favorites: true }, count: 0, coverAssetId: null },
      { id: "raw", query: { ...q, kind: "raw" }, count: 14, coverAssetId: 3 },
      { id: "hdr", query: { ...q, kind: "hdr" }, count: 0, coverAssetId: null },
    ]);
    mount("/albums");
    expect(await screen.findByText("Trip")).toBeInTheDocument();
    expect(await screen.findByText("Favorites")).toBeInTheDocument();
    expect(screen.getByText("Everything you star")).toBeInTheDocument();
    expect(screen.getByText("14 items")).toBeInTheDocument();
    expect(screen.getByText("RAW").closest("a")).toHaveAttribute("href", `/library?tags=${encodeURIComponent('[{"label":"RAW","value":{"type":"kind","kind":"raw"}}]')}`);
    expect(screen.queryByText("HDR"), "an empty collection other than Favorites has no card").toBeNull();
    expect(screen.getByText("12 items")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "New album" }));
    await userEvent.type(await screen.findByRole("textbox", { name: "Album name" }), "Summer");
    await userEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("create_album", { name: "Summer", parentId: null }));
  });

  it("creates a smart album from the editor's rules", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_albums", () => albums);
    mockCommand("count_scope", () => 7);
    mockCommand("scope_ids", () => []);
    mockCommand("search_overview", () => ({ chips: [], rest: "clip", total: 7, people: [], pets: [], places: [], albums: [], memories: [] }));
    mount("/albums");
    await screen.findByText("Trip");
    await userEvent.click(screen.getByRole("button", { name: "More ways to create" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "New smart album" }));
    await userEvent.type(await screen.findByRole("textbox", { name: "Album name" }), "Recent videos");
    await userEvent.click(screen.getByRole("button", { name: "Kind" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Videos" }));
    expect(await screen.findByText("7 match")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Create album" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "create_smart_album",
        expect.objectContaining({ name: "Recent videos", query: expect.objectContaining({ text: null, tagExpr: expect.objectContaining({ items: [expect.objectContaining({ type: "tag" })] }) }) }),
      ),
    );
  });

  it("renders a smart album from its query, not from album membership", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_album", () => albums[2]);
    mount("/albums/3");
    expect(await screen.findByRole("button", { name: "Best videos" })).toBeInTheDocument();
    expect(screen.getByText("Updates itself")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add photos" })).toBeNull();
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "get_layout",
        expect.objectContaining({ scope: expect.objectContaining({ query: expect.objectContaining({ minRating: 4 }), albumId: null }) }),
      ),
    );
  });

  it("shows an album's grid in album scope", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_album", () => albums[0]);
    mount("/albums/1");
    expect(await screen.findByRole("button", { name: "Trip" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "All albums" }), "a way back sits before the title").toHaveAttribute("href", "/albums");
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("get_layout", expect.objectContaining({ scope: expect.objectContaining({ albumId: 1, sortByDate: false }) })),
    );
  });

  it("creates an album that hides its photos from the Library", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_albums", () => albums);
    mockCommand("list_collections", () => []);
    mockCommand("create_album", () => ({ ...albums[1], id: 9, name: "Homework" }));
    mockCommand("set_album_hides", () => null);
    mount("/albums");
    await userEvent.click(await screen.findByRole("button", { name: "New album" }));
    await userEvent.type(await screen.findByRole("textbox", { name: "Album name" }), "Homework");
    await userEvent.click(screen.getByRole("switch", { name: /Hide these photos from the Library/ }));
    await userEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_album_hides", { id: 9, hides: true }));
  });

  it("marks a hiding album and turns hiding off from its menu, with undo", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_albums", () => albums);
    mockCommand("list_collections", () => []);
    mockCommand("set_album_hides", () => null);
    render(
      <Providers>
        <MemoryRouter initialEntries={["/albums"]}>
          <Routes>
            <Route path="/albums" element={<AlbumsRoute />} />
          </Routes>
        </MemoryRouter>
        <ContextMenuHost />
      </Providers>,
    );
    const card = (await screen.findByText("School")).closest("a")!;
    expect(card.querySelector("[title='Hidden from the Library']")).not.toBeNull();
    await userEvent.pointer({ keys: "[MouseRight]", target: card });
    await userEvent.click(await screen.findByRole("menuitem", { name: "Show its photos in the Library" }, { timeout: 5000 }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_album_hides", { id: 4, hides: false }));
    const t = toastStore.get().toasts.at(-1)!;
    expect(t.message).toBe("“School” is back in the Library");
    t.action?.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_album_hides", { id: 4, hides: true }));
  });

  it("toggles hiding from the album's own page", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_album", () => albums[3]);
    mockCommand("set_album_hides", () => null);
    mount("/albums/4");
    const toggle = await screen.findByRole("button", { name: "Hidden from the Library" });
    expect(toggle).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(toggle);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_album_hides", { id: 4, hides: false }));
  });

  it("offers the order and the hiding switch in a ⋯ menu for when the header is narrow", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_album", () => albums[3]);
    mockCommand("set_album_hides", () => null);
    mount("/albums/4");
    await userEvent.click(await screen.findByRole("button", { name: "More actions" }));
    expect(await screen.findByRole("menuitem", { name: "Sort by date" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("menuitem", { name: "Show its photos in the Library" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_album_hides", { id: 4, hides: false }));
  });

  it("has no ⋯ menu on a smart album, which has no actions to fold", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_album", () => albums[2]);
    mount("/albums/3");
    await screen.findByRole("button", { name: "Best videos" });
    expect(screen.queryByRole("button", { name: "More actions" })).toBeNull();
  });

  describe("collections", () => {
    // Trips (1) ⊃ Portugal (2); Trip and Family sit in Trips, School in Portugal.
    const inCollections = [
      { ...albums[0], collectionId: 1 },
      { ...albums[1], collectionId: 1 },
      { ...albums[2], collectionId: null },
      { ...albums[3], collectionId: 2 },
    ];
    const collections = [
      { id: 1, name: "Trips", parentId: null, createdAt: 1, updatedAt: 1, coverStyle: null },
      { id: 2, name: "Portugal", parentId: 1, createdAt: 1, updatedAt: 1, coverStyle: { kind: "icon", shape: "folder", icon: "plane", color: "swatch-6" } },
    ];
    const setUp = () => {
      mockCommand("get_settings", () => null);
      mockLibraryDefaults();
      mockCommand("list_albums", () => inCollections);
      mockCommand("list_album_collections", () => collections);
      mockCommand("list_collections", () => [{ id: "raw", query: { ...EMPTY_QUERY, kind: "raw" }, count: 14, coverAssetId: 3 }]);
    };

    it("shows collections first at the top, and an album only where it sits", async () => {
      setUp();
      mount("/albums");
      expect(await screen.findByText("Best videos")).toBeInTheDocument();
      expect(screen.getByRole("heading", { name: "Media types" })).toBeInTheDocument();
      const trips = screen.getByText("Trips").closest("a")!;
      expect(trips).toHaveAttribute("href", "/albums?c=1");
      expect(screen.getByText("3 albums"), "Trip, Family and School further in").toBeInTheDocument();
      expect(screen.queryByText("Trip"), "an album in a collection is shown inside it").toBeNull();
      expect(screen.queryByText("Portugal")).toBeNull();
    });

    it("opens a collection with its path, its collections and its albums", async () => {
      setUp();
      mount("/albums?c=1");
      expect(await screen.findByText("Trip")).toBeInTheDocument();
      expect(screen.getByText("Family")).toBeInTheDocument();
      expect(screen.getByText("Portugal")).toBeInTheDocument();
      expect(screen.queryByText("Best videos")).toBeNull();
      expect(screen.queryByRole("heading", { name: "Media types" })).toBeNull();
      expect(screen.getByRole("link", { name: "All albums" })).toHaveAttribute("href", "/albums");
    });

    // Julian's 2026-11-08 list, items 1 and 3: fanned prints, cover on top, or an icon cover.
    it("fans an album's photos as prints and shows icon covers in their colour", async () => {
      setUp();
      mount("/albums?c=1");
      const trip = (await screen.findByText("Trip")).closest("a")!;
      const prints = trip.querySelectorAll(".cover-stack__print");
      expect(prints).toHaveLength(3);
      expect(prints[0]).toHaveAttribute("data-pos", "front");
      expect(prints[0].querySelector("img")!.getAttribute("src")).toContain("/4");
      const portugal = screen.getByText("Portugal").closest("a")!;
      const icon = portugal.querySelector(".icon-cover")!;
      expect(icon).toHaveAttribute("data-shape", "folder");
      expect((icon as HTMLElement).style.getPropertyValue("--cover-color")).toBe("var(--cover-swatch-6)");
      expect(screen.getByText("Family").closest("a")!.querySelector(".cover-stack, .icon-cover"), "no photos, no prints").toBeNull();
    });

    // Round 19 §10: a collection's photos in one grid, a section per album; a header opens its album.
    it("opens a collection's photos by album, and a section header opens that album", async () => {
      setUp();
      mockCommand("get_album", () => albums[0]);
      const tile = { id: 7, x: 0, w: 300, h: 200, thumbhash: null, color: null, mediaType: 0, durationMs: null, renderable: true, extension: "jpg", isFavorite: false, trashedAt: null, rating: 0, hdr: 0, raw: false, colorLabel: 0, flag: 0 };
      const section = (index: number, title: string, albumId: number, firstRow: number) => ({ index, year: 0, month: 1, title, y: index * 250, height: 250, firstRow, rowCount: 2, itemCount: 1, albumId });
      mockCommand("get_layout", () => ({ totalHeight: 500, itemCount: 2, rowHeights: [44, 200, 44, 200], sections: [section(0, "Trip", 1, 0), section(1, "Family · in Portugal", 2, 2)] }));
      mockCommand("get_layout_rows", () => [
        { index: 0, kind: "header", y: 0, height: 44, section: 0, title: "Trip", tiles: [] },
        { index: 1, kind: "tiles", y: 44, height: 200, section: 0, title: null, tiles: [tile] },
        { index: 2, kind: "header", y: 250, height: 44, section: 1, title: "Family · in Portugal", tiles: [] },
        { index: 3, kind: "tiles", y: 294, height: 200, section: 1, title: null, tiles: [tile] },
      ]);
      mount("/albums?c=1");
      expect(await screen.findByRole("link", { name: "All photos" })).toHaveAttribute("href", "/albums?c=1&all=1");
      cleanup();
      mount("/albums?c=1&all=1");
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_layout", expect.objectContaining({ scope: expect.objectContaining({ collectionId: 1, albumId: null }) })));
      expect(await screen.findByRole("button", { name: "Family · in Portugal" })).toBeInTheDocument();
      await userEvent.click(screen.getByRole("button", { name: "Trip" }));
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_album", { id: 1 }));
    });

    it("shows a custom colour as it was picked", async () => {
      setUp();
      mount("/albums?c=2");
      const school = (await screen.findByText("School")).closest("a")!;
      const icon = school.querySelector(".icon-cover") as HTMLElement;
      expect(icon).toHaveAttribute("data-shape", "box");
      expect(icon.style.getPropertyValue("--cover-color")).toBe("#12ab34");
    });

    it("shows the whole way down to a nested collection", async () => {
      setUp();
      mount("/albums?c=2");
      expect(await screen.findByText("School")).toBeInTheDocument();
      const path = screen.getByRole("navigation", { name: "Collection path" });
      expect(path).toHaveTextContent("AlbumsTripsPortugal");
      expect(within(path).getByRole("link", { name: "Trips" })).toHaveAttribute("href", "/albums?c=1");
    });

    it("makes a new album inside the open collection", async () => {
      setUp();
      mockCommand("create_album", () => ({ ...albums[1], id: 9, name: "Porto", collectionId: null }));
      mockCommand("move_album_to_collection", () => null);
      mount("/albums?c=2");
      await screen.findByText("School");
      await userEvent.click(screen.getByRole("button", { name: "New album" }));
      await userEvent.type(await screen.findByRole("textbox", { name: "Album name" }), "Porto");
      await userEvent.click(screen.getByRole("button", { name: "Create" }));
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("move_album_to_collection", { albumId: 9, collectionId: 2 }));
    });

    it("makes a new collection where it is", async () => {
      setUp();
      mockCommand("create_album_collection", () => 3);
      mount("/albums?c=1");
      await screen.findByText("Trip");
      await userEvent.click(screen.getByRole("button", { name: "New collection" }));
      await userEvent.type(await screen.findByRole("textbox", { name: "Name" }), "Spain");
      await userEvent.click(screen.getByRole("button", { name: "Create" }));
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("create_album_collection", { name: "Spain", parentId: 1 }));
    });
  });
});
