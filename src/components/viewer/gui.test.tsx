import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { describe, expect, it, onTestFinished, vi } from "vitest";
import { Providers } from "@/app";
import { ContextMenuHost } from "@/components/context-menu";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { clearViewerSequence, setViewerSequence } from "@/lib/viewer-sequence";
import { clearViewerScope, setViewerScope } from "@/lib/viewer-scope";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { servesOriginalForHdr, Viewer } from "./gui";

const asset = (id: number) => ({
  id,
  filename: `IMG_${id}.jpg`,
  extension: "jpg",
  mediaType: 0,
  byteSize: 1000,
  width: 4000,
  height: 3000,
  orientation: 1,
  capturedAt: 1_758_000_000,
  importedAt: 1_758_000_000,
  durationMs: null,
  deviceName: "Sony ILCE-7M4",
  gpsLat: null,
  gpsLon: null,
  locality: null,
  placeName: null,
  placeSource: 0,
  isFavorite: false,
  rating: 0,
  hdr: 0,
  relativePath: `2026/09/17/${id}.jpg`,
  contentHash: "abc",
  metadata: { lens: "FE 35mm" },
  renderable: true,
  raw: false,
  liveVideoId: null as number | null,
});

type Preload = { src: string; naturalWidth: number; naturalHeight: number; onload: (() => void) | null };

/** jsdom never loads an image: record the preloads and let the test finish them. */
function stubImages() {
  const loads: Preload[] = [];
  vi.stubGlobal(
    "Image",
    class {
      naturalWidth = 0;
      naturalHeight = 0;
      onload: (() => void) | null = null;
      onerror: (() => void) | null = null;
      src = "";
      constructor() {
        loads.push(this as never);
      }
    },
  );
  onTestFinished(() => void vi.unstubAllGlobals());
  return loads;
}

function mountViewer(url: string) {
  return render(
    <Providers>
      <MemoryRouter initialEntries={[url]}>
        <Routes>
          <Route path="*" element={<Viewer />} />
        </Routes>
      </MemoryRouter>
    </Providers>,
  );
}

function Probe() {
  return <p data-testid="search">{useLocation().search}</p>;
}

