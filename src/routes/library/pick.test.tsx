import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { pickAlbumId } from "@/lib/pick-mode";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { LayoutRow } from "@/types/generated/LayoutRow";
import { LibraryRoute } from "./gui";

const tile = (id: number, x: number) => ({ id, x, w: 300, h: 200, thumbhash: null, color: null, mediaType: 0, durationMs: null, renderable: true, extension: "jpg", isFavorite: false, trashedAt: null, rating: 0, hdr: 0, raw: false, colorLabel: 0, flag: 0 });
const rows: LayoutRow[] = [{ index: 0, kind: "tiles", y: 0, height: 200, section: 0, title: null, tiles: [tile(1, 0), tile(2, 304), tile(3, 608)] }];

const album = { id: 7, name: "Italy", kind: 0, parentId: null, collectionId: null, itemCount: 1, hidesItems: false, query: null };

describe("Library in pick mode", () => {
  const props = ["clientWidth", "clientHeight", "offsetWidth", "offsetHeight"] as const;
  beforeEach(() => {
    for (const p of props) Object.defineProperty(HTMLElement.prototype, p, { configurable: true, get: () => (p.endsWith("Width") ? 1000 : 800) });
  });
  afterEach(() => {
    for (const p of props) delete (HTMLElement.prototype as unknown as Record<string, unknown>)[p];
  });

  it("reads the album from ?pick=", () => {
    expect(pickAlbumId("?pick=album:7")).toBe(7);
    expect(pickAlbumId("pick=album:7&q=x")).toBe(7);
    expect(pickAlbumId("?pick=person:7")).toBeNull();
    expect(pickAlbumId("")).toBeNull();
  });

  it("toggles tiles, keeps photos already in the album locked, and Add N sends the picked ids", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_library_stats", () => ({ items: 3, videos: 0, bytes: 1000 }));
    mockCommand("get_album", () => album);
    mockCommand("list_albums", () => [album]);
    mockCommand("list_album_collections", () => []);
    mockCommand("list_people", () => []);
    mockCommand("list_pets", () => []);
    mockCommand("scope_ids", () => [3]);
    mockCommand("add_to_album", () => 2);
    mockCommand("remove_from_album", () => 2);
    mockCommand("get_layout", () => ({
      totalHeight: 200,
      itemCount: 3,
      rowHeights: [200],
      sections: [{ index: 0, year: 2026, title: "", y: 0, height: 200, firstRow: 0, rowCount: 1, itemCount: 3 }],
    }));
    mockCommand("get_layout_rows", (a) => rows.slice(Number(a?.start), Number(a?.start) + Number(a?.count)));
    render(
      <Providers>
        <MemoryRouter initialEntries={["/library?pick=album:7"]}>
          <Routes>
            <Route path="/library" element={<LibraryRoute />} />
            <Route path="/albums/:id" element={<p>Album page</p>} />
          </Routes>
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByText("Italy")).toBeInTheDocument();
    const one = await screen.findByRole("button", { name: "Item 1" });
    // Photo 3 is in the album already: checked and labelled once the album's ids arrive.
    await waitFor(() => expect(document.getElementById("tile-3")).toHaveAttribute("data-locked", "true"));
    expect(screen.getByText("In album")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add photos" })).toBeDisabled();

    await userEvent.click(one);
    await userEvent.click(screen.getByRole("button", { name: "Item 2" }));
    await userEvent.click(screen.getByRole("button", { name: "Item 3" })); // locked: no effect
    expect(await screen.findByRole("button", { name: "Add 2 photos" })).toBeEnabled();

    await userEvent.click(screen.getByRole("button", { name: "Add 2 photos" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("add_to_album", { albumId: 7, assetIds: [1, 2] }));
    expect(await screen.findByText("Album page")).toBeInTheDocument();
  });
});
