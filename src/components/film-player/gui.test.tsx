import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { MotionGlobalConfig } from "motion/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { toastStore } from "@/components/toast/store";
import { filmStore, playFilm } from "@/lib/film";
import { emitMock, invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { SlideshowDoc } from "@/types/generated/SlideshowDoc";
import { FilmPlayerHost } from "./gui";

MotionGlobalConfig.skipAnimations = true;

function Where() {
  const { pathname } = useLocation();
  return <p data-testid="where">{pathname}</p>;
}

function mount() {
  render(
    <Providers>
      <MemoryRouter initialEntries={["/memories/7"]}>
        <Routes>
          <Route path="*" element={<Where />} />
        </Routes>
        <FilmPlayerHost />
      </MemoryRouter>
    </Providers>,
  );
}

const doc = {
  resolution: [1920, 1080],
  fps: 30,
  audio: null,
  clips: [
    { asset_id: 3, duration_ms: 4000, transition: { kind: "crossfade", duration_ms: 600 }, motion: { kind: "none", from: [0, 0, 1], to: [0, 0, 1] }, frame: "fill", extra: [] },
    { asset_id: 4, duration_ms: 4000, transition: { kind: "none", duration_ms: 0 }, motion: { kind: "none", from: [0, 0, 1], to: [0, 0, 1] }, frame: "fill", extra: [] },
  ],
} as unknown as SlideshowDoc;

beforeEach(() => {
  mockLibraryDefaults();
  mockCommand("get_settings", () => null);
  filmStore.set(null);
  toastStore.set({ toasts: [] });
});

describe("FilmPlayer", () => {
  it("makes the film, saying what it is doing, then plays it", async () => {
    let finish: (f: unknown) => void = () => {};
    mockCommand("memory_film", () => new Promise((r) => (finish = r)));
    mount();
    act(() => playFilm({ memoryId: 7, title: "Trip to Lisbon", coverAssetId: 3 }));
    expect(await screen.findByRole("dialog", { name: "Trip to Lisbon" })).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("memory_film", { memoryId: 7 });
    expect(screen.getAllByRole("status")[0]).toHaveTextContent("Putting the film together…");
    await act(async () => emitMock("film://progress", { memoryId: 7, stage: "music" }));
    expect(screen.getAllByRole("status")[0]).toHaveTextContent("Choosing music…");
    await act(async () => emitMock("film://progress", { memoryId: 8, stage: "beat" }));
    expect(screen.getAllByRole("status")[0]).toHaveTextContent("Choosing music…");
    await act(async () => emitMock("film://progress", { memoryId: 7, stage: "beat" }));
    expect(screen.getAllByRole("status")[0]).toHaveTextContent("Finding the beat…");
    await act(async () => finish({ name: "Trip to Lisbon", doc, bpm: 92 }));
    expect(await screen.findByLabelText("Slideshow preview")).toBeInTheDocument();
    expect(screen.getByText("Cut to 92 BPM")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Pause" })).toBeInTheDocument();
  });

  it("Space pauses and Esc closes", async () => {
    mockCommand("memory_film", () => ({ name: "Trip", doc, bpm: null }));
    mount();
    act(() => playFilm({ memoryId: 7, title: "Trip", coverAssetId: null }));
    await screen.findByLabelText("Slideshow preview");
    await userEvent.keyboard(" ");
    expect(screen.getByRole("button", { name: "Play" })).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(filmStore.get()).toBeNull();
  });

  it("Esc leaves full screen first, and closes only after (Round 8B)", async () => {
    mockCommand("memory_film", () => ({ name: "Trip", doc, bpm: null }));
    mount();
    act(() => playFilm({ memoryId: 7, title: "Trip", coverAssetId: null }));
    await screen.findByLabelText("Slideshow preview");
    await userEvent.click(screen.getByRole("button", { name: "Full screen" }));
    expect(screen.getByRole("button", { name: "Exit full screen" })).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Full screen" })).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("hides its controls and the cursor when idle, and a key brings them back (Round 8B)", async () => {
    mockCommand("memory_film", () => ({ name: "Trip", doc, bpm: null }));
    mount();
    act(() => playFilm({ memoryId: 7, title: "Trip", coverAssetId: null }));
    await screen.findByLabelText("Slideshow preview");
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      act(() => {
        // A move re-arms the idle timer on the fake clock.
        window.dispatchEvent(new PointerEvent("pointermove"));
        vi.advanceTimersByTime(2500);
      });
      expect(screen.getByRole("dialog")).toHaveAttribute("data-idle", "true");
      expect(document.querySelector(".media-controls")).toHaveAttribute("data-hidden", "true");
      act(() => {
        window.dispatchEvent(new KeyboardEvent("keydown", { key: "Shift" }));
      });
      expect(screen.getByRole("dialog")).toHaveAttribute("data-idle", "false");
    } finally {
      vi.useRealTimers();
    }
  });

  it("Edit slideshow keeps the film as a slideshow and opens it", async () => {
    mockCommand("memory_film", () => ({ name: "Trip", doc, bpm: null }));
    mockCommand("save_memory_film", () => 12);
    mount();
    act(() => playFilm({ memoryId: 7, title: "Trip", coverAssetId: null }));
    await screen.findByLabelText("Slideshow preview");
    await userEvent.click(screen.getByRole("button", { name: "Edit slideshow" }));
    await waitFor(() => expect(screen.getByTestId("where")).toHaveTextContent("/slideshows/12"));
    expect(invokeMock).toHaveBeenCalledWith("save_memory_film", { memoryId: 7 });
    expect(filmStore.get()).toBeNull();
  });

  // Round 6: the viewer's video controls.
  it("has the video controls: time, seek, mute and full screen", async () => {
    mockCommand("memory_film", () => ({ name: "Trip", doc, bpm: null }));
    mount();
    act(() => playFilm({ memoryId: 7, title: "Trip", coverAssetId: null }));
    await screen.findByLabelText("Slideshow preview");
    const seek = screen.getByRole("slider", { name: "Seek" });
    // Two 4 s clips overlapping by 0.6 s.
    expect(seek).toHaveAttribute("max", "7400");
    expect(screen.getByText("0:07")).toBeInTheDocument();
    act(() => {
      seek.focus();
    });
    await userEvent.keyboard(" ");
    expect(screen.getByRole("button", { name: "Play" })).toBeInTheDocument();
    // Dragging the bar moves the film; → skips 5 s on from there.
    fireEvent.change(seek, { target: { value: "1000" } });
    expect(seek).toHaveValue("1000");
    await userEvent.keyboard("{ArrowRight}");
    expect(seek).toHaveValue("6000");
    await userEvent.click(screen.getByRole("button", { name: "Mute" }));
    expect(screen.getByRole("button", { name: "Unmute" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Full screen" })).toBeInTheDocument();
  });

  it("exports the film as a video from the player (Round 6)", async () => {
    mockCommand("memory_film", () => ({ name: "Trip", doc, bpm: null }));
    mockCommand("export_memory_film", () => null);
    vi.mocked(openDialog).mockResolvedValueOnce("D:/Films");
    mount();
    act(() => playFilm({ memoryId: 7, title: "Trip", coverAssetId: null }));
    await screen.findByLabelText("Slideshow preview");
    await userEvent.click(screen.getByRole("button", { name: "Export video" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("export_memory_film", { memoryId: 7, destDir: "D:/Films" }));
  });

  it("a memory with nothing to play says so and closes", async () => {
    mockCommand("memory_film", () => ({ name: "Empty", doc: { ...doc, clips: [] }, bpm: null }));
    mount();
    act(() => playFilm({ memoryId: 7, title: "Empty", coverAssetId: null }));
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.message).toBe("This memory has nothing to play."));
    await waitFor(() => expect(filmStore.get()).toBeNull());
  });
});
