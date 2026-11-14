import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { toastStore } from "@/components/toast/store";
import { selectionStore } from "@/hooks/use-selection";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { SlideshowDoc } from "@/types/generated/SlideshowDoc";
import { SlideshowsRoute } from "./gui";

const doc = (): SlideshowDoc => ({
  version: 1,
  resolution: [1920, 1080],
  fps: 30,
  audio: null,
  clips: [1, 2, 3].map((asset_id) => ({
    asset_id,
    extra: [],
    frame: "fill" as const,
    duration_ms: 4000,
    transition: { kind: "crossfade", duration_ms: 600 },
    motion: { kind: "none", from: [0, 0, 1], to: [0, 0, 1] },
  })),
});

function mount() {
  render(
    <Providers>
      <MemoryRouter initialEntries={["/slideshows/3"]}>
        <Routes>
          <Route path="/slideshows/:id" element={<SlideshowsRoute />} />
        </Routes>
      </MemoryRouter>
    </Providers>,
  );
}

/** The document as the last autosave saw it. */
const savedDoc = () => {
  const calls = invokeMock.mock.calls.filter(([c]) => c === "save_slideshow");
  return (calls.at(-1)?.[1] as { doc: SlideshowDoc } | undefined)?.doc;
};

beforeEach(() => {
  mockCommand("get_settings", () => null);
  mockLibraryDefaults();
  mockCommand("get_slideshow", () => ({ id: 3, name: "Trip to Lisbon", doc: doc() }));
  toastStore.set({ toasts: [] });
});

