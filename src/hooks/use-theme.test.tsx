import { render, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { mockCommand, windowMock } from "@/test/tauri-mock";

describe("useApplyTheme", () => {
  it("stamps data-theme and reduced-motion from settings", async () => {
    mockCommand("get_settings", () => ({
      version: 1,
      appearance: { theme: "paper", scheme: "dark", tileSize: "m", reduceMotion: true, sidebarCollapsed: false },
      import: { duplicates: "skip", promptOnVolume: true },
      playback: { loopVideos: false, muteByDefault: false },
  map: { online: true, style: "map" as const },
      window: { width: 1280, height: 800, x: null, y: null, maximized: false },
    }));
    render(
      <Providers>
        <div />
      </Providers>,
    );
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe("paper"));
    // Remembered for index.html's inline script, so the next splash is already in this theme.
    expect(localStorage.getItem("lumen.theme")).toBe("paper");
    expect(document.documentElement.dataset.reducedMotion).toBe("true");
    expect(document.documentElement.getAttribute("data-mantine-color-scheme")).toBe("dark");
  });

  it("paints the native window the theme's base colour, so a maximized window's 1 px edge is not black", async () => {
    // A maximized borderless window leaves 1 px for an auto-hiding taskbar; that strip shows the
    // native window background, not the page.
    document.documentElement.style.setProperty("--bg-base", "oklch(12% 0 0)");
    mockCommand("get_settings", () => ({
      version: 1,
      appearance: { theme: "gallery", scheme: "dark", tileSize: "m", reduceMotion: false, sidebarCollapsed: false },
      import: { duplicates: "skip", promptOnVolume: true },
      playback: { loopVideos: false, muteByDefault: false },
      map: { online: true, style: "map" as const },
      window: { width: 1280, height: 800, x: null, y: null, maximized: false },
    }));
    render(
      <Providers>
        <div />
      </Providers>,
    );
    await waitFor(() => expect(windowMock.setBackgroundColor).toHaveBeenCalledWith([6, 6, 6, 255]));
    document.documentElement.style.removeProperty("--bg-base");
  });
});

describe("useApplyTheme with a plugin theme", () => {
  const NORD = { id: "a.b", name: "Nord", base: "darkroom", defaultScheme: "dark" };
  const settings = (theme: unknown) => ({
    version: 1,
    appearance: { theme, scheme: "dark", tileSize: "m", reduceMotion: false, sidebarCollapsed: false },
    import: { duplicates: "skip", promptOnVolume: true },
    playback: { loopVideos: false, muteByDefault: false },
    map: { online: true, style: "map" as const },
    window: { width: 1280, height: 800, x: null, y: null, maximized: false },
  });
  const mount = () =>
    render(
      <Providers>
        <div />
      </Providers>,
    );

  it("sets_plugin_attribute", async () => {
    delete document.documentElement.dataset.pluginTheme;
    mockCommand("get_settings", () => settings({ plugin: "a.b", theme: "theme" }));
    mockCommand("themes_list", () => [NORD]);
    mockCommand("plugin_theme_css", () => "[data-plugin-theme] { --accent: red; }");
    mount();
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe("darkroom"));
    expect(document.documentElement.dataset.pluginTheme).toBe("a.b/theme");
    await waitFor(() => expect(document.getElementById("plugin-themes")?.textContent).toBe("[data-plugin-theme] { --accent: red; }"));
  });

  it("missing_plugin_theme_falls_back", async () => {
    document.documentElement.dataset.theme = "darkroom";
    document.documentElement.dataset.pluginTheme = "a.b/nord";
    mockCommand("get_settings", () => settings({ plugin: "a.b", theme: "theme" }));
    mockCommand("themes_list", () => []);
    mockCommand("plugin_theme_css", () => "");
    mount();
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe("gallery"));
    expect(document.documentElement.dataset.pluginTheme).toBeUndefined();
  });

  it("a built-in choice carries no plugin attribute", async () => {
    document.documentElement.dataset.pluginTheme = "a.b/nord";
    mockCommand("get_settings", () => settings("paper"));
    mockCommand("themes_list", () => [NORD]);
    mockCommand("plugin_theme_css", () => "");
    mount();
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe("paper"));
    expect(document.documentElement.dataset.pluginTheme).toBeUndefined();
  });
});