describe("Viewer", () => {
  it("pages only through the set it was opened on, and says where it is", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => asset(Number(a?.id)));
    mockCommand("get_asset_neighbors", () => ({ prev: 1, next: 99 }));
    setViewerSequence([21, 22, 23]);
    mountViewer("/library?asset=21");
    expect(await screen.findByText(/1 of 3/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Previous" })).toBeDisabled();
    await userEvent.keyboard("{ArrowRight}");
    expect(await screen.findByText("IMG_22.jpg")).toBeInTheDocument();
    await userEvent.keyboard("{ArrowRight}");
    expect(await screen.findByText("IMG_23.jpg")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
    expect(invokeMock).not.toHaveBeenCalledWith("get_asset_neighbors", expect.anything());
    clearViewerSequence();
  });

  it("opened from an album, pages inside the album and stops at its end", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => asset(Number(a?.id)));
    mockCommand("get_asset_neighbors", () => ({ prev: 1, next: 99 }));
    mockCommand("get_scope_neighbors", (a) => (Number(a?.id) === 5 ? { prev: 3, next: null } : { prev: null, next: 5 }));
    setViewerScope({ width: 1000, rowHeight: 200, gap: 4, scope: { ...LIBRARY_SCOPE, albumId: 7 }, groupBy: "month" });
    mountViewer("/albums/7?asset=5");
    expect(await screen.findByText("IMG_5.jpg")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("button", { name: "Next" })).toBeDisabled());
    expect(screen.getByRole("button", { name: "Previous" })).toBeEnabled();
    expect(invokeMock).toHaveBeenCalledWith("get_scope_neighbors", expect.objectContaining({ id: 5, scope: expect.objectContaining({ albumId: 7 }), width: 1000 }));
    expect(invokeMock).not.toHaveBeenCalledWith("get_asset_neighbors", expect.anything());
    await userEvent.keyboard("{ArrowLeft}");
    expect(await screen.findByText("IMG_3.jpg")).toBeInTheDocument();
    clearViewerScope();
  });

  it("falls back to the timeline when the photo is no longer in the grid's set", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => asset(Number(a?.id)));
    mockCommand("get_asset_neighbors", () => ({ prev: 1, next: 99 }));
    mockCommand("get_scope_neighbors", () => null);
    setViewerScope({ width: 1000, rowHeight: 200, gap: 4, scope: { ...LIBRARY_SCOPE, albumId: 7 }, groupBy: "month" });
    mountViewer("/albums/7?asset=5");
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_asset_neighbors", expect.objectContaining({ id: 5 })));
    await waitFor(() => expect(screen.getByRole("button", { name: "Next" })).toBeEnabled());
    clearViewerScope();
  });

  it("a set of the screen's own (the Map) wins over the grid's scope", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => asset(Number(a?.id)));
    mockCommand("get_scope_neighbors", () => ({ prev: 1, next: 2 }));
    setViewerScope({ width: 1000, rowHeight: 200, gap: 4, scope: LIBRARY_SCOPE, groupBy: "month" });
    setViewerSequence([21, 22]);
    mountViewer("/map?asset=21");
    expect(await screen.findByText(/1 of 2/)).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith("get_scope_neighbors", expect.anything());
    clearViewerSequence();
    clearViewerScope();
  });

  it("goes full screen, and Escape leaves full screen before it closes", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => asset(Number(a?.id)));
    mockCommand("get_asset_neighbors", () => ({ prev: null, next: null }));
    mountViewer("/library?asset=5");
    await userEvent.click(await screen.findByRole("button", { name: "Full screen" }));
    const viewer = screen.getByRole("dialog");
    expect(viewer).toHaveAttribute("data-fullscreen", "true");
    await userEvent.keyboard("{Escape}");
    expect(viewer).toHaveAttribute("data-fullscreen", "false");
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("leaves a video's full screen with Escape even from the seek bar, and hides its controls when idle (Round 8B)", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => ({ ...asset(Number(a?.id)), mediaType: 1, extension: "mp4" }));
    mockCommand("get_asset_neighbors", () => ({ prev: 4, next: 6 }));
    mockCommand("get_playback", () => ({ mode: "native", proxyReady: false, videoCodec: "h264", audioCodec: "aac", container: "mov,mp4", durationMs: 2000, width: 320, height: 240 }));
    mountViewer("/library?asset=5");
    await userEvent.click(await screen.findByRole("button", { name: "Full screen" }));
    const viewer = screen.getByRole("dialog");
    const seek = await screen.findByRole("slider", { name: "Seek" });
    act(() => seek.focus());
    // The seek bar keeps its own keys: → seeks, it never pages to the next photo.
    await userEvent.keyboard("{ArrowRight}");
    expect(invokeMock).not.toHaveBeenCalledWith("get_asset", { id: 6 });
    await userEvent.keyboard("{Escape}");
    expect(viewer).toHaveAttribute("data-fullscreen", "false");
    expect(screen.getByRole("dialog"), "the first Escape only leaves full screen").toBeInTheDocument();
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      act(() => {
        window.dispatchEvent(new PointerEvent("pointermove"));
        vi.advanceTimersByTime(2500);
      });
      expect(viewer).toHaveAttribute("data-idle", "true");
      expect(document.querySelector(".media-controls")).toHaveAttribute("data-hidden", "true");
    } finally {
      vi.useRealTimers();
    }
  });

  it("tags a RAW original and an HDR one beside the filename", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => ({ ...asset(Number(a?.id)), filename: "DSC_1.NEF", extension: "nef", raw: true, hdr: 2 }));
    mockCommand("get_asset_neighbors", () => ({ prev: null, next: null }));
    mountViewer("/library?asset=4");
    expect(await screen.findByText("DSC_1.NEF")).toBeInTheDocument();
    expect(screen.getByText("RAW")).toBeInTheDocument();
    expect(screen.getByText("HDR")).toBeInTheDocument();
  });

  it("plays a Live Photo's motion while the LIVE button is hovered", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => ({ ...asset(Number(a?.id)), extension: "heic", mediaType: 2, liveVideoId: 90 }));
    mockCommand("get_asset_neighbors", () => ({ prev: null, next: null }));
    mountViewer("/library?asset=9");
    const live = await screen.findByRole("button", { name: /Play Live Photo/ });
    expect(document.querySelector("video.viewer__live")).toBeNull();
    await userEvent.hover(live);
    await waitFor(() => expect(document.querySelector("video.viewer__live")?.getAttribute("src")).toContain("/video/90"));
    expect(document.querySelector("video.viewer__live")).toHaveProperty("muted", true);
  });

  it("opens from ?asset=, navigates with arrows, toggles info, closes with Escape", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => asset(Number(a?.id)));
    mockCommand("get_asset_neighbors", (a) => (Number(a?.id) === 7 ? { prev: null, next: 8 } : { prev: 7, next: null }));
    render(
      <Providers>
        <MemoryRouter initialEntries={["/library?asset=7"]}>
          <Routes>
            <Route
              path="*"
              element={
                <>
                  <Probe />
                  <Viewer />
                </>
              }
            />
          </Routes>
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByText("IMG_7.jpg")).toBeInTheDocument();
    await userEvent.keyboard("{ArrowRight}");
    await waitFor(() => expect(screen.getByTestId("search")).toHaveTextContent("asset=8"));
    expect(await screen.findByText("IMG_8.jpg")).toBeInTheDocument();
    await userEvent.keyboard("i");
    expect(await screen.findByRole("complementary", { name: "Info" })).toHaveTextContent("FE 35mm");
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.getByTestId("search")).not.toHaveTextContent("asset="));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("links an export added to the library back to the photo it was made from", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) =>
      Number(a?.id) === 8 ? { ...asset(8), filename: "IMG_3-edited.png", derivedFrom: { id: 3, filename: "IMG_3.jpg" } } : { ...asset(Number(a?.id)), derivedFrom: null },
    );
    mockCommand("get_asset_neighbors", () => ({ prev: null, next: null }));
    render(
      <Providers>
        <MemoryRouter initialEntries={["/library?asset=8"]}>
          <Routes>
            <Route
              path="*"
              element={
                <>
                  <Probe />
                  <Viewer />
                </>
              }
            />
          </Routes>
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByText("IMG_3-edited.png")).toBeInTheDocument();
    await userEvent.keyboard("i");
    const info = await screen.findByRole("complementary", { name: "Info" });
    expect(info).toHaveTextContent("Exported from");
    await userEvent.click(screen.getByRole("button", { name: "IMG_3.jpg" }));
    await waitFor(() => expect(screen.getByTestId("search")).toHaveTextContent("asset=3"));
  });

  it("copies the photo as shown from a right-click on it (Round 4G)", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => asset(Number(a?.id)));
    mockCommand("get_asset_neighbors", () => ({ prev: null, next: null }));
    mockCommand("faces_for_asset", () => []);
    mockCommand("copy_image_to_clipboard", () => null);
    render(
      <Providers>
        <MemoryRouter initialEntries={["/library?asset=7"]}>
          <Viewer />
        </MemoryRouter>
        <ContextMenuHost />
      </Providers>,
    );
    await screen.findByRole("button", { name: "Add to Favorites" });
    await userEvent.pointer({ keys: "[MouseRight]", target: document.querySelector(".viewer__stage")! });
    // Opened from the Library, not an album: no cover to set.
    // The item's name carries its shortcut, shown on the right.
    expect(await screen.findByRole("menuitem", { name: /^Copy image/ }, { timeout: 5000 })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Set as album cover" })).toBeNull();
    await userEvent.click(screen.getByRole("menuitem", { name: /^Copy image/ }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("copy_image_to_clipboard", { id: 7 }));
  });

  it("shows pets' boxes with Shift+P, and a box's menu says Not a pet with Undo (Round 8B)", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => asset(Number(a?.id)));
    mockCommand("get_asset_neighbors", () => ({ prev: null, next: null }));
    mockCommand("faces_for_asset", () => []);
    mockCommand("asset_pets", () => [{ id: 41, petId: 1, name: "Milo", species: "dog", x: 0.2, y: 0.2, w: 0.3, h: 0.3 }]);
    const place = { detection: 41, pet: 1, pinned: false };
    mockCommand("reject_pet_detection", () => place);
    mockCommand("undo_reject_pet_detection", () => null);
    const { toastStore } = await import("@/components/toast/store");
    // The overlay lies on the measured picture: give it a size, and measure at once.
    const width = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
    const height = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
    Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => 800 });
    Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 600 });
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(private cb: (entries: never) => void) {}
        observe() {
          this.cb([] as never);
        }
        disconnect() {}
      },
    );
    onTestFinished(() => {
      if (width) Object.defineProperty(HTMLElement.prototype, "clientWidth", width);
      if (height) Object.defineProperty(HTMLElement.prototype, "clientHeight", height);
      vi.unstubAllGlobals();
    });
    render(
      <Providers>
        <MemoryRouter initialEntries={["/library?asset=7"]}>
          <Viewer />
        </MemoryRouter>
        <ContextMenuHost />
      </Providers>,
    );
    await screen.findByRole("button", { name: "Add to Favorites" });
    expect(screen.queryByRole("button", { name: /^Milo:/ })).toBeNull();
    await userEvent.keyboard("{Shift>}P{/Shift}");
    const box = await screen.findByRole("button", { name: "Milo: change which pet this is" });
    await userEvent.pointer({ keys: "[MouseRight]", target: box });
    await userEvent.click(await screen.findByRole("menuitem", { name: "Not a pet" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("reject_pet_detection", { detection: 41 }));
    toastStore.get().toasts.at(-1)!.action!.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("undo_reject_pet_detection", { place }));
    await userEvent.keyboard("{Shift>}P{/Shift}");
    await waitFor(() => expect(screen.queryByRole("button", { name: /^Milo:/ })).toBeNull());
  });

  it("favorites the open photo from the toolbar and with f", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => asset(Number(a?.id)));
    mockCommand("get_asset_neighbors", () => ({ prev: null, next: null }));
    render(
      <Providers>
        <MemoryRouter initialEntries={["/library?asset=7"]}>
          <Viewer />
        </MemoryRouter>
      </Providers>,
    );
    await userEvent.click(await screen.findByRole("button", { name: "Add to Favorites" }));
    expect(invokeMock).toHaveBeenCalledWith("set_favorite", { ids: [7], value: true });
    // The button reflects the new state, so pressing f again clears it.
    expect(await screen.findByRole("button", { name: "Remove from Favorites" })).toBeInTheDocument();
    await userEvent.keyboard("f");
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_favorite", { ids: [7], value: false }));
  });

  it("decides to serve the original only for HDR files the webview can decode", () => {
    // A gain-map JPEG or PNG must arrive as its own bytes; our 2560 preview is SDR.
    expect(servesOriginalForHdr(1, "jpg")).toBe(true);
    expect(servesOriginalForHdr(2, "PNG")).toBe(true);
    expect(servesOriginalForHdr(3, "avif")).toBe(true);
    // HEIC is HDR but Chromium cannot decode it, so the rendered derivative stands.
    expect(servesOriginalForHdr(1, "heic")).toBe(false);
    expect(servesOriginalForHdr(1, "nef")).toBe(false);
    // An ordinary photo keeps the cheaper preview path.
    expect(servesOriginalForHdr(0, "jpg")).toBe(false);
  });

  it("keeps the SDR preview path for an ordinary photo", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => asset(Number(a?.id)));
    mockCommand("get_asset_neighbors", () => ({ prev: null, next: null }));
    render(
      <Providers>
        <MemoryRouter initialEntries={["/library?asset=7"]}>
          <Viewer />
        </MemoryRouter>
      </Providers>,
    );
    await screen.findByText("IMG_7.jpg");
    const img = document.querySelector(".zoom-pan__img") as HTMLImageElement | null;
    expect(img?.getAttribute("src") ?? "").not.toContain("full/7");
  });

  it("loads an oversized photo in full in the background, then swaps it in; an ordinary one shows no ring", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    const loads = stubImages();
    mockCommand("get_asset", (a) => ({ ...asset(Number(a?.id)), width: a?.id === 8 ? 16000 : 4000, height: a?.id === 8 ? 12000 : 3000 }));
    mockCommand("get_asset_neighbors", () => ({ prev: null, next: null }));
    const shown = () => (document.querySelector(".zoom-pan__img") as HTMLImageElement | null)?.getAttribute("src") ?? "";

    mountViewer("/library?asset=8");
    await screen.findByText("IMG_8.jpg");
    // The 2560 preview first; once it is up, the full resolution is asked for without a zoom.
    await waitFor(() => expect(loads.some((l) => l.src.includes("preview/8"))).toBe(true));
    // The first preload belongs to the render before the asset arrived and was dropped.
    act(() => loads.filter((l) => l.src.includes("preview/8")).at(-1)!.onload?.());
    await waitFor(() => expect(loads.some((l) => l.src.includes("full/8"))).toBe(true));
    expect(await screen.findByText("Loading full resolution…")).toBeInTheDocument();
    expect(shown()).not.toContain("full/8");
    const full = loads.filter((l) => l.src.includes("full/8")).at(-1)!;
    full.naturalWidth = 12000;
    full.naturalHeight = 9000;
    act(() => full.onload?.());
    await waitFor(() => expect(shown()).toContain("full/8"));
    expect(screen.queryByText("Loading full resolution…")).toBeNull();
    expect(screen.getByText(/Shown at 75 % — larger than this PC can open in full/)).toBeInTheDocument();
  });

  it("drops the full-resolution request when the viewer closes", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    const loads = stubImages();
    mockCommand("get_asset", (a) => ({ ...asset(Number(a?.id)), width: 16000, height: 12000 }));
    mockCommand("get_asset_neighbors", () => ({ prev: null, next: null }));
    const view = mountViewer("/library?asset=8");
    await screen.findByText("IMG_8.jpg");
    await waitFor(() => expect(loads.some((l) => l.src.includes("preview/8"))).toBe(true));
    act(() => loads.filter((l) => l.src.includes("preview/8")).at(-1)!.onload?.());
    await waitFor(() => expect(loads.some((l) => l.src.includes("full/8"))).toBe(true));
    const full = loads.filter((l) => l.src.includes("full/8")).at(-1)!;
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    view.unmount();
    expect(full.src).toBe("");
    // A late `load` of the dropped request changes nothing (and warns of nothing).
    full.onload?.();
    expect(errors).not.toHaveBeenCalled();
    errors.mockRestore();
  });

  it("shows no full-resolution ring for an ordinary photo", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => asset(Number(a?.id)));
    mockCommand("get_asset_neighbors", () => ({ prev: null, next: null }));
    mountViewer("/library?asset=9");
    await screen.findByText("IMG_9.jpg");
    expect(screen.queryByText("Loading full resolution…")).toBeNull();
    expect(screen.queryByText(/larger than this PC/)).toBeNull();
  });

  it("rates with the number keys and keeps zoom on Ctrl+0 / Ctrl+1", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => asset(Number(a?.id)));
    mockCommand("get_asset_neighbors", () => ({ prev: null, next: null }));
    render(
      <Providers>
        <MemoryRouter initialEntries={["/library?asset=7"]}>
          <Viewer />
        </MemoryRouter>
      </Providers>,
    );
    await screen.findByText("IMG_7.jpg");
    await userEvent.keyboard("3");
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_rating", { ids: [7], rating: 3 }));
    await userEvent.keyboard("0");
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_rating", { ids: [7], rating: 0 }));
    // Ctrl+1 is zoom, not a rating.
    invokeMock.mockClear();
    await userEvent.keyboard("{Control>}1{/Control}");
    expect(invokeMock).not.toHaveBeenCalledWith("set_rating", expect.objectContaining({ rating: 1 }));
    expect(screen.getByRole("button", { name: "Rate 3 stars" })).toBeInTheDocument();
  });

  it("opens edit mode from the toolbar and returns on Cancel without saving", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => asset(Number(a?.id)));
    mockCommand("get_asset_neighbors", () => ({ prev: null, next: null }));
    render(
      <Providers>
        <MemoryRouter initialEntries={["/library?asset=1"]}>
          <Routes>
            <Route path="*" element={<><Viewer /><Probe /></>} />
          </Routes>
        </MemoryRouter>
      </Providers>,
    );
    await userEvent.click(await screen.findByRole("button", { name: "Edit" }));
    expect(await screen.findByText("Straighten")).toBeInTheDocument();
    expect(screen.getByTestId("search")).toHaveTextContent("edit=1");
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByText("Exposure")).not.toBeInTheDocument());
    expect(screen.getByTestId("search")).not.toHaveTextContent("edit=1");
    expect(invokeMock).not.toHaveBeenCalledWith("save_edit", expect.anything());
  });

  it("Done saves the draft and leaves edit mode", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => asset(Number(a?.id)));
    mockCommand("get_asset_neighbors", () => ({ prev: null, next: null }));
    mountViewer("/library?asset=1&edit=1");
    await userEvent.click(await screen.findByRole("button", { name: "Rotate right" }));
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("save_edit", { assetId: 1, recipe: expect.objectContaining({ rotate: 90 }) }));
    await waitFor(() => expect(screen.queryByText("Exposure")).not.toBeInTheDocument());
  });

  it("offers no edit button for a video", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => ({ ...asset(Number(a?.id)), mediaType: 1, extension: "mp4" }));
    mockCommand("get_asset_neighbors", () => ({ prev: null, next: null }));
    mountViewer("/library?asset=1");
    await screen.findByRole("button", { name: "Info" });
    expect(screen.queryByRole("button", { name: "Edit" })).not.toBeInTheDocument();
  });

  it("plays a sequence: one photo every four seconds, then stops", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_asset", (a) => asset(Number(a?.id)));
    setViewerSequence([1, 2, 3]);
    render(
      <Providers>
        <MemoryRouter initialEntries={["/memories/7?asset=1&play=1"]}>
          <Routes>
            <Route path="*" element={<><Viewer /><Probe /></>} />
          </Routes>
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByText("IMG_1.jpg")).toBeInTheDocument();
    await act(async () => {
      vi.advanceTimersByTime(4100);
    });
    expect(await screen.findByText("IMG_2.jpg")).toBeInTheDocument();
    await act(async () => {
      vi.advanceTimersByTime(4100);
    });
    expect(await screen.findByText("IMG_3.jpg")).toBeInTheDocument();
    await act(async () => {
      vi.advanceTimersByTime(4100);
    });
    await waitFor(() => expect(screen.getByTestId("search")).not.toHaveTextContent("play=1"));
    expect(screen.getByTestId("search")).toHaveTextContent("asset=3");
    clearViewerSequence();
    vi.useRealTimers();
  });

  describe("text in photos (Round 13-3)", () => {
    const quad = [0.1, 0.1, 0.5, 0.1, 0.5, 0.2, 0.1, 0.2];
    const reading = (on: boolean) => ({ appearance: { reduceMotion: false }, smart: { enabled: true, experimental: { readText: on } } });
    function mountText(on = true) {
      mockCommand("get_settings", () => reading(on));
      mockLibraryDefaults();
      mockCommand("get_asset", (a) => asset(Number(a?.id)));
      mockCommand("get_asset_neighbors", () => ({ prev: 1, next: 99 }));
      // Photo 42 has a sign in it; its neighbours have no text.
      mockCommand("asset_text", (a) => (Number(a?.id) === 42 ? [{ ord: 0, text: "Gostilna", quad }] : []));
      mockCommand("prefetch_text", () => null);
      mountViewer("/library?asset=42");
    }

    it("the text button only shows for a photo with lines", async () => {
      mountText();
      expect(await screen.findByRole("button", { name: "Text" })).toBeInTheDocument();
      await userEvent.keyboard("{ArrowRight}");
      expect(await screen.findByText("IMG_99.jpg")).toBeInTheDocument();
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("asset_text", { id: 99 }));
      await waitFor(() => expect(screen.queryByRole("button", { name: "Text" })).toBeNull());
    });

    it("T toggles the text over the photo", async () => {
      mountText();
      const button = await screen.findByRole("button", { name: "Text" });
      expect(button).toHaveAttribute("aria-pressed", "false");
      await userEvent.keyboard("t");
      expect(button).toHaveAttribute("aria-pressed", "true");
      await userEvent.keyboard("t");
      expect(button).toHaveAttribute("aria-pressed", "false");
    });

    it("Esc turns the text off before anything else", async () => {
      mountText();
      const button = await screen.findByRole("button", { name: "Text" });
      await userEvent.click(button);
      expect(button).toHaveAttribute("aria-pressed", "true");
      await userEvent.keyboard("{Escape}");
      expect(button).toHaveAttribute("aria-pressed", "false");
      expect(screen.getByRole("dialog")).toBeInTheDocument();
    });

    it("lists the text in the info panel", async () => {
      mountText();
      await screen.findByRole("button", { name: "Text" });
      await userEvent.keyboard("i");
      expect(await screen.findByRole("button", { name: "Copy" })).toBeInTheDocument();
      expect(screen.getByText("Gostilna")).toBeInTheDocument();
    });

    it("puts the text on the photo from the info panel's On photo", async () => {
      mountText();
      const toolbar = await screen.findByRole("button", { name: "Text" });
      await userEvent.keyboard("i");
      const onPhoto = await screen.findByRole("button", { name: /On photo/ });
      expect(onPhoto).toHaveAttribute("aria-pressed", "false");
      await userEvent.click(onPhoto);
      expect(onPhoto).toHaveAttribute("aria-pressed", "true");
      expect(toolbar).toHaveAttribute("aria-pressed", "true");
    });

    it("hides everything while reading text is off", async () => {
      mountText(false);
      expect(await screen.findByText("IMG_42.jpg")).toBeInTheDocument();
      await userEvent.keyboard("t");
      expect(screen.queryByRole("button", { name: "Text" })).toBeNull();
      expect(invokeMock.mock.calls.some(([c]) => c === "asset_text" || c === "prefetch_text")).toBe(false);
    });

    it("asks for the neighbours' text ahead", async () => {
      mountText();
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("prefetch_text", { ids: [1, 99] }));
    });
  });
});
