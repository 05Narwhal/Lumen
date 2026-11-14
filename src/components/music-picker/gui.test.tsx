import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router";
import { Providers } from "@/app";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { MusicPicker } from "./gui";

const track = (id: string, title: string) => ({
  id,
  title,
  artist: "Marta Vidal",
  durationMs: 184_000,
  licenceName: "CC BY 3.0",
  licenceUrl: "http://creativecommons.org/licenses/by/3.0/",
  shareUrl: `https://www.jamendo.com/track/${id}`,
  previewUrl: "",
  downloadUrl: "",
  tags: ["calm"],
  derivativesAllowed: true,
});

const saved = (title: string, path: string) => ({
  id: path,
  title,
  artist: "Marta Vidal",
  durationMs: 184_000,
  licenceName: "CC BY 3.0",
  licenceUrl: "",
  shareUrl: "",
  credit: "credit",
  path,
  durationMsRaw: 0,
});

function mount(onUse = vi.fn()) {
  render(
    <Providers>
      <MemoryRouter>
        <MusicPicker onClose={() => {}} onUse={onUse} onChooseFile={() => {}} />
      </MemoryRouter>
    </Providers>,
  );
  return onUse;
}

beforeEach(() => {
  mockLibraryDefaults();
  mockCommand("get_settings", () => null);
});

describe("MusicPicker", () => {
  it("lists what is already in the library and uses a track", async () => {
    mockCommand("list_library_music", () => [saved("Lantern Song", "C:/lib/music/1.mp3")]);
    const onUse = mount();
    await userEvent.click(await screen.findByRole("button", { name: /Use Lantern Song/ }));
    expect(onUse).toHaveBeenCalledWith(expect.objectContaining({ path: "C:/lib/music/1.mp3" }), expect.any(Number));
  });

  it("explains itself when there is no client id yet", async () => {
    mockCommand("list_library_music", () => []);
    mount();
    await userEvent.click(await screen.findByRole("tab", { name: "Find music" }));
    expect(await screen.findByText(/Jamendo/)).toBeInTheDocument();
    expect(screen.queryByRole("searchbox", { name: "Search for music" })).not.toBeInTheDocument();
  });

  it("searches once a client id is set, and passes the filters", async () => {
    mockCommand("get_settings", () => ({
      version: 1,
      appearance: { theme: "gallery", scheme: "auto", tileSize: "m", reduceMotion: false, sidebarCollapsed: false },
      import: { duplicates: "skip", promptOnVolume: true },
      playback: { loopVideos: false, muteByDefault: false },
      map: { online: true, style: "map" },
      music: { jamendoClientId: "KEY" },
      window: { width: 1280, height: 800, x: null, y: null, maximized: false },
    }));
    mockCommand("list_library_music", () => []);
    mockCommand("music_search", () => [track("9", "Sunny Afternoon")]);
    mount();
    await userEvent.click(await screen.findByRole("tab", { name: "Find music" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("music_search", expect.objectContaining({ query: expect.anything() })));
    expect(await screen.findByText("Sunny Afternoon")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("radio", { name: "Calm" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("music_search", { query: expect.objectContaining({ vibe: "calm" }) }),
    );
  });

  it("saves a track into the library before using it", async () => {
    mockCommand("get_settings", () => ({
      version: 1,
      appearance: { theme: "gallery", scheme: "auto", tileSize: "m", reduceMotion: false, sidebarCollapsed: false },
      import: { duplicates: "skip", promptOnVolume: true },
      playback: { loopVideos: false, muteByDefault: false },
      map: { online: true, style: "map" },
      music: { jamendoClientId: "KEY" },
      window: { width: 1280, height: 800, x: null, y: null, maximized: false },
    }));
    mockCommand("list_library_music", () => []);
    mockCommand("music_search", () => [track("9", "Sunny Afternoon")]);
    mockCommand("music_download", () => saved("Sunny Afternoon", "C:/lib/music/9.mp3"));
    const onUse = mount();
    await userEvent.click(await screen.findByRole("tab", { name: "Find music" }));
    await userEvent.click(await screen.findByRole("button", { name: /Use Sunny Afternoon/ }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("music_download", expect.anything()));
    await waitFor(() => expect(onUse).toHaveBeenCalledWith(expect.objectContaining({ path: "C:/lib/music/9.mp3" }), expect.any(Number)));
  });
  it("plays at the chosen volume, and passes it on as the show's level", async () => {
    mockCommand("list_library_music", () => [saved("Lantern Song", "C:/lib/music/1.mp3")]);
    const onUse = mount();
    const volume = await screen.findByLabelText("Preview volume");
    fireEvent.change(volume, { target: { value: "50" } });
    await userEvent.click(await screen.findByRole("button", { name: "Play Lantern Song" }));
    const audio = document.querySelector("audio") as HTMLAudioElement;
    await waitFor(() => expect(audio.volume).toBeCloseTo(0.5, 2));
    await userEvent.click(screen.getByRole("button", { name: "Use Lantern Song" }));
    // 20 log10(0.5) = -6.02 dB
    expect(onUse).toHaveBeenCalledWith(expect.objectContaining({ title: "Lantern Song" }), expect.closeTo(-6.02, 1));
  });

  it("stops the track that was playing when another starts", async () => {
    mockCommand("list_library_music", () => [saved("One", "C:/lib/music/1.mp3"), saved("Two", "C:/lib/music/2.mp3")]);
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "Play One" }));
    expect(screen.getByRole("button", { name: "Stop One" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Play Two" }));
    expect(screen.getByRole("button", { name: "Play One" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Stop Two" })).toBeInTheDocument();
  });
});
