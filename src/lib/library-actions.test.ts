import { QueryClient } from "@tanstack/react-query";
import { act, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { exportSheetStore } from "@/components/export-sheet";
import { toastStore } from "@/components/toast/store";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { collectionPickerStore } from "@/components/collection-picker";
import { backUpNow, duplicateAlbum, exportAlbum, moveAlbumToCollection, openCollection, openId, setMemoryHidden, shuffleAlbumCover } from "./library-actions";

const lastToast = () => toastStore.get().toasts.at(-1);

describe("library actions shared by menus and the palette", () => {
  it("reads the open object's id from the path", () => {
    expect(openId("albums", "/albums/12")).toBe(12);
    expect(openId("albums", "/albums/12/")).toBe(12);
    expect(openId("albums", "/albums")).toBeNull();
    expect(openId("memories", "/albums/12")).toBeNull();
  });

  it("reads the open collection from the Albums page's address (Round 5A)", () => {
    expect(openCollection({ pathname: "/albums", search: "?c=3" })).toBe(3);
    expect(openCollection({ pathname: "/albums", search: "" })).toBeNull();
    expect(openCollection({ pathname: "/albums/3", search: "?c=3" })).toBeNull();
  });

  it("opens the collection picker on the album open now", async () => {
    mockCommand("get_album", () => ({ id: 4, name: "Lisbon", collectionId: 2 }));
    await moveAlbumToCollection(4);
    expect(collectionPickerStore.get().album).toEqual({ id: 4, name: "Lisbon", collectionId: 2 });
  });

  it("shuffles an album's cover and refreshes the albums", async () => {
    mockCommand("shuffle_album_cover", () => null);
    await shuffleAlbumCover(new QueryClient(), 4);
    expect(invokeMock.mock.calls.some(([cmd, args]) => cmd === "shuffle_album_cover" && (args as { id: number }).id === 4)).toBe(true);
    mockCommand("shuffle_album_cover", () => {
      throw { code: "not_found", message: "That album is gone." };
    });
    await shuffleAlbumCover(new QueryClient(), 4);
    expect(lastToast()?.message).toBe("That album is gone.");
  });

  it("backs up now, or says where to choose a destination first", async () => {
    mockCommand("start_backup", () => null);
    await backUpNow(new QueryClient());
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "start_backup")).toBe(true);
    expect(lastToast()?.message).toBe("Backing up…");
    mockCommand("start_backup", () => {
      throw { code: "backup_no_destination", message: "Choose where to back up first." };
    });
    await backUpNow(new QueryClient());
    expect(lastToast()?.message).toBe("Choose where to back up first.");
  });

  it("duplicates an album and undoes it by deleting the copy", async () => {
    mockCommand("duplicate_album", () => ({ id: 30, name: "Lisbon copy" }));
    mockCommand("delete_album", () => null);
    await duplicateAlbum(new QueryClient(), 4);
    expect(invokeMock).toHaveBeenCalledWith("duplicate_album", { id: 4 });
    expect(lastToast()?.message).toBe("Made “Lisbon copy”");
    act(() => lastToast()?.action?.run());
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("delete_album", { id: 30 }));
  });

  it("exports an album through the export sheet", async () => {
    mockCommand("scope_ids", () => [5, 6]);
    await exportAlbum(4);
    expect(invokeMock).toHaveBeenCalledWith("scope_ids", { scope: expect.objectContaining({ albumId: 4 }) });
    expect(exportSheetStore.get().ids).toEqual([5, 6]);
  });

  it("fails softly when the memory has gone", async () => {
    mockCommand("set_memory_hidden", () => {
      throw { code: "not_found", message: "That memory is gone." };
    });
    await expect(setMemoryHidden(new QueryClient(), { id: 9 }, true)).resolves.toBeUndefined();
    expect(lastToast()?.message).toBe("That memory is gone.");
  });
});
