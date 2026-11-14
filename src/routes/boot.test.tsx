import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { Boot } from "./boot";

const settings = {
  version: 1,
  appearance: { theme: "gallery", scheme: "auto", tileSize: "m", reduceMotion: false, sidebarCollapsed: false },
  import: { duplicates: "skip", promptOnVolume: true },
  playback: { loopVideos: false, muteByDefault: false },
  map: { online: true, style: "map" as const },
  window: { width: 1280, height: 800, x: null, y: null, maximized: false },
};

describe("Boot", () => {
  it("shows the splash until settings and libraries are loaded, then the app", async () => {
    mockCommand("get_settings", () => settings);
    mockLibraryDefaults();
    mockCommand("get_current_library", () => null);
    render(
      <Providers>
        <Boot>
          <p>app</p>
        </Boot>
      </Providers>,
    );
    expect(screen.getByRole("status", { name: "Loading Lumen" })).toBeInTheDocument();
    expect(screen.queryByText("app")).toBeNull();
    // A stuck start can still be closed (Round 21 §11).
    expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Minimize" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText("app")).toBeInTheDocument(), { timeout: 3000 });
    await waitFor(() => expect(screen.queryByRole("status", { name: "Loading Lumen" })).toBeNull());
  });

  it("opens where Restart Lumen left off, with no splash", async () => {
    mockCommand("get_settings", () => settings);
    mockLibraryDefaults();
    mockCommand("get_current_library", () => null);
    mockCommand("take_resume", () => "/develop/12?panel=detail");
    const resume = vi.fn();
    render(
      <Providers>
        <Boot resume={resume}>
          <p>app</p>
        </Boot>
      </Providers>,
    );
    await waitFor(() => expect(screen.getByText("app")).toBeInTheDocument(), { timeout: 400 });
    expect(resume).toHaveBeenCalledWith("/develop/12?panel=detail");
    await waitFor(() => expect(screen.queryByRole("status", { name: "Loading Lumen" })).toBeNull(), { timeout: 400 });
  });
});
