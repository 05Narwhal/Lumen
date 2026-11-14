import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { open } from "@tauri-apps/plugin-dialog";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { ContextMenuHost } from "@/components/context-menu";
import { toastStore } from "@/components/toast/store";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { ImportRoute } from "./gui";

const findChoose = () => screen.findByRole("button", { name: "Choose…" });

/** The split button: its main half picks photos and videos, the chevron lists the other ways in. */
async function chooseFromMenu(name: string | RegExp) {
  await userEvent.click(await screen.findByRole("button", { name: "More ways to import" }));
  await userEvent.click(await screen.findByRole("menuitem", { name }, { timeout: 5000 }));
}

describe("ImportRoute", () => {
  // Round 11: a Lightroom Classic catalog Lumen found is offered, and only then.
  it("offers a found Lightroom catalog and says which originals are missing", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_import_batches", () => []);
    mockCommand("find_lightroom_catalogs", () => [{ path: "C:/Users/j/Pictures/Lightroom/Lightroom Catalog.lrcat", name: "Lightroom Catalog", photoCount: 1234, modified: 1 }]);
    mockCommand("start_lightroom_import", () => ({
      batchId: 4,
      summary: { name: "Lightroom Catalog", items: 1200, missing: ["E:/Photos/a.jpg", "E:/Photos/b.jpg"], virtualCopies: 3, smartCollections: ["Five stars"] },
    }));
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    render(
      <Providers>
        <MemoryRouter>
          <ImportRoute />
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByText("Lightroom Catalog")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: /Lightroom catalogs/ })).toBeInTheDocument();
    expect(screen.getByText("1,234 photos")).toBeInTheDocument();
    expect(document.querySelector('[data-lightroom-catalog="C:/Users/j/Pictures/Lightroom/Lightroom Catalog.lrcat"]')).not.toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Import Lightroom Catalog" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("start_lightroom_import", { path: "C:/Users/j/Pictures/Lightroom/Lightroom Catalog.lrcat" }));
    await waitFor(() => expect(toastStore.get().toasts.map((t) => t.message)).toContain("Importing 1,200 photos from Lightroom Catalog · 2 originals not found"));
    act(() => toastStore.get().toasts.find((t) => t.action?.label === "Copy list")?.action?.run());
    expect(writeText).toHaveBeenCalledWith("E:/Photos/a.jpg\nE:/Photos/b.jpg");
  });

  it("offers no Lightroom catalog when none is found", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_import_batches", () => []);
    render(
      <Providers>
        <MemoryRouter>
          <ImportRoute />
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByRole("button", { name: "Choose…" })).toBeInTheDocument();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("find_lightroom_catalogs", undefined));
    expect(screen.queryByRole("heading", { name: /Lightroom catalogs/ })).toBeNull();
  });

  it("lists batches and starts an import from the picked folder", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_import_batches", () => [
      { id: 2, sourceKind: 0, sourceLabel: "Screenshots", sourcePath: "C:/Pics/Screenshots", startedAt: 1, finishedAt: 2, itemCount: 80, skippedCount: 4, failedCount: 1, state: 1, skipCount: 0 },
      { id: 3, sourceKind: 0, sourceLabel: "DCIM", sourcePath: "D:/DCIM", startedAt: 5, finishedAt: null, itemCount: 0, skippedCount: 0, failedCount: 0, state: 0, skipCount: 0 },
    ]);
    mockCommand("get_import_failures", () => [{ path: "bad.jpg", reason: "decode failed" }]);
    mockCommand("start_folder_import", () => 9);
    render(
      <Providers>
        <MemoryRouter>
          <ImportRoute />
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByText("Screenshots")).toBeInTheDocument();
    expect(screen.getByRole("cell", { name: "80" })).toBeInTheDocument();
    expect(screen.getByText("4 skipped")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
    expect(screen.getAllByRole("columnheader").map((h) => h.textContent)).toEqual(["Source", "When", "Result", "Photos", ""]);
    await userEvent.click(screen.getByRole("button", { name: /1 failed/ }));
    expect(await screen.findByText("decode failed")).toBeInTheDocument();
    vi.mocked(open).mockResolvedValueOnce("C:/Users/j/Pictures/Trip");
    await chooseFromMenu("Folder…");
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("start_folder_import", { path: "C:/Users/j/Pictures/Trip", recursive: true, limitBytes: null }));
    vi.mocked(open).mockResolvedValueOnce("C:/Users/j/Pictures/Trips");
    await chooseFromMenu("Folder as albums…");
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("start_folder_import", { path: "C:/Users/j/Pictures/Trips", recursive: true, limitBytes: null, asAlbums: true }));
  });

  it("shows the ten newest imports and a button for the rest", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_import_batches", () =>
      Array.from({ length: 12 }, (_, i) => ({ id: 20 - i, sourceKind: 0, sourceLabel: `Batch ${20 - i}`, sourcePath: `D:/B${i}`, startedAt: 100 - i, finishedAt: 101 - i, itemCount: 1, skippedCount: 0, failedCount: 0, state: 1, skipCount: 0 })),
    );
    render(
      <Providers>
        <MemoryRouter>
          <ImportRoute />
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByText("Batch 20")).toBeInTheDocument();
    expect(screen.getAllByRole("row")).toHaveLength(11); // header + ten
    await userEvent.click(screen.getByRole("button", { name: "Show all 12" }));
    expect(screen.getAllByRole("row")).toHaveLength(13);
    await userEvent.click(screen.getByRole("button", { name: "Show fewer" }));
    expect(screen.getAllByRole("row")).toHaveLength(11);
  });

  it("lists the files an import left out, each with the way to find it", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_import_batches", () => [
      { id: 2, sourceKind: 0, sourceLabel: "Disk", sourcePath: "D:/Disk", startedAt: 1, finishedAt: 2, itemCount: 8, skippedCount: 1, failedCount: 0, state: 1, skipCount: 2 },
    ]);
    mockCommand("get_import_skips", () => [
      { path: "notes.txt", reason: "unsupported", assetId: null },
      { path: "sub/a-copy.jpg", reason: "duplicate", assetId: 7 },
    ]);
    render(
      <Providers>
        <MemoryRouter>
          <ImportRoute />
        </MemoryRouter>
      </Providers>,
    );
    await userEvent.click(await screen.findByRole("button", { name: /2 not imported/ }));
    expect(await screen.findByText("Not a photo or video")).toBeInTheDocument();
    expect(screen.getByText("Already in the library")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Show in folder" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Show library copy" }));
    expect(invokeMock).toHaveBeenCalledWith("reveal_asset", { id: 7 });
  });

  it("retries a folder import's failures and dismisses a row from its right-click menu", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_import_batches", () => [
      { id: 2, sourceKind: 0, sourceLabel: "Screenshots", sourcePath: "C:/Pics/Screenshots", startedAt: 1, finishedAt: 2, itemCount: 80, skippedCount: 0, failedCount: 1, state: 1, skipCount: 0 },
    ]);
    mockCommand("retry_import_failures", () => 12);
    render(
      <Providers>
        <MemoryRouter>
          <ImportRoute />
        </MemoryRouter>
        <ContextMenuHost />
      </Providers>,
    );
    const row = (await screen.findByText("Screenshots")).closest("tr")!;
    await userEvent.pointer({ keys: "[MouseRight]", target: row });
    expect(await screen.findByRole("menuitem", { name: "Show in folder" }, { timeout: 5000 })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("menuitem", { name: "Retry failed" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("retry_import_failures", { batchId: 2 }));

    await userEvent.pointer({ keys: "[MouseRight]", target: row });
    await userEvent.click(await screen.findByRole("menuitem", { name: "Dismiss" }, { timeout: 5000 }));
    await waitFor(() => expect(screen.queryByText("Screenshots")).toBeNull());
    act(() => toastStore.get().toasts.at(-1)?.action?.run());
    expect(await screen.findByText("Screenshots")).toBeInTheDocument();
  });

  it("imports a folder without its subfolders, or just the chosen files", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_import_batches", () => []);
    mockCommand("start_folder_import", () => 9);
    mockCommand("start_files_import", () => 10);
    render(
      <Providers>
        <MemoryRouter>
          <ImportRoute />
        </MemoryRouter>
      </Providers>,
    );
    await userEvent.click(await screen.findByRole("switch", { name: "Include subfolders" }));
    vi.mocked(open).mockResolvedValueOnce("D:/Trip");
    await chooseFromMenu("Folder…");
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("start_folder_import", { path: "D:/Trip", recursive: false, limitBytes: null }));

    vi.mocked(open).mockResolvedValueOnce(["D:/Trip/a.jpg", "E:/b.NEF"]);
    await userEvent.click(screen.getByRole("button", { name: "Choose…" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("start_files_import", { paths: ["D:/Trip/a.jpg", "E:/b.NEF"], limitBytes: null }));
    expect(vi.mocked(open).mock.calls.at(-1)?.[0]).toMatchObject({ multiple: true, filters: [expect.objectContaining({ extensions: expect.arrayContaining(["heic", "nef", "mov"]) })] });
  });

  it("lists memory cards with new-item counts and imports what is new", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_volumes", () => [{ mount: "E:\\", label: "EOS_DIGITAL", key: "vol:EOS_DIGITAL:64", totalItems: 312, newItems: 48, removable: true }]);
    render(
      <Providers>
        <MemoryRouter>
          <ImportRoute />
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByText("EOS_DIGITAL")).toBeInTheDocument();
    expect(screen.getByText("48 new of 312 items in DCIM")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Import 48 new" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("start_volume_import", { mount: "E:\\", limitBytes: null }));
  });

  it("lists a trusted iPhone and imports from it", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_iphones", () => ({
      usbmuxdAvailable: true,
      devices: [{ udid: "00008120-A", name: "Julian's iPhone", model: "iPhone16,1", paired: true }],
    }));
    render(
      <Providers>
        <MemoryRouter>
          <ImportRoute />
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByText("Julian's iPhone")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Import new photos" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("start_iphone_import", { udid: "00008120-A", limitBytes: null }));
  });

  it("explains a missing usbmuxd instead of showing an empty list", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_iphones", () => ({ usbmuxdAvailable: false, devices: [] }));
    render(
      <Providers>
        <MemoryRouter>
          <ImportRoute />
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByText(/iPhone import needs/)).toBeInTheDocument();
  });

  it("waits for Trust on an unpaired iPhone", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_iphones", () => ({
      usbmuxdAvailable: true,
      devices: [{ udid: "00008120-B", name: "iPhone", model: null, paired: false }],
    }));
    render(
      <Providers>
        <MemoryRouter>
          <ImportRoute />
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByRole("button", { name: "Waiting for Trust" })).toBeDisabled();
  });

  it("opens the page of an Apple Photos library picked from disk", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_import_batches", () => []);
    mockCommand("find_apple_photos_libraries", () => [{ path: "C:/Users/me/Pictures/Photos Library.photoslibrary", name: "Photos Library", disk: null, place: "Pictures" }]);
    function Where() {
      return <p>{useLocation().pathname}</p>;
    }
    render(
      <Providers>
        <MemoryRouter initialEntries={["/import"]}>
          <Routes>
            <Route path="/import" element={<ImportRoute />} />
            <Route path="/connected/:kind/:id" element={<Where />} />
          </Routes>
        </MemoryRouter>
      </Providers>,
    );
    // A Windows path, as the dialog returns it, trailing separator included.
    vi.mocked(open).mockResolvedValueOnce("D:\\Backup\\Old Mac.photoslibrary\\");
    await findChoose();
    await chooseFromMenu(/^Apple Photos library…/);
    expect(await screen.findByText(`/connected/photos/${encodeURIComponent("D:\\Backup\\Old Mac.photoslibrary")}`)).toBeInTheDocument();
  });

  it("refuses a folder that is not a Photos library", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_import_batches", () => []);
    mockCommand("find_apple_photos_libraries", () => [{ path: "C:/Users/me/Pictures/Photos Library.photoslibrary", name: "Photos Library", disk: null, place: "Pictures" }]);
    render(
      <Providers>
        <MemoryRouter>
          <ImportRoute />
        </MemoryRouter>
      </Providers>,
    );
    vi.mocked(open).mockResolvedValueOnce("D:/Backup/Pictures");
    toastStore.set({ toasts: [] });
    await findChoose();
    await chooseFromMenu(/^Apple Photos library…/);
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.message).toBe("That folder is not an Apple Photos library."));
  });

  it("lists Apple Photos libraries only when Lumen finds one (Round 9B)", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_import_batches", () => []);
    mockCommand("find_apple_photos_libraries", () => []);
    const { unmount } = render(
      <Providers>
        <MemoryRouter>
          <ImportRoute />
        </MemoryRouter>
      </Providers>,
    );
    await findChoose();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("find_apple_photos_libraries", undefined));
    expect(screen.queryByRole("heading", { name: /Apple Photos libraries/ })).not.toBeInTheDocument();
    unmount();

    mockCommand("find_apple_photos_libraries", () => [
      { path: "C:/Users/me/Pictures/Photos Library.photoslibrary", name: "Photos Library", disk: null, place: "Pictures" },
      { path: "/Users/me/Pictures/Family.photoslibrary", name: "Family", disk: "disk3-part2", place: "Macintosh HD" },
    ]);
    render(
      <Providers>
        <MemoryRouter>
          <ImportRoute />
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByRole("heading", { name: /Apple Photos libraries/ })).toBeInTheDocument();
    expect(screen.getByText("in Pictures")).toBeInTheDocument();
    expect(screen.getByText("on Macintosh HD")).toBeInTheDocument();
    const links = screen.getAllByRole("link", { name: "Show what is in it" }).map((a) => a.getAttribute("href"));
    expect(links).toEqual([`/connected/photos/${encodeURIComponent("C:/Users/me/Pictures/Photos Library.photoslibrary")}`, "/connected/mac/disk3-part2"]);
  });

  it("says what to plug in when no card or iPhone is found", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_import_batches", () => []);
    mockCommand("list_iphones", () => ({ usbmuxdAvailable: true, devices: [] }));
    render(
      <Providers>
        <MemoryRouter>
          <ImportRoute />
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByText("Plug in a memory card or an unlocked iPhone and it shows up here.")).toBeInTheDocument();
    expect(screen.getByText("Nothing imported yet.")).toBeInTheDocument();
  });

  it("shows three Lightroom catalogs and folds the rest behind Show more", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_import_batches", () => []);
    mockCommand("find_lightroom_catalogs", () =>
      ["A", "B", "C", "D"].map((n, i) => ({ path: `C:/Lr/${n}.lrcat`, name: n, photoCount: 10 + i, modified: 1 })),
    );
    render(
      <Providers>
        <MemoryRouter>
          <ImportRoute />
        </MemoryRouter>
      </Providers>,
    );
    await screen.findByText("A");
    expect(document.querySelectorAll("[data-lightroom-catalog]")).toHaveLength(3);
    expect(screen.queryByText("D")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Show 1 more" }));
    expect(document.querySelectorAll("[data-lightroom-catalog]")).toHaveLength(4);
    expect(screen.queryByRole("button", { name: /Show \d+ more/ })).toBeNull();
  });

  it("counts the found Apple Photos libraries in the picker menu", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_import_batches", () => []);
    mockCommand("find_apple_photos_libraries", () => [{ path: "C:/Users/me/Pictures/Photos Library.photoslibrary", name: "Photos Library", disk: null, place: "Pictures" }]);
    render(
      <Providers>
        <MemoryRouter>
          <ImportRoute />
        </MemoryRouter>
      </Providers>,
    );
    await userEvent.click(await screen.findByRole("button", { name: "More ways to import" }));
    expect(await screen.findByRole("menuitem", { name: /Apple Photos library…/ }, { timeout: 5000 })).toHaveTextContent("found 1");
    expect(screen.getByRole("menuitem", { name: "Lightroom catalog file…" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Photos and videos…" })).toBeInTheDocument();
  });
});
