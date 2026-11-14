import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router";
import { describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { SetupRoute } from "./gui";

const settings = {
  version: 1,
  appearance: { theme: "gallery", scheme: "auto", tileSize: "m", reduceMotion: false, sidebarCollapsed: false },
  import: { duplicates: "skip", promptOnVolume: true },
  playback: { loopVideos: false, muteByDefault: false },
  map: { online: true, style: "map" as const },
  window: { width: 1280, height: 800, x: null, y: null, maximized: false },
};

describe("SetupRoute", () => {
  it("walks welcome → theme → library → import and creates the library", async () => {
    mockCommand("get_settings", () => settings);
    mockCommand("update_settings", (a) => a?.settings);
    mockCommand("get_app_info", () => ({
      version: "0.1.0",
      platform: "windows",
      appDataDir: "C:/appdata",
      defaultLibraryDir: "C:/Users/j/Pictures/Lumen Library",
      isDebug: true,
    }));
    mockCommand("get_free_space", () => 120 * 1024 ** 3);
    mockCommand("library_exists_at", () => false);
    const create = vi.fn(() => ({ name: "Lumen Library", path: "C:/Users/j/Pictures/Lumen Library", schemaVersion: 1 }));
    mockCommand("create_library", create);
    mockLibraryDefaults();
    mockCommand("get_current_library", () => null);

    const router = createMemoryRouter(
      [
        { path: "/setup", Component: SetupRoute },
        { path: "/", element: <p>home</p> },
      ],
      { initialEntries: ["/setup"] },
    );
    render(
      <Providers>
        <RouterProvider router={router} />
      </Providers>,
    );

    await userEvent.click(await screen.findByRole("button", { name: "Get started" }));
    await userEvent.click(await screen.findByRole("radio", { name: /Darkroom/ }));
    expect(invokeMock).toHaveBeenCalledWith(
      "update_settings",
      expect.objectContaining({
        settings: expect.objectContaining({ appearance: expect.objectContaining({ theme: "darkroom" }) }),
      }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(await screen.findByText("C:/Users/j/Pictures/Lumen Library")).toBeInTheDocument();
    expect(await screen.findByText(/120 GB free/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Create library" }));
    expect(create).toHaveBeenCalledWith({ path: "C:/Users/j/Pictures/Lumen Library" });
    await userEvent.click(await screen.findByRole("button", { name: "Skip for now" }));
    expect(await screen.findByText("home")).toBeInTheDocument();
  });
});
