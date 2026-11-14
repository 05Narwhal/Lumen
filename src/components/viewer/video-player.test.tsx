import { render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { emitMock, invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { VideoPlayer } from "./video-player";

const playback = (mode: "native" | "proxy" | "unavailable", proxyReady = false) => ({
  mode,
  proxyReady,
  videoCodec: mode === "native" ? "h264" : "prores",
  audioCodec: "aac",
  container: "mov,mp4",
  durationMs: 2000,
  width: 320,
  height: 240,
});

function mount(id: number) {
  render(
    <Providers>
      <MemoryRouter>
        <VideoPlayer id={id} extension="mov" />
      </MemoryRouter>
    </Providers>,
  );
}

describe("VideoPlayer", () => {
  it("renders a native <video> with controls", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_playback", () => playback("native"));
    mount(4);
    const video = await waitFor(() => {
      const v = document.querySelector("video");
      if (!v) throw new Error("no video yet");
      return v;
    });
    expect(video.getAttribute("src")).toContain("video/4");
    expect(screen.getByRole("slider", { name: "Seek" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Loop" })).toBeInTheDocument();
  });

  it("asks for the proxy, shows progress, then plays it once finished", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    let ready = false;
    mockCommand("get_playback", () => playback("proxy", ready));
    mount(9);
    expect(await screen.findByText(/Preparing playback/)).toBeInTheDocument();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("prepare_proxy", { id: 9 }));
    emitMock("proxy://progress", { assetId: 9, progress: 0.5 });
    expect(await screen.findByText(/50%/)).toBeInTheDocument();
    ready = true;
    emitMock("proxy://finished", { assetId: 9, ok: true });
    const video = await waitFor(() => {
      const v = document.querySelector("video");
      if (!v) throw new Error("no video yet");
      return v;
    });
    expect(video.getAttribute("src")).toContain("proxy=1");
  });

  it("explains when playback is unavailable", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_playback", () => playback("unavailable"));
    mount(2);
    expect(await screen.findByText(/needs FFmpeg to play/)).toBeInTheDocument();
  });
});
