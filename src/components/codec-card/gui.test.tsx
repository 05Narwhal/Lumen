import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { CodecCard } from "./gui";

describe("CodecCard", () => {
  it("shows install instructions when FFmpeg is missing and re-checks on demand", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_ffmpeg_status", () => ({ available: false, version: null, path: null, envVar: "LUMEN_FFMPEG_DIR" }));
    render(
      <Providers>
        <CodecCard />
      </Providers>,
    );
    expect(await screen.findByText("Video and HEIC need FFmpeg")).toBeInTheDocument();
    expect(screen.getByText(/LUMEN_FFMPEG_DIR/)).toBeInTheDocument();
    invokeMock.mockClear();
    await userEvent.click(screen.getByRole("button", { name: "Check again" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_ffmpeg_status", undefined));
  });

  it("shows the version when FFmpeg is present and nothing in compact mode", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_ffmpeg_status", () => ({ available: true, version: "ffmpeg version 7.1", path: "C:/ffmpeg.exe", envVar: "LUMEN_FFMPEG_DIR" }));
    const { unmount } = render(
      <Providers>
        <CodecCard />
      </Providers>,
    );
    expect(await screen.findByText("ffmpeg version 7.1")).toBeInTheDocument();
    unmount();
    render(
      <Providers>
        <CodecCard compact />
      </Providers>,
    );
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_ffmpeg_status", undefined));
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });
});
