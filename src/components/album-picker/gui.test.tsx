import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { selectAll, selectionStore } from "@/hooks/use-selection";
import { Toaster } from "@/components/toast";
import { AlbumPickerHost } from "./gui";
import { albumPickerStore, openAlbumPicker } from "./store";

describe("AlbumPickerHost", () => {
  it("adds the ids to the chosen album", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_albums", () => [
      { id: 4, name: "Trip", kind: 0, parentId: null, coverAssetId: null, itemCount: 3, createdAt: 1, updatedAt: 1 },
    ]);
    render(
      <Providers>
        <AlbumPickerHost />
      </Providers>,
    );
    act(() => selectAll([10, 11]));
    act(() => openAlbumPicker([10, 11]));
    await userEvent.click(await screen.findByRole("treeitem", { name: /Trip/ }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("add_to_album", { albumId: 4, assetIds: [10, 11] }));
    await waitFor(() => expect(selectionStore.get().ids.size).toBe(0));
  });

  it("keeps the selection when the picker is cancelled", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    render(
      <Providers>
        <AlbumPickerHost />
      </Providers>,
    );
    act(() => selectAll([10, 11]));
    act(() => openAlbumPicker([10, 11]));
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(albumPickerStore.get().ids).toBeNull());
    expect(selectionStore.get().ids.size).toBe(2);
  });

  it("creates a new album and adds to it", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    render(
      <Providers>
        <AlbumPickerHost />
      </Providers>,
    );
    act(() => openAlbumPicker([5]));
    await userEvent.click(await screen.findByRole("button", { name: "New album…" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Album name" }), "Summer");
    await userEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("create_album", { name: "Summer", parentId: null }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("add_to_album", { albumId: 1, assetIds: [5] }));
  });

  it("keeps the selection when adding fails, and says so", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_albums", () => [{ id: 4, name: "Trip", kind: 0, parentId: null, coverAssetId: null, itemCount: 3, createdAt: 1, updatedAt: 1 }]);
    mockCommand("add_to_album", () => {
      throw { code: "x", message: "The album could not be changed." };
    });
    render(
      <Providers>
        <AlbumPickerHost />
        <Toaster />
      </Providers>,
    );
    act(() => selectAll([10, 11]));
    act(() => openAlbumPicker([10, 11]));
    await userEvent.click(await screen.findByRole("treeitem", { name: /Trip/ }));
    expect(await screen.findByText("The album could not be changed.")).toBeInTheDocument();
    expect(selectionStore.get().ids.size).toBe(2);
  });

  it("adding the viewer's one photo leaves the grid's selection alone", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_albums", () => [{ id: 4, name: "Trip", kind: 0, parentId: null, coverAssetId: null, itemCount: 3, createdAt: 1, updatedAt: 1 }]);
    render(
      <Providers>
        <AlbumPickerHost />
      </Providers>,
    );
    act(() => selectAll([10, 11]));
    act(() => openAlbumPicker([5]));
    await userEvent.click(await screen.findByRole("treeitem", { name: /Trip/ }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("add_to_album", { albumId: 4, assetIds: [5] }));
    expect(selectionStore.get().ids.size).toBe(2);
  });

  it("filters to a flat list with paths as you type, and Enter adds to the first hit", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    const album = (id: number, name: string, collectionId: number | null) => ({ id, name, kind: 0, parentId: null, coverAssetId: null, itemCount: id, createdAt: 1, updatedAt: 1, collectionId, coverStyle: null, previewIds: [] });
    mockCommand("list_albums", () => [album(1, "Munich", 2), album(2, "Family", null), album(3, "Munich old", null)]);
    mockCommand("list_album_collections", () => [
      { id: 1, name: "Trips", parentId: null, createdAt: 1, updatedAt: 1, coverStyle: null },
      { id: 2, name: "2026", parentId: 1, createdAt: 1, updatedAt: 1, coverStyle: null },
    ]);
    render(
      <Providers>
        <AlbumPickerHost />
      </Providers>,
    );
    act(() => openAlbumPicker([7]));
    const box = await screen.findByRole("combobox", { name: "Search albums" });
    await userEvent.type(box, "mun");
    const items = await screen.findAllByRole("treeitem");
    // The two hits, then *Create album "mun"*; the collection path sits on a second line.
    expect(items).toHaveLength(3);
    expect(items[0]).toHaveTextContent("Munich");
    expect(items[0]).toHaveTextContent("Trips › 2026");
    expect(items[2]).toHaveTextContent("Create album “mun”");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("add_to_album", { albumId: 1, assetIds: [7] }));
  });
});