describe("Slideshow editor", () => {
  it("shows the name, the preview and one card per clip", async () => {
    mount();
    expect(await screen.findByDisplayValue("Trip to Lisbon")).toBeInTheDocument();
    expect(screen.getByLabelText("Slideshow preview")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /^Clip \d/ })).toHaveLength(3);
    expect(screen.getByLabelText("Playback position")).toBeInTheDocument();
    expect(screen.getByText("0:00 / 0:11")).toBeInTheDocument();
  });

  it("reorders clips by dragging one onto another", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mount();
    const clips = await screen.findAllByRole("button", { name: /^Clip \d/ });
    fireEvent.dragStart(clips[0]);
    fireEvent.dragOver(clips[2]);
    fireEvent.drop(clips[2]);
    await vi.advanceTimersByTimeAsync(700);
    expect(savedDoc()?.clips.map((c) => c.asset_id)).toEqual([2, 3, 1]);
    vi.useRealTimers();
  });

  it("trims a clip by dragging its right edge, in tenths of a second", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mount();
    const handle = (await screen.findAllByRole("separator", { name: /^Trim clip/ }))[0];
    fireEvent.pointerDown(handle, { clientX: 100 });
    fireEvent.pointerMove(window, { clientX: 148 });
    fireEvent.pointerUp(window, { clientX: 148 });
    await vi.advanceTimersByTimeAsync(700);
    expect(savedDoc()?.clips[0].duration_ms).toBe(5000);
    vi.useRealTimers();
  });

  it("removes the selected clip with Delete", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mount();
    const clips = await screen.findAllByRole("button", { name: /^Clip \d/ });
    fireEvent.click(clips[1]);
    fireEvent.keyDown(window, { key: "Delete" });
    await vi.advanceTimersByTimeAsync(700);
    expect(savedDoc()?.clips.map((c) => c.asset_id)).toEqual([1, 3]);
    vi.useRealTimers();
  });

  it("carries an inspector change into the document", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mount();
    const clips = await screen.findAllByRole("button", { name: /^Clip \d/ });
    fireEvent.click(clips[0]);
    await userEvent.click(await screen.findByRole("combobox", { name: "Transition" }));
    await userEvent.click(await screen.findByRole("option", { name: "Slide up" }));
    await vi.advanceTimersByTimeAsync(700);
    expect(savedDoc()?.clips[0].transition.kind).toBe("slideup");
    // A join the user chose is theirs: a beat sync must not choose it again.
    expect(savedDoc()?.clips[0].transition.auto).toBe(false);
    vi.useRealTimers();
  });

  it("changes the document's resolution when no clip is selected", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mount();
    fireEvent.click(await screen.findByRole("radio", { name: "4K" }));
    await vi.advanceTimersByTimeAsync(700);
    expect(savedDoc()?.resolution).toEqual([3840, 2160]);
    vi.useRealTimers();
  });

  it("changes a clip's frame", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mount();
    const clips = await screen.findAllByRole("button", { name: /^Clip \d/ });
    fireEvent.click(clips[0]);
    await userEvent.click(await screen.findByRole("combobox", { name: "Frame" }));
    await userEvent.click(await screen.findByRole("option", { name: "Whole, on a blurred backdrop" }));
    await vi.advanceTimersByTimeAsync(700);
    expect(savedDoc()?.clips[0].frame).toBe("blur");
    vi.useRealTimers();
  });

  it("offers every transition, grouped, and every one Rust knows", async () => {
    mount();
    const clips = await screen.findAllByRole("button", { name: /^Clip \d/ });
    fireEvent.click(clips[0]);
    await userEvent.click(await screen.findByRole("combobox", { name: "Transition" }));
    const list = within(await screen.findByRole("listbox"));
    expect(list.getAllByRole("option").map((o) => o.textContent)).toEqual([
      "None",
      "Crossfade",
      "Through black",
      "Through white",
      "Slide",
      "Slide up",
      "Slide down",
      "Cover",
      "Reveal",
      "Wipe",
      "Wipe up",
      "Corner",
      "Open vertically",
      "Open across",
      "Circle in",
      "Circle out",
      "Squeeze",
      "Zoom",
      "Slices",
      "Bands",
    ]);
    // The five groups keep twenty of them findable.
    expect(list.getAllByRole("group")).toHaveLength(5);
  });

  it("opens the music picker and attaches what it returns", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mockCommand("list_library_music", () => [
      {
        id: "1886711",
        title: "Lantern Song",
        artist: "Marta Vidal",
        durationMs: 184_000,
        licenceName: "CC BY 3.0",
        licenceUrl: "",
        shareUrl: "",
        credit: "credit",
        path: "C:/lib/music/1886711.mp3",
      },
    ]);
    mount();
    await userEvent.click(await screen.findByRole("button", { name: /Choose music/ }));
    await userEvent.click(await screen.findByRole("button", { name: /Use Lantern Song/ }));
    await vi.advanceTimersByTimeAsync(700);
    expect(savedDoc()?.audio).toMatchObject({ path: "C:/lib/music/1886711.mp3", fade_in_ms: 1000, fade_out_ms: 1500 });
    // The picker's slider decides the show's level, and it never arrives above the recording.
    expect(savedDoc()?.audio?.gain_db).toBeLessThanOrEqual(0);
    vi.useRealTimers();
  });

  it("moves chosen music to the part of the song that suits the show", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mockCommand("list_library_music", () => [
      {
        id: "7",
        title: "Chorus",
        artist: "A",
        durationMs: 184_000,
        licenceName: "CC BY 3.0",
        licenceUrl: "",
        shareUrl: "",
        credit: "credit",
        path: "C:/lib/music/7.mp3",
      },
    ]);
    mockCommand("music_window", () => ({ start_ms: 42_000, fade_in_ms: 400, fade_out_ms: 1000 }));
    mount();
    await userEvent.click(await screen.findByRole("button", { name: /Choose music/ }));
    await userEvent.click(await screen.findByRole("button", { name: /Use Chorus/ }));
    await vi.advanceTimersByTimeAsync(700);
    expect(invokeMock).toHaveBeenCalledWith("music_window", expect.objectContaining({ path: "C:/lib/music/7.mp3" }));
    expect(savedDoc()?.audio).toMatchObject({ path: "C:/lib/music/7.mp3", start_ms: 42_000, fade_in_ms: 400, fade_out_ms: 1000 });
    expect(await screen.findByText("Plays from 0:42")).toBeInTheDocument();
    vi.useRealTimers();
  });

  it("cuts the show to the beat of its music", async () => {
    mockCommand("get_slideshow", () => ({
      id: 3,
      name: "Trip to Lisbon",
      doc: { ...doc(), audio: { path: "m.mp3", fade_in_ms: 0, fade_out_ms: 0, gain_db: 0, start_ms: 0 } },
    }));
    mount();
    await userEvent.click(await screen.findByRole("button", { name: /Cut to the beat/ }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("sync_slideshow_to_music", { id: 3 }));
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.message).toBe("Cut to 120 BPM"));
  });

  it("says so when the track has no beat", async () => {
    mockCommand("get_slideshow", () => ({
      id: 3,
      name: "Trip to Lisbon",
      doc: { ...doc(), audio: { path: "m.mp3", fade_in_ms: 0, fade_out_ms: 0, gain_db: 0, start_ms: 0 } },
    }));
    mockCommand("sync_slideshow_to_music", () => {
      throw { code: "no_beat", message: "This track has no steady beat to cut to." };
    });
    mount();
    await userEvent.click(await screen.findByRole("button", { name: /Cut to the beat/ }));
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.message).toBe("This track has no steady beat to cut to."));
  });

  it("shows a grid clip as the photos it holds", async () => {
    mockCommand("get_slideshow", () => {
      const d = doc();
      d.clips[0] = { ...d.clips[0], frame: "grid", extra: [9] };
      return { id: 3, name: "Trip to Lisbon", doc: d };
    });
    mount();
    const first = (await screen.findAllByRole("button", { name: /^Clip \d/ }))[0];
    expect(first.querySelectorAll("img")).toHaveLength(2);
  });

  it("shuffles the frames and transitions", async () => {
    mount();
    await userEvent.click(await screen.findByRole("button", { name: /Shuffle/ }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("rebuild_slideshow", { id: 3, style: "random" }));
  });

  it("asks where to save, then exports through the backend (Round 6)", async () => {
    vi.mocked(openDialog).mockResolvedValueOnce("D:/Films");
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "Export" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("export_slideshow", { id: 3, destDir: "D:/Films" }));
  });

  it("exports nothing when the folder dialog is cancelled", async () => {
    vi.mocked(openDialog).mockResolvedValueOnce(null);
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "Export" }));
    await waitFor(() => expect(openDialog).toHaveBeenCalled());
    expect(invokeMock).not.toHaveBeenCalledWith("export_slideshow", expect.anything());
  });

  it("explains why Export is off when FFmpeg is missing", async () => {
    mockCommand("get_ffmpeg_status", () => ({ available: false, version: null, path: null, envVar: "LUMEN_FFMPEG_DIR" }));
    mount();
    const button = await screen.findByRole("button", { name: "Export" });
    await waitFor(() => expect(button).toBeDisabled());
    expect(screen.getByText(/needs FFmpeg/i)).toBeInTheDocument();
  });

  it("adds photos and videos picked to the end of the show", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const added = { ...doc().clips[0], asset_id: 9, duration_ms: 4500, video: { in_ms: 2000, out_ms: 6500 } };
    mockCommand("slideshow_clips", () => [added]);
    mount();
    await userEvent.click(await screen.findByRole("button", { name: /Add photos or videos/ }));
    selectionStore.set({ ids: new Set([9]), anchor: 9 });
    await userEvent.click(await screen.findByRole("button", { name: "Add" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("slideshow_clips", { assetIds: [9] }));
    await vi.advanceTimersByTimeAsync(700);
    expect(savedDoc()?.clips.map((c) => c.asset_id)).toEqual([1, 2, 3, 9]);
    expect(savedDoc()?.clips[3].video).toEqual({ in_ms: 2000, out_ms: 6500 });
    vi.useRealTimers();
  });

  it("lets a video clip start elsewhere, or go back to its best part", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const withVideo = doc();
    withVideo.clips[1] = { ...withVideo.clips[1], video: { in_ms: 7000, out_ms: 11000 } };
    mockCommand("get_slideshow", () => ({ id: 3, name: "Trip to Lisbon", doc: withVideo }));
    mockCommand("video_best_part", () => ({ in_ms: 12_000, out_ms: 16_500 }));
    mount();
    await userEvent.click(await screen.findByRole("button", { name: /^Clip 2/ }));
    expect(await screen.findByLabelText(/Starts at/)).toHaveValue("7");
    expect(screen.queryByLabelText("Ken Burns")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Best part" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("video_best_part", { assetId: 2 }));
    await vi.advanceTimersByTimeAsync(700);
    expect(savedDoc()?.clips[1]).toMatchObject({ video: { in_ms: 12_000, out_ms: 16_500 }, duration_ms: 4500 });
    vi.useRealTimers();
  });
});
