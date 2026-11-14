import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { ExportFinished } from "@/types/generated/ExportFinished";
import { selectAll, selectionStore } from "@/hooks/use-selection";
import { ExportLosses, ExportSheet, exportFinishedMessage, selectionLabel } from "./gui";

function mount(ids: number[], summary = { photos: ids.length, videos: 0, trimmed: 0, edited: 0 }, setup?: () => void) {
  mockCommand("get_settings", () => null);
  mockLibraryDefaults();
  setup?.();
  mockCommand("export_assets", () => 1);
  mockCommand("export_summary", () => summary);
  const onClose = vi.fn();
  render(
    <Providers>
      <ExportSheet ids={ids} onClose={onClose} />
    </Providers>,
  );
  return onClose;
}

const sent = () => invokeMock.mock.calls.filter(([c]) => c === "export_assets").map(([, a]) => a as { ids: number[]; options: Record<string, unknown> })[0];

describe("ExportSheet", () => {
  beforeEach(() => {
    try {
      localStorage.clear();
    } catch {
      // jsdom always has storage; a guard for the rule, not the test.
    }
    vi.mocked(openDialog).mockResolvedValue("C:/Out");
  });

  // Round 6: every export asks where to save; nothing goes into the library's own folder.
  it("asks where to save, opening on the folder chosen last time", async () => {
    mount([1, 2]);
    await userEvent.click(await screen.findByRole("button", { name: "Export 2 photos" }));
    await waitFor(() => expect(sent()?.options.destination).toEqual({ kind: "folder", dir: "C:/Out" }));
    expect(openDialog).toHaveBeenLastCalledWith(expect.objectContaining({ directory: true, defaultPath: undefined }));
    mount([3]);
    await userEvent.click((await screen.findAllByRole("button", { name: /^Export 1/ })).at(-1)!);
    await waitFor(() => expect(openDialog).toHaveBeenLastCalledWith(expect.objectContaining({ defaultPath: "C:/Out" })));
  });

  it("exports nothing when the folder dialog is cancelled", async () => {
    vi.mocked(openDialog).mockResolvedValue(null);
    const onClose = mount([1, 2]);
    await userEvent.click(await screen.findByRole("button", { name: "Export 2 photos" }));
    await waitFor(() => expect(openDialog).toHaveBeenCalled());
    expect(sent()).toBeUndefined();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("exports with the chosen format and size, keeping metadata by default", async () => {
    const onClose = mount([1, 2]);
    await userEvent.click(await screen.findByRole("radio", { name: "PNG" }));
    await userEvent.click(screen.getByRole("radio", { name: "2048 px" }));
    await userEvent.click(await screen.findByRole("button", { name: "Export 2 photos" }));
    expect(sent()).toEqual({
      ids: [1, 2],
      options: { format: "png", quality: 90, lossless: false, maxLongEdge: 2048, metadata: true, destination: { kind: "folder", dir: "C:/Out" }, videoCut: "precise" },
    });
    expect(onClose).toHaveBeenCalled();
  });

  it("clears the selection once the export has started", async () => {
    selectAll([1, 2]);
    mount([1, 2]);
    await userEvent.click(await screen.findByRole("button", { name: "Export 2 photos" }));
    await waitFor(() => expect(selectionStore.get().ids.size).toBe(0));
  });

  it("exporting the viewer's one photo leaves the grid's selection alone", async () => {
    selectAll([1, 2]);
    mount([5]);
    await userEvent.click(await screen.findByRole("button", { name: /^Export/ }));
    await waitFor(() => expect(sent()?.ids).toEqual([5]));
    expect(selectionStore.get().ids.size).toBe(2);
  });

  it("keeps the selection when the export cannot start", async () => {
    selectAll([1, 2]);
    mount([1, 2], undefined, () => undefined);
    mockCommand("export_assets", () => {
      throw { code: "x", message: "No room." };
    });
    await userEvent.click(await screen.findByRole("button", { name: "Export 2 photos" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("export_assets", expect.anything()));
    expect(selectionStore.get().ids.size).toBe(2);
  });

  it("offers Lossless for WebP, which hides the quality", async () => {
    mount([1]);
    await userEvent.click(await screen.findByRole("radio", { name: "WebP" }));
    expect(screen.getByRole("slider", { name: "Quality" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("switch", { name: /Lossless/ }));
    expect(screen.queryByRole("slider", { name: "Quality" })).toBeNull();
    await userEvent.click(await screen.findByRole("button", { name: "Export 1 photo" }));
    expect(sent().options).toMatchObject({ format: "webp", lossless: true });
  });

  it("adds to the library, and can leave metadata out", async () => {
    mount([4, 5, 6]);
    await userEvent.click(await screen.findByRole("radio", { name: "Add to library" }));
    await userEvent.click(screen.getByRole("switch", { name: /Keep metadata/ }));
    await userEvent.click(await screen.findByRole("button", { name: "Add 3 photos to library" }));
    expect(sent().options).toMatchObject({ destination: { kind: "library" }, metadata: false });
  });

  // Round 5: export straight into an album, an existing one or one made on the spot.
  describe("into an album", () => {
    const album = (id: number, name: string, kind = 0) => ({ id, name, kind, parentId: null, coverAssetId: null, itemCount: 0, createdAt: 1, updatedAt: 1, query: null });
    const withAlbums = () => mockCommand("list_albums", () => [album(3, "Trip"), album(4, "Best of", 1)]);

    it("adds the exports to a chosen album", async () => {
      mount([4, 5], undefined, withAlbums);
      await userEvent.click(await screen.findByRole("radio", { name: "Add to album" }));
      const button = screen.getByRole("button", { name: "Add 2 photos to album" });
      expect(button).toBeDisabled();
      await userEvent.click(screen.getByPlaceholderText("Choose an album"));
      // Only albums photos can be put in: a smart album fills itself.
      expect(screen.queryByRole("option", { name: "Best of" })).toBeNull();
      await userEvent.click(await screen.findByRole("option", { name: "Trip" }));
      await userEvent.click(button);
      expect(sent().options).toMatchObject({ destination: { kind: "library", albumId: 3 } });
    });

    it("makes a new album first when asked", async () => {
      mount([4], undefined, withAlbums);
      await userEvent.click(await screen.findByRole("radio", { name: "Add to album" }));
      await userEvent.click(screen.getByPlaceholderText("Choose an album"));
      await userEvent.click(await screen.findByRole("option", { name: "New album…" }));
      await userEvent.type(screen.getByRole("textbox", { name: "New album name" }), "Edits");
      await userEvent.click(screen.getByRole("button", { name: "Add 1 photo to album" }));
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("create_album", { name: "Edits", parentId: null }));
      await waitFor(() => expect(sent().options).toMatchObject({ destination: { kind: "library", albumId: 1 } }));
    });
  });

  it("asks how to cut trimmed videos only when there are some", async () => {
    mount([1, 2], { photos: 1, videos: 1, trimmed: 1, edited: 0 });
    await userEvent.click(await screen.findByRole("radio", { name: "Fast" }));
    expect(screen.getByText(/nearest key frame/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Export 1 photo and 1 video" }));
    expect(sent().options).toMatchObject({ videoCut: "fast" });
  });

  it("disables AVIF and HEIC without FFmpeg, and metadata without ExifTool", async () => {
    mount([1], undefined, () => {
      mockCommand("get_ffmpeg_status", () => ({ available: false, version: null, path: null, envVar: "LUMEN_FFMPEG_DIR" }));
      mockCommand("get_exiftool_status", () => ({ available: false, version: null, path: null, envVar: "LUMEN_EXIFTOOL_DIR" }));
    });
    await waitFor(() => expect(screen.getByRole("radio", { name: "AVIF" })).toBeDisabled());
    expect(screen.getByRole("radio", { name: "HEIC" })).toBeDisabled();
    await waitFor(() => expect(screen.getByRole("switch", { name: /Keep metadata/ })).toBeDisabled());
    expect(screen.getByText(/Install ExifTool/)).toBeInTheDocument();
  });

  it("explains what Original means for an edited photo", async () => {
    mount([1]);
    await userEvent.click(await screen.findByRole("radio", { name: "Original" }));
    expect(screen.getByText(/Edited photos are exported as JPEG/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Export 1 photo" })).toBeEnabled();
  });
});

const finished = (over: Partial<ExportFinished>): ExportFinished => ({ batch: 1, done: 2, failed: 0, dir: "x", metadata: "skipped", losses: [], library: null, ...over });

describe("exportFinishedMessage", () => {
  it("counts what went out and what did not", () => {
    expect(exportFinishedMessage(finished({}))).toBe("Exported 2 photos");
    expect(exportFinishedMessage(finished({ done: 1 }))).toBe("Exported 1 photo");
    expect(exportFinishedMessage(finished({ done: 3, failed: 2 }))).toBe("Exported 3 photos, 2 could not be exported");
  });

  it("says what happened to the metadata", () => {
    expect(exportFinishedMessage(finished({ done: 12, metadata: "verified" }))).toBe("Exported 12 photos · metadata verified");
    expect(exportFinishedMessage(finished({ metadata: "unavailable" }))).toMatch(/metadata not carried/);
    const losses = [{ assetId: 1, file: "a.webp", lost: [{ tag: "GPSLatitude", before: "46.05", after: "" }] }];
    expect(exportFinishedMessage(finished({ metadata: "verified", losses }))).toMatch(/some metadata did not carry over/);
  });

  it("counts what the library gained", () => {
    expect(exportFinishedMessage(finished({ dir: "", library: { added: 3, duplicates: 0 } }))).toBe("Added 3 to the library");
    expect(exportFinishedMessage(finished({ dir: "", library: { added: 2, duplicates: 1 } }))).toBe("Added 2 to the library, 1 was already there");
    expect(exportFinishedMessage(finished({ dir: "", library: { added: 2, duplicates: 1, album: "Trip" } }))).toBe("Added 3 to “Trip”, 1 was already in the library");
  });
});

describe("selectionLabel", () => {
  it("names photos and videos", () => {
    expect(selectionLabel(2, undefined)).toBe("2 photos");
    expect(selectionLabel(3, { photos: 0, videos: 3, trimmed: 0, edited: 0 })).toBe("3 videos");
    expect(selectionLabel(2, { photos: 1, videos: 1, trimmed: 0, edited: 0 })).toBe("1 photo and 1 video");
  });
});

describe("ExportLosses", () => {
  it("lists each file and what it lost", () => {
    render(
      <Providers>
        <ExportLosses losses={[{ assetId: 1, file: "DSC_1.avif", lost: [{ tag: "LensModel", before: "NIKKOR Z 24-70mm f/4 S", after: "" }] }]} />
      </Providers>,
    );
    expect(screen.getByText("DSC_1.avif")).toBeInTheDocument();
    expect(screen.getByText("LensModel")).toBeInTheDocument();
    expect(screen.getByText("NIKKOR Z 24-70mm f/4 S")).toBeInTheDocument();
    expect(screen.getByText("missing")).toBeInTheDocument();
  });
});
