import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, useLocation } from "react-router";
import { MotionGlobalConfig } from "motion/react";
import { describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { Toaster } from "@/components/toast";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { rememberPage } from "@/lib/return-to";
import { getDateOrder, setDateOrder } from "@/utils/date";
import { SettingsRoute } from "./gui";

// A restored face tile fades out before it leaves; jsdom never finishes an exit otherwise.
MotionGlobalConfig.skipAnimations = true;

const settings = {
  version: 1,
  appearance: { theme: "gallery", scheme: "auto", tileSize: "m", reduceMotion: false, sidebarCollapsed: false },
  import: { duplicates: "skip", promptOnVolume: true },
  playback: { loopVideos: false, muteByDefault: false },
  map: { online: true, style: "map" as const },
  music: { jamendoClientId: null },
  smart: { enabled: true, gpu: true, experimental: { autoModel: false, retouch: false } },
  window: { width: 1280, height: 800, x: null, y: null, maximized: false },
};

/** Where the router is, for the tests that follow the URL. */
function Here() {
  const l = useLocation();
  return <output data-testid="here">{`${l.pathname}${l.search}${l.hash}`}</output>;
}

function mountSettings(mlStatus?: object, at = "/settings", setup?: () => void, withToaster = false) {
  mockCommand("get_settings", () => settings);
  mockCommand("update_settings", (a) => a?.settings);
  mockLibraryDefaults();
  setup?.();
  if (mlStatus) mockCommand("get_ml_status", () => mlStatus);
  mockCommand("get_current_library", () => ({ id: "a", name: "Lumen Library", path: "D:/Pics/Lumen Library", schemaVersion: 2 }));
  mockCommand("list_libraries", () => ({ libraries: [{ entry: { id: "a", name: "Lumen Library", path: "D:/Pics/Lumen Library", lastOpenedAt: 1 }, available: true }], current: "a" }));
  mockCommand("get_app_info", () => ({
    version: "0.1.0",
    platform: "windows",
    appDataDir: "C:/AppData/JL-Studios/Lumen",
    defaultLibraryDir: "x",
    isDebug: true,
  }));
  render(
    <Providers>
      <MemoryRouter initialEntries={[at]}>
        <SettingsRoute />
        <Here />
        {withToaster && <Toaster />}
      </MemoryRouter>
    </Providers>,
  );
}

/** The sidebar: one entry per section under group headings. */
const sidebar = async () => within(await screen.findByRole("navigation", { name: "Settings" }));

/** Opens a themed `Select` and picks an option by its label. */
const choose = async (box: HTMLElement, option: string) => {
  await userEvent.click(box);
  await userEvent.click(await screen.findByRole("option", { name: option }));
};

/** Opens a themed `Select` so its options can be read. */
const openList = async (box: HTMLElement) => {
  await userEvent.click(box);
  return within(await screen.findByRole("listbox"));
};

/** Opens a section as a click on its sidebar entry would. */
const open = async (name: RegExp) => {
  const nav = await sidebar();
  const label = new RegExp(`^${name.source.replace(/^\^/, "")}`, name.flags);
  return userEvent.click(nav.getAllByRole("button").find((b) => label.test(b.querySelector(".settings-nav__label")?.textContent ?? "")) ?? nav.getByRole("button", { name }));
};

describe("SettingsRoute", () => {
  // Round 11: the date order, detected from the system and changeable.
  describe("search & dates", () => {
    it("names what Automatic resolves to and follows a chosen order", async () => {
      mountSettings(undefined, "/settings?section=search");
      const select = await screen.findByRole("combobox", { name: "Date order" });
      await waitFor(() => expect(select).toHaveValue("Automatic (Day-month-year)"));
      expect(screen.getByText("4.6.2026 means 4 June 2026")).toBeInTheDocument();
      await choose(select, "Month-day-year");
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("update_settings", expect.objectContaining({ settings: expect.objectContaining({ search: expect.objectContaining({ dateOrder: "mdy" }) }) })));
      expect(await screen.findByText("4.6.2026 means 6 April 2026")).toBeInTheDocument();
      await waitFor(() => expect(getDateOrder()).toBe("mdy"));
      setDateOrder("dmy");
    });
  });

  // Round 17: Adobe DNG Converter for the RAWs Lumen can't read itself, and its DNG cache.
  describe("RAW formats", () => {
    const status = (converter: string, cacheBytes = 0) => ({
      converter,
      converterPath: converter === "found" ? "C:/Program Files/Adobe/Adobe DNG Converter/Adobe DNG Converter.exe" : null,
      cacheBytes,
      cacheFiles: cacheBytes > 0 ? 3 : 0,
      envVar: "LUMEN_DNG_CONVERTER",
    });

    it("offers Adobe's page when the converter isn't installed", async () => {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      mountSettings(undefined, "/settings?section=raw-formats", () => mockCommand("get_raw_formats_status", () => status("not_found")));
      await userEvent.click(await screen.findByRole("button", { name: "Get Adobe DNG Converter" }));
      expect(openUrl).toHaveBeenCalledWith(expect.stringMatching(/^https:\/\/helpx\.adobe\.com\//));
      // In the row, and in the inspector since the click pointed at that row.
      expect(screen.getAllByText(/open as the camera's JPEG/)[0]).toBeInTheDocument();
    });

    it("shows where it was found and clears the converted copies", async () => {
      let bytes = 12 * 1024 ** 3;
      mountSettings(undefined, "/settings?section=raw-formats", () => {
        mockCommand("get_raw_formats_status", () => status("found", bytes));
        mockCommand("clear_dng_cache", () => {
          const freed = bytes;
          bytes = 0;
          return freed;
        });
      });
      expect(await screen.findByText(/Adobe DNG Converter\.exe$/)).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Get Adobe DNG Converter" })).not.toBeInTheDocument();
      expect(screen.getByText("12 GB")).toBeInTheDocument();
      await userEvent.click(screen.getByRole("button", { name: "Clear" }));
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("clear_dng_cache", undefined));
      await waitFor(() => expect(screen.getByRole("button", { name: "Clear" })).toBeDisabled());
    });

    it("says Adobe makes no Linux version", async () => {
      mountSettings(undefined, "/settings?section=raw-formats", () => mockCommand("get_raw_formats_status", () => status("no_linux_version")));
      expect(await screen.findByText(/Adobe makes no Linux version/)).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Get Adobe DNG Converter" })).not.toBeInTheDocument();
    });
  });

  // Round 6: Settings fills the window; Back (or Esc) returns to the page it was opened from.
  describe("back", () => {
    it("goes back to the page Settings was opened from", async () => {
      rememberPage("/albums/3?asset=9");
      mountSettings();
      await userEvent.click(await screen.findByRole("link", { name: "Back" }));
      expect(screen.getByTestId("here")).toHaveTextContent("/albums/3?asset=9");
    });

    it("goes Home when Settings was opened first, and Esc does the same", async () => {
      rememberPage(null);
      mountSettings();
      await screen.findByRole("link", { name: "Back" });
      await userEvent.keyboard("{Escape}");
      expect(screen.getByTestId("here")).toHaveTextContent(/^\/$/);
    });
  });

  // Round 5: one section at a time, chosen from a list that shows each one's state.
  describe("sections", () => {
    it("lists every section under six group headings, one page each", async () => {
      mountSettings();
      const nav = await sidebar();
      expect([...document.querySelectorAll(".settings-nav__heading")].map((h) => h.textContent)).toEqual(["General", "Library", "Smart features", "Connections", "Plugins", "About"]);
      expect(nav.getAllByRole("button").map((b) => b.querySelector(".settings-nav__label")?.textContent)).toEqual([
        "Appearance",
        "Search & dates",
        "Keyboard",
        "Library",
        "Storage",
        "Backup",
        "RAW formats",
        "Playback & codecs",
        "Models",
        "Experimental",
        "People",
        "AI assistants",
        "Map",
        "Music",
        "External apps",
        "Plugins",
        "Updates",
        "About",
        "Licences",
      ]);
      expect(nav.getByRole("button", { name: /^Appearance/ })).toHaveAttribute("aria-current", "page");
      expect(screen.queryByRole("tab")).toBeNull();
      expect(screen.getByRole("switch", { name: "Reduce motion" })).toBeInTheDocument();
      await open(/Music/);
      expect(screen.getByLabelText("Jamendo client id")).toBeInTheDocument();
      expect(screen.getByRole("heading", { level: 2, name: "Music" })).toBeInTheDocument();
      expect(screen.queryByRole("switch", { name: "Reduce motion" })).toBeNull();
      expect(screen.getByTestId("here")).toHaveTextContent("/settings?section=music");
    });

    it.each(["appearance", "search", "keyboard", "library", "backup", "raw-formats", "playback", "smart", "experimental", "people", "ai-assistants", "map", "music", "updates", "about", "licences"])(
      "opens ?section=%s",
      async (id) => {
        mountSettings(undefined, `/settings?section=${id}`);
        const nav = await sidebar();
        const current = nav.getAllByRole("button").filter((b) => b.getAttribute("aria-current") === "page");
        expect(current.map((b) => b.id)).toEqual([`settings-section-${id}`]);
      },
    );

    it("follows an old #anchor", async () => {
      mountSettings(undefined, "/settings#playback");
      expect(await screen.findByRole("switch", { name: "Loop videos" })).toBeInTheDocument();
    });

    it("moves between sections in the sidebar with the up and down arrow keys", async () => {
      mountSettings();
      const nav = await sidebar();
      nav.getByRole("button", { name: /^Appearance/ }).focus();
      await userEvent.keyboard("{ArrowDown}");
      expect(nav.getByRole("button", { name: /^Search & dates/ })).toHaveAttribute("aria-current", "page");
      expect(nav.getByRole("button", { name: /^Search & dates/ })).toHaveFocus();
      await userEvent.keyboard("{ArrowUp}{ArrowUp}");
      expect(nav.getByRole("button", { name: /^Licences/ })).toHaveAttribute("aria-current", "page");
    });

    it("shows Lumen's logo beside its version in About (Round 8B)", async () => {
      mountSettings();
      await open(/About/);
      expect(await screen.findByText(/Lumen 0\.1\.0/, { selector: ".settings__value" })).toBeInTheDocument();
      expect(document.querySelector(".settings__about img.brand-mark")).not.toBeNull();
    });

    it("about shows creator, licence and source link", async () => {
      mountSettings();
      await open(/About/);
      expect(await screen.findByText("Created by Julian · JL Studios")).toBeInTheDocument();
      expect(screen.getByText("Free and open source · GPL-3.0-or-later")).toBeInTheDocument();
      expect(screen.getByRole("link", { name: "Source code" })).toHaveAttribute("href", "https://github.com/05Narwhal/lumen");
    });

    it("tells each section's state in the sidebar", async () => {
      mountSettings();
      const nav = await sidebar();
      expect(await nav.findByRole("button", { name: /^Appearance.*Gallery/ })).toBeInTheDocument();
      expect(nav.getByRole("button", { name: /^Map.*Online detail/ })).toBeInTheDocument();
      expect(await nav.findByRole("button", { name: /^About.*Lumen 0\.1\.0/ })).toBeInTheDocument();
    });
  });

  describe("backup (Round 5A)", () => {
    const backedUp = {
      config: {
        destination: "E:/",
        schedule: "weekly",
        lastSuccessAt: 1,
        last: { at: 1_790_000_000, error: null, report: { copied: 42, copiedBytes: 1_300_000_000, unchanged: 5100, retired: 0, failed: [], cancelled: false } },
      },
      target: "E:/Lumen Library",
      available: true,
      running: false,
    };

    it("asks where to back up when nothing is set up", async () => {
      mountSettings();
      expect(await (await sidebar()).findByRole("button", { name: /^Backup.*Not set up/ })).toBeInTheDocument();
      await open(/Backup/);
      expect(await screen.findByRole("button", { name: "Choose a folder or drive…" })).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Back up now" })).toBeNull();
    });

    it("shows where, how often and how the last run went, and backs up now", async () => {
      mountSettings();
      mockCommand("get_backup", () => backedUp);
      mockCommand("set_backup_schedule", (a) => ({ ...backedUp, config: { ...backedUp.config, schedule: a?.schedule } }));
      mockCommand("start_backup", () => null);
      await open(/Backup/);
      expect(await screen.findByText("E:/Lumen Library")).toBeInTheDocument();
      expect(screen.getByText(/Backed up 42 new files/)).toBeInTheDocument();
      expect(screen.getByRole("radio", { name: "Weekly" })).toHaveAttribute("aria-checked", "true");
      await userEvent.click(screen.getByRole("radio", { name: "Daily" }));
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_backup_schedule", { schedule: "daily" }));
      await userEvent.click(screen.getByRole("button", { name: "Back up now" }));
      await waitFor(() => expect(invokeMock.mock.calls.some(([cmd]) => cmd === "start_backup")).toBe(true));
    });

    it("says when the backup drive is not connected", async () => {
      mountSettings();
      mockCommand("get_backup", () => ({ ...backedUp, available: false }));
      await open(/Backup/);
      expect(await screen.findByText(/Connect the drive and the backup runs/)).toBeInTheDocument();
      expect(await (await sidebar()).findByRole("button", { name: /^Backup.*Drive not connected/ })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Back up now" })).toBeDisabled();
    });
  });

  it("lists the keyboard shortcuts and finds one", async () => {
    mountSettings();
    await open(/Keyboard/);
    const keyboard = await screen.findByRole("region", { name: "Quick editor" });
    expect(keyboard).toHaveTextContent("Rotate right");
    await userEvent.type(screen.getByRole("textbox", { name: "Find a shortcut" }), "undo");
    expect(screen.queryByRole("region", { name: "Viewer" })).not.toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Quick editor" })).toHaveTextContent("Undo");
    expect(screen.getByRole("region", { name: "Video trimmer" })).toHaveTextContent("Undo");
  });

  it("has a Licences section listing what Lumen ships", async () => {
    mountSettings(undefined, "/settings?section=licences");
    expect(await screen.findByRole("heading", { level: 2, name: "Licences" })).toBeInTheDocument();
    expect(await screen.findByRole("region", { name: "Programs Lumen runs" })).toHaveTextContent("FFmpeg");
  });

  it("shows the library path and toggles reduce motion", async () => {
    mountSettings();
    await open(/Library/);
    expect(await screen.findByText("D:/Pics/Lumen Library")).toBeInTheDocument();
    await open(/Appearance/);
    await userEvent.click(screen.getByRole("switch", { name: "Reduce motion" }));
    expect(invokeMock).toHaveBeenCalledWith(
      "update_settings",
      expect.objectContaining({
        settings: expect.objectContaining({ appearance: expect.objectContaining({ reduceMotion: true }) }),
      }),
    );
  });

  it("keeps exact place names off until turned on, and credits OpenStreetMap", async () => {
    mountSettings();
    await open(/^Map/);
    const toggle = await screen.findByRole("switch", { name: "Exact place names" });
    expect(toggle).not.toBeChecked();
    expect(screen.getByText(/never the photo, to OpenStreetMap/)).toBeInTheDocument();
    expect(screen.getByText(/© OpenStreetMap contributors/)).toBeInTheDocument();
    await userEvent.click(toggle);
    expect(invokeMock).toHaveBeenCalledWith(
      "update_settings",
      expect.objectContaining({ settings: expect.objectContaining({ exactPlaces: true }) }),
    );
  });

  describe("Detailed satellite imagery (Esri)", () => {
    it("is a password-style field that explains the person's own key, and saves it trimmed", async () => {
      mountSettings();
      await open(/^Map/);
      const field = await screen.findByLabelText("Detailed satellite imagery (Esri)");
      expect(field).toHaveAttribute("type", "password");
      expect(field).toBeEnabled();
      expect(screen.getByText(/Your own free ArcGIS Location Platform key from location\.arcgis\.com/)).toBeInTheDocument();
      expect(screen.getByText(/satellite imagery comes from EOX Sentinel-2 cloudless/)).toBeInTheDocument();
      await userEvent.type(field, "  AAPK-own-key  ");
      await userEvent.tab();
      expect(invokeMock).toHaveBeenCalledWith(
        "update_settings",
        expect.objectContaining({ settings: expect.objectContaining({ map: expect.objectContaining({ esriKey: "AAPK-own-key" }) }) }),
      );
    });

    it("saves null when the field is cleared or only spaces", async () => {
      mountSettings(undefined, "/settings", () => mockCommand("get_settings", () => ({ ...settings, map: { ...settings.map, esriKey: "AAPK-own-key" } })));
      await open(/^Map/);
      const field = await screen.findByLabelText("Detailed satellite imagery (Esri)");
      await waitFor(() => expect(field).toHaveValue("AAPK-own-key"));
      await userEvent.clear(field);
      await userEvent.type(field, "   ");
      await userEvent.tab();
      expect(invokeMock).toHaveBeenLastCalledWith(
        "update_settings",
        expect.objectContaining({ settings: expect.objectContaining({ map: expect.objectContaining({ esriKey: null }) }) }),
      );
    });

    it("is disabled while the online map is off", async () => {
      mountSettings(undefined, "/settings", () => mockCommand("get_settings", () => ({ ...settings, map: { online: false, style: "map" as const } })));
      await open(/^Map/);
      expect(await screen.findByLabelText("Detailed satellite imagery (Esri)")).toBeDisabled();
    });
  });

  it("uses the graphics card for slideshow exports unless told not to (Round 4F)", async () => {
    mountSettings();
    await open(/Playback/);
    const toggle = await screen.findByRole("switch", { name: "Use the graphics card for slideshow exports" });
    expect(toggle).toBeChecked();
    await userEvent.click(toggle);
    expect(invokeMock).toHaveBeenCalledWith(
      "update_settings",
      expect.objectContaining({ settings: expect.objectContaining({ playback: expect.objectContaining({ hardwareEncoding: false }) }) }),
    );
  });

  it("prepares videos in the background unless told not to (Round 9A)", async () => {
    mountSettings();
    await open(/Playback/);
    const toggle = await screen.findByRole("switch", { name: "Prepare videos in the background" });
    expect(toggle).toBeChecked();
    await userEvent.click(toggle);
    expect(invokeMock).toHaveBeenCalledWith(
      "update_settings",
      expect.objectContaining({ settings: expect.objectContaining({ playback: expect.objectContaining({ prepareVideos: false }) }) }),
    );
  });

  it("marks the videos Lumen makes unless told not to (Round 8A)", async () => {
    mountSettings();
    await open(/Playback/);
    const toggle = await screen.findByRole("switch", { name: "Add the Lumen mark to videos Lumen makes" });
    expect(toggle).toBeChecked();
    await userEvent.click(toggle);
    expect(invokeMock).toHaveBeenCalledWith(
      "update_settings",
      expect.objectContaining({ settings: expect.objectContaining({ export: expect.objectContaining({ watermark: false }) }) }),
    );
  });

  it("saves a Jamendo client id, and says nothing is asked for without one", async () => {
    mountSettings();
    await open(/Music/);
    expect(await screen.findByText(/devportal\.jamendo\.com/)).toBeInTheDocument();
    const field = screen.getByLabelText("Jamendo client id");
    expect(field).toHaveAttribute("type", "password");
    await userEvent.type(field, "abc123");
    await userEvent.tab();
    expect(invokeMock).toHaveBeenCalledWith(
      "update_settings",
      expect.objectContaining({ settings: expect.objectContaining({ music: { jamendoClientId: "abc123" } }) }),
    );
  });

  it("says the Jamendo client id is the person's own and that using it accepts Jamendo's terms", async () => {
    mountSettings();
    await open(/Music/);
    expect(await screen.findByText(/your own, free from/)).toBeInTheDocument();
    expect(screen.getByText(/using it means accepting/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("link", { name: "Jamendo's API terms" }));
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    expect(openUrl).toHaveBeenCalledWith("https://devportal.jamendo.com/api_terms_of_use");
  });

  // Round 17: the opt-in Discord status.
  describe("Discord status", () => {
    const info = (over: object = {}) => ({ available: true, state: "off", preview: "Competing in Lumen · Browsing the library · with elapsed time", ...over });

    it("is hidden when this build has no Application ID", async () => {
      mockCommand("discord_info", () => info({ available: false }));
      mountSettings(undefined, "/settings?section=discord");
      await (await sidebar()).findByRole("button", { name: /^Appearance/ });
      await waitFor(() => expect(invokeMock.mock.calls.some(([c]) => c === "discord_info")).toBe(true));
      expect(screen.queryByRole("button", { name: /^Discord status/ })).toBeNull();
      expect(screen.queryByRole("switch", { name: "Show Lumen in my Discord status" })).toBeNull();
    });

    it("is off by default, turns on, and shows what others see", async () => {
      mockCommand("discord_info", () => info());
      mountSettings(undefined, "/settings?section=discord");
      mockCommand("update_settings", (a) => ({ ...(a?.settings as object), discord: { enabled: true, showActivity: true, showElapsed: true } }));
      const toggle = await screen.findByRole("switch", { name: "Show Lumen in my Discord status" });
      expect(toggle).not.toBeChecked();
      expect(screen.getByRole("switch", { name: "Show what I'm doing" })).toBeDisabled();
      expect(screen.getByRole("switch", { name: "Show elapsed time" })).toBeDisabled();
      expect(screen.getByText("Off. Nothing is shared.")).toBeInTheDocument();

      await userEvent.click(toggle);
      expect(invokeMock).toHaveBeenCalledWith("update_settings", expect.objectContaining({ settings: expect.objectContaining({ discord: expect.objectContaining({ enabled: true }) }) }));
      expect(await screen.findByText("Competing in Lumen · Browsing the library · with elapsed time")).toBeInTheDocument();
      expect(screen.getByRole("switch", { name: "Show what I'm doing" })).toBeEnabled();
    });
  });

  // Round 16: the opt-in MCP server.
  describe("AI assistants", () => {
    it("is off by default, shows each client's setup with the port and key, and makes a new key", async () => {
      mountSettings(undefined, "/settings?section=ai-assistants");
      mockCommand("mcp_status", () => ({ state: "waiting", client: null, message: null, exe: "C:/Apps/Lumen.exe" }));
      mockCommand("update_settings", (a) => ({ ...(a?.settings as object), aiAssistants: { enabled: true, port: 47820, token: "abc" } }));
      mockCommand("mcp_regenerate_token", () => ({ ...settings, aiAssistants: { enabled: true, port: 47820, token: "def" } }));
      const toggle = await screen.findByRole("switch", { name: "Let AI assistants edit in Develop" });
      expect(toggle).not.toBeChecked();
      expect(screen.getByText("Off. No port is open.")).toBeInTheDocument();
      expect(screen.queryByLabelText("Claude Code setup")).toBeNull();

      await userEvent.click(toggle);
      expect(invokeMock).toHaveBeenCalledWith("update_settings", expect.objectContaining({ settings: expect.objectContaining({ aiAssistants: expect.objectContaining({ enabled: true }) }) }));
      const code = await screen.findByLabelText("Claude Code setup");
      expect(code).toHaveTextContent("http://127.0.0.1:47820/mcp --header \"Authorization: Bearer abc\"");
      expect(screen.getByLabelText("Codex setup")).toHaveTextContent('http_headers = { "Authorization" = "Bearer abc" }');
      expect(screen.getByLabelText("Claude Desktop setup")).toHaveTextContent('"command": "C:/Apps/Lumen.exe"');
      expect(await screen.findByText("Waiting for a connection on port 47820.")).toBeInTheDocument();

      await userEvent.click(screen.getByRole("button", { name: "New key" }));
      await waitFor(() => expect(screen.getByLabelText("Claude Code setup")).toHaveTextContent("Bearer def"));
    });
  });

  // Round 23 A4: what plugins add to the server.
  describe("AI assistants plugin tools", () => {
    const withCount = async (n: number) => {
      mountSettings(undefined, "/settings?section=ai-assistants");
      mockCommand("mcp_status", () => ({ state: "waiting", client: null, message: null, exe: "C:/Apps/Lumen.exe" }));
      mockCommand("plugin_tool_count", () => n);
      mockCommand("update_settings", (a) => ({ ...(a?.settings as object), aiAssistants: { enabled: true, port: 47820, token: "abc" } }));
      await userEvent.click(await screen.findByRole("switch", { name: "Let AI assistants edit in Develop" }));
    };

    it("shows_plugin_tool_count", async () => {
      await withCount(3);
      expect(await screen.findByText("Plugins add 3 tools")).toBeInTheDocument();
    });

    it("says tool in the singular for one", async () => {
      await withCount(1);
      expect(await screen.findByText("Plugins add 1 tool")).toBeInTheDocument();
    });

    it("hides the line at zero and while AI assistants are off", async () => {
      await withCount(0);
      await screen.findByLabelText("Claude Code setup");
      expect(screen.queryByText(/Plugins add/)).toBeNull();
    });

    it("is hidden while the switch is off even when plugins have tools", async () => {
      mountSettings(undefined, "/settings?section=ai-assistants");
      mockCommand("mcp_status", () => ({ state: "off", client: null, message: null, exe: null }));
      mockCommand("plugin_tool_count", () => 3);
      await screen.findByRole("switch", { name: "Let AI assistants edit in Develop" });
      expect(screen.queryByText(/Plugins add/)).toBeNull();
    });
  });

  describe("AI assistants connect", () => {
    it("connects each assistant in one click and says what happened", async () => {
      mountSettings(undefined, "/settings?section=ai-assistants");
      mockCommand("mcp_status", () => ({ state: "waiting", client: null, message: null, exe: "C:/Apps/Lumen.exe" }));
      mockCommand("update_settings", (a) => ({ ...(a?.settings as object), aiAssistants: { enabled: true, port: 47820, token: "abc" } }));
      mockCommand("mcp_connect", (a) => `Connected ${a?.client as string}`);
      await userEvent.click(await screen.findByRole("switch", { name: "Let AI assistants edit in Develop" }));
      await userEvent.click(await screen.findByRole("button", { name: "Connect Codex" }));
      expect(invokeMock).toHaveBeenCalledWith("mcp_connect", { client: "codex" });
      await userEvent.click(screen.getByRole("button", { name: "Connect Claude Code" }));
      expect(invokeMock).toHaveBeenCalledWith("mcp_connect", { client: "claude-code" });
      await userEvent.click(screen.getByRole("button", { name: "Connect Claude Desktop" }));
      expect(invokeMock).toHaveBeenCalledWith("mcp_connect", { client: "claude-desktop" });
    });
  });

  describe("Smart features", () => {
    const status = (ready: boolean, enabled = true) => ({
      enabled,
      models: [
        { id: "clip-vit-b32-fp16@1", role: "image_embedding", bytes: 176_214_230, ready },
        { id: "beat-this-small@1", role: "beat_activation", bytes: 9_274_894, ready },
      ],
      embedded: 5,
      photos: 10,
      faces: 0,
      people: 0,
    });

    it("says how many photos the models have seen", async () => {
      mountSettings(status(true), "/settings?section=smart");
      expect(await screen.findByText(/5 of 10 photos analysed/)).toBeInTheDocument();
    });

    it("says what is still to download", async () => {
      mountSettings(status(false), "/settings?section=smart");
      expect(await screen.findByText(/185\.5 MB to download/)).toBeInTheDocument();
    });

    it("switches Denoise off the graphics card", async () => {
      mountSettings(status(true), "/settings?section=smart");
      await userEvent.click(await screen.findByRole("switch", { name: "Use the graphics card for Denoise" }));
      expect(invokeMock).toHaveBeenCalledWith(
        "update_settings",
        expect.objectContaining({ settings: expect.objectContaining({ smart: expect.objectContaining({ gpu: false }) }) }),
      );
    });

    it("turns off, and turning back on fetches at once", async () => {
      mountSettings(status(true), "/settings?section=smart");
      const toggle = await screen.findByRole("switch", { name: "Smart features" });
      await userEvent.click(toggle);
      expect(invokeMock).toHaveBeenCalledWith(
        "update_settings",
        expect.objectContaining({ settings: expect.objectContaining({ smart: expect.objectContaining({ enabled: false }) }) }),
      );
      await userEvent.click(toggle);
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("fetch_models", undefined));
    });

    // Round 14: the Experimental switches moved to their own tab.
    it("no longer holds the Experimental switches", async () => {
      mountSettings(status(true), "/settings?section=smart");
      await screen.findByRole("switch", { name: "Smart features" });
      expect(screen.queryByRole("switch", { name: /Auto correction|Retouch/ })).not.toBeInTheDocument();
    });
  });

  // Round 14: Settings → Experimental, a model and a performance tier per task.
  describe("Experimental", () => {
    const tier = (t: string, ms: number | null, extra: object = {}) => ({ tier: t, ready: true, reason: null, precision: "fp32", tta: t === "ultra" ? 5 : 1, valDe: null, ms, ...extra });
    const ray = {
      id: "lumen-ray-1.0.1-d",
      name: "Lumen Ray 1.0.1-d",
      version: "1.0.1-d",
      legacy: false,
      tiers: [tier("low", 90, { precision: "int8" }), tier("high", 180, { valDe: 3.94 }), tier("ultra", 4100)],
    };
    const row = (task: string, family: string, extra: object = {}) => ({
      task,
      family,
      switchable: task === "auto" || task === "retouch",
      builtin: null,
      models: [],
      model: null,
      recommended: "high",
      testing: false,
      tier: null,
      budgetMs: task === "auto" ? 1000 : task === "retouch" ? 1500 : null,
      problem: null,
      ...extra,
    });
    const catalog = (auto: object = {}) => [
      row("auto", "ray", { models: [ray], model: ray.id, tier: "high", ...auto }),
      row("retouch", "prism"),
      row("search", "spectrum", { builtin: "Lumen Spectrum 1.0 (based on CLIP ViT-B/32)" }),
      row("text", "glyph"),
      row("masks", "facet"),
    ];
    const mountExperimental = (rows: object[]) => {
      mockCommand("experimental_catalog", () => rows);
      mockCommand("retest_models", () => null);
      mountSettings(undefined, "/settings?section=experimental");
    };

    it("greys families with no model", async () => {
      mountExperimental(catalog());
      const model = await screen.findByRole("combobox", { name: "Text in photos (Glyph) model" });
      expect(model).toBeDisabled();
      expect(screen.getByRole("combobox", { name: "AI masks (Facet) model" })).toBeDisabled();
      expect(model).toHaveValue("No model yet");
      expect(screen.getByRole("combobox", { name: "AI masks (Facet) model" })).toHaveValue("No model yet");
      expect(screen.getByRole("combobox", { name: "Text in photos (Glyph) performance" })).toHaveValue("No model yet");
      expect(screen.getByRole("combobox", { name: "Auto correction (Ray) model" })).toBeEnabled();
    });

    it("shows Retouch as In development without Prism", async () => {
      mountExperimental(catalog());
      expect(await screen.findByText("In development")).toBeInTheDocument();
      expect(screen.getByRole("switch", { name: "Retouch (Prism)" })).toBeDisabled();
      expect(screen.getByRole("switch", { name: "Auto correction (Ray)" })).toBeEnabled();
    });

    it("names the recommendation in Automatic and lists each tier's time", async () => {
      mountExperimental(catalog({ recommended: "ultra" }));
      const list = await openList(await screen.findByRole("combobox", { name: "Auto correction (Ray) performance" }));
      expect(list.getByRole("option", { name: "Automatic (Ultra)" })).toBeInTheDocument();
      expect(list.getByRole("option", { name: "Low · 90 ms" })).toBeInTheDocument();
      expect(list.getByRole("option", { name: "High · 180 ms · ΔE 3.9" })).toBeInTheDocument();
      expect(list.getByRole("option", { name: "Ultra · 4.1 s" })).toBeInTheDocument();
    });

    it("warns when the pick is over budget, and saves it", async () => {
      mountExperimental(catalog());
      const perf = await screen.findByRole("combobox", { name: "Auto correction (Ray) performance" });
      await choose(perf, "Ultra · 4.1 s");
      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith(
          "update_settings",
          expect.objectContaining({
            settings: expect.objectContaining({
              smart: expect.objectContaining({ experimental: expect.objectContaining({ tasks: expect.objectContaining({ auto: { model: null, tier: "ultra" } }) }) }),
            }),
          }),
        ),
      );
      expect(await screen.findByText("Ultra takes about 4.1 s here; Lumen recommends High.")).toBeInTheDocument();
    });

    // Round 13: reading text in photos is switched on the Glyph row, off by default.
    it("switches reading text on once the text models are there", async () => {
      const glyph = "Lumen Glyph 1.0.0-d (based on PP-OCRv5)";
      mountExperimental(catalog().map((r) => (r.task === "text" ? { ...r, switchable: true, builtin: glyph } : r)));
      const sw = await screen.findByRole("switch", { name: "Text in photos (Glyph)" });
      expect(sw).toBeEnabled();
      expect(sw).not.toBeChecked();
      expect(screen.getByText(`Ready · ${glyph}`)).toBeInTheDocument();
      await userEvent.click(sw);
      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith(
          "update_settings",
          expect.objectContaining({
            settings: expect.objectContaining({ smart: expect.objectContaining({ experimental: expect.objectContaining({ readText: true }) }) }),
          }),
        ),
      );
    });

    it("switches AI masks on before Facet is here, which fetches it", async () => {
      mountExperimental(catalog().map((r) => (r.task === "masks" ? { ...r, switchable: true } : r)));
      const sw = await screen.findByRole("switch", { name: "AI masks (Facet)" });
      expect(sw).toBeEnabled();
      expect(screen.getByText("Downloads Lumen Facet (48 MB) once on")).toBeInTheDocument();
      await userEvent.click(sw);
      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith(
          "update_settings",
          expect.objectContaining({
            settings: expect.objectContaining({ smart: expect.objectContaining({ experimental: expect.objectContaining({ aiMasks: true }) }) }),
          }),
        ),
      );
    });

    it("says why the Facet download failed", async () => {
      const problem = "Couldn't download Lumen Facet 1.0.0-d: it isn't on the download server yet. Turn the switch off and on to try again.";
      mountExperimental(catalog().map((r) => (r.task === "masks" ? { ...r, switchable: true, problem } : r)));
      expect(await screen.findByText(problem)).toBeInTheDocument();
      expect(screen.queryByText("Downloads Lumen Facet (48 MB) once on")).not.toBeInTheDocument();
    });

    it("keeps reading text off while its models are missing", async () => {
      mountExperimental(catalog().map((r) => (r.task === "text" ? { ...r, switchable: true } : r)));
      expect(await screen.findByRole("switch", { name: "Text in photos (Glyph)" })).toBeDisabled();
      expect(screen.getByText("Needs the text models")).toBeInTheDocument();
    });

    it("says where each tier was timed, and which could not run here", async () => {
      const tiers = [
        tier("low", null, { precision: "int8", failed: "inference failed: 80070057" }),
        tier("high", 22, { ep: "directml" }),
        tier("ultra", 412, { ep: "cpu" }),
      ];
      mountExperimental(catalog({ models: [{ ...ray, tiers }] }));
      const list = await openList(await screen.findByRole("combobox", { name: "Auto correction (Ray) performance" }));
      expect(list.getByRole("option", { name: "High · 22 ms on the graphics card" })).toBeInTheDocument();
      expect(list.getByRole("option", { name: "Ultra · 412 ms on the processor" })).toBeInTheDocument();
      expect(list.getByRole("option", { name: "Low · Could not run here" })).toHaveAttribute("data-combobox-disabled", "true");
      expect(screen.getByText("Low could not run here: inference failed: 80070057")).toBeInTheDocument();
    });

    it("says it is testing this PC while timings are missing", async () => {
      mountExperimental(catalog({ testing: true }));
      expect(await screen.findByText("Testing this PC…")).toBeInTheDocument();
    });

    it("shows Search as one size with its built-in model", async () => {
      mountExperimental(catalog());
      const model = await screen.findByRole("combobox", { name: "Search (Spectrum) model" });
      expect(model).toBeDisabled();
      expect(model).toHaveValue("Lumen Spectrum 1.0 (based on CLIP ViT-B/32)");
      expect(screen.getByRole("combobox", { name: "Search (Spectrum) performance" })).toHaveValue("One size");
    });

    it("re-tests on demand", async () => {
      mountExperimental(catalog());
      await userEvent.click(await screen.findByRole("button", { name: "Re-test" }));
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("retest_models", undefined));
    });
  });

  describe("People", () => {
    it("lists the people removed from People, and restores one", async () => {
      let removed = [{ id: 4, name: "Marko", faceCount: 9, photoCount: 7, coverFaceId: 12 }];
      mockCommand("list_removed_people", () => removed);
      mockCommand("restore_removed_person", () => {
        removed = [];
        return null;
      });
      mountSettings(undefined, "/settings?section=people");
      expect(await screen.findByText("Marko")).toBeInTheDocument();
      await userEvent.click(screen.getByRole("button", { name: "Restore Marko" }));
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("restore_removed_person", { id: 4 }));
      expect(await screen.findByText("No one has been removed from People.")).toBeInTheDocument();
    });

    it("shows the faces Lumen decided were not people, and brings one back", async () => {
      let rejected = [
        { faceId: 7, assetId: 3, reason: "print" },
        { faceId: 6, assetId: 2, reason: "user" },
      ];
      mockCommand("list_rejected_faces", () => rejected);
      mockCommand("restore_face", (a) => {
        rejected = rejected.filter((f) => f.faceId !== a?.face);
        return null;
      });
      mountSettings(undefined, "/settings?section=people");
      expect(await screen.findByText("2 faces set aside")).toBeInTheDocument();
      const buttons = screen.getAllByRole("button", { name: /This is a real face/ });
      expect(buttons[0]).toHaveAccessibleName("This is a real face (a printed photo)");
      await userEvent.click(buttons[0]);
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("restore_face", { face: 7 }));
      expect(await screen.findByText("1 face set aside")).toBeInTheDocument();
    });

    it("selects several removed people and restores them together", async () => {
      mockCommand("list_removed_people", () => [
        { id: 4, name: "Marko", faceCount: 9, photoCount: 7, coverFaceId: 12 },
        { id: 5, name: "Ana", faceCount: 3, photoCount: 2, coverFaceId: 13 },
      ]);
      mockCommand("restore_removed_person", () => null);
      mountSettings(undefined, "/settings?section=people");
      await userEvent.click(await screen.findByRole("button", { name: "Select Marko" }));
      await userEvent.click(screen.getByRole("button", { name: "Select Ana" }));
      expect(screen.getByText("2 selected")).toBeInTheDocument();
      await userEvent.click(screen.getByRole("button", { name: "Restore 2" }));
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("restore_removed_person", { id: 5 }));
      expect(invokeMock).toHaveBeenCalledWith("restore_removed_person", { id: 4 });
    });
  });

  describe("Search and inspector (2026-11)", () => {
    it("filters rows across sections, marks the match and counts hits per section", async () => {
      mountSettings();
      await userEvent.type(await screen.findByRole("searchbox", { name: "Search settings" }), "reduce motion");
      const results = await screen.findByRole("region", { name: "Search results" });
      expect(within(results).getByText("Reduce motion", { selector: "mark" })).toBeInTheDocument();
      expect(within(results).queryByText("Default tile size")).not.toBeInTheDocument();
      await waitFor(() => expect(screen.getByLabelText("1 match")).toBeInTheDocument());
    });

    it("says so when nothing matches, and clears", async () => {
      mountSettings();
      await userEvent.type(await screen.findByRole("searchbox", { name: "Search settings" }), "zzqx");
      expect(await screen.findByText("No settings match “zzqx”.")).toBeInTheDocument();
      await userEvent.click(screen.getByRole("button", { name: "Clear search" }));
      expect(screen.getByRole("searchbox", { name: "Search settings" })).toHaveValue("");
    });

    it("focuses the search box on Ctrl+F", async () => {
      mountSettings();
      const box = await screen.findByRole("searchbox", { name: "Search settings" });
      await userEvent.keyboard("{Control>}f{/Control}");
      expect(box).toHaveFocus();
    });

    it("shows the theme and mode in Appearance's inspector", async () => {
      mountSettings();
      const panel = await screen.findByRole("complementary", { name: "About Appearance" });
      expect(within(panel).getByText("Theme").nextElementSibling).toHaveTextContent("Gallery");
      expect(within(panel).getByText("Mode").nextElementSibling).toHaveTextContent(/^Automatic/);
    });

    it("shows how much the models have analysed", async () => {
      mountSettings({ enabled: true, models: [], embedded: 48, photos: 50, faces: 9, people: 3 }, "/settings?section=smart");
      const panel = await screen.findByRole("complementary", { name: "About Models" });
      expect(await within(panel).findByText("96%")).toBeInTheDocument();
      expect(within(panel).getByRole("meter", { name: "Analysed" })).toHaveAttribute("aria-valuenow", "96");
      expect(within(panel).getByText("Photos").nextElementSibling).toHaveTextContent("48 / 50");
      expect(within(panel).getByText("People").nextElementSibling).toHaveTextContent("3");
    });

    it("counts the library's items in its inspector", async () => {
      mountSettings(undefined, "/settings?section=library", () => mockCommand("get_library_stats", () => ({ items: 21865, videos: 2463, bytes: 148e9 })));
      const panel = await screen.findByRole("complementary", { name: "About Library" });
      expect(await within(panel).findByText("21,865")).toBeInTheDocument();
      expect(within(panel).getByText("Photos").nextElementSibling).toHaveTextContent("19,402");
      expect(within(panel).getByText("Videos").nextElementSibling).toHaveTextContent("2,463");
    });

    it("shows the version and checks for updates from Updates' inspector", async () => {
      mountSettings(undefined, "/settings?section=updates", () => {
        mockCommand("update_status", () => ({ version: "26.1.0-alpha", state: { state: "idle" }, whatsNew: null }));
        mockCommand("check_for_updates", () => ({ version: "26.1.0-alpha", state: { state: "checking" }, whatsNew: null }));
      });
      const panel = await screen.findByRole("complementary", { name: "About Updates" });
      expect(await within(panel).findByText("26.1.0-alpha")).toBeInTheDocument();
      await userEvent.click(within(panel).getByRole("button", { name: "Check now" }));
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("check_for_updates", undefined));
    });

    it("counts removed people and restores the selected ones from People's inspector", async () => {
      mockCommand("list_removed_people", () => [
        { id: 4, name: "Marko", faceCount: 9, photoCount: 7, coverFaceId: 12 },
        { id: 5, name: "Ana", faceCount: 3, photoCount: 2, coverFaceId: 13 },
      ]);
      mockCommand("restore_removed_person", () => null);
      mountSettings(undefined, "/settings?section=people");
      const panel = await screen.findByRole("complementary", { name: "About People" });
      expect(await within(panel).findByText("2")).toBeInTheDocument();
      expect(within(panel).getByRole("button", { name: "Restore selected" })).toBeDisabled();
      await userEvent.click(await screen.findByRole("button", { name: "Select Marko" }));
      await userEvent.click(within(panel).getByRole("button", { name: "Restore 1 selected" }));
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("restore_removed_person", { id: 4 }));
      expect(invokeMock).not.toHaveBeenCalledWith("restore_removed_person", { id: 5 });
    });

    it("explains the row under the pointer in the inspector", async () => {
      mountSettings();
      const label = await screen.findByText("Reduce motion");
      await userEvent.hover(label);
      const panel = screen.getByRole("complementary", { name: "About Appearance" });
      expect(within(panel).getByText("Reduce motion")).toBeInTheDocument();
    });
  });
});

describe("External apps (Round 22)", () => {
  const app = (id: string, name: string, kind: string, accepts: string[], writes = "inPlace", custom = false) => ({ id, name, kind, accepts, iconUrl: `photos://app-icon/1`, custom, send: "automatic", writes });
  const apps = [
    app("photoshop", "Adobe Photoshop", "editor", ["photo", "raw"]),
    app("darktable", "darktable", "developer", ["photo", "raw"], "sidecar"),
    app("resolve", "DaVinci Resolve", "video", ["video"], "never"),
    app("custom-1", "Mine", "editor", ["photo", "raw", "video"], "inPlace", true),
  ];

  it("lists the defaults per media, the send choice and a removable custom app", async () => {
    let list = apps;
    mountSettings(undefined, "/settings?section=external-apps", () => {
      mockCommand("list_external_apps", () => list);
      mockCommand("get_default_apps", () => ({ photo: "photoshop", raw: null, video: null }));
      mockCommand("set_default_app", () => null);
      mockCommand("remove_custom_app", () => {
        list = apps.slice(0, 3);
        return null;
      });
    });
    const videos = await screen.findByRole("combobox", { name: "Default app for videos" });
    expect((await openList(videos)).getAllByRole("option").map((o) => o.textContent)).toEqual(["None", "DaVinci Resolve", "Mine"]);
    await userEvent.keyboard("{Escape}");
    expect(screen.getByRole("combobox", { name: "Default app for photos" })).toHaveValue("Adobe Photoshop");
    expect(screen.getByRole("combobox", { name: "What Adobe Photoshop is sent" })).toBeInTheDocument();
    expect(screen.getByText("Gets a copy")).toBeInTheDocument();
    expect(screen.getByText("Opens the original")).toBeInTheDocument();
    await choose(videos, "DaVinci Resolve");
    expect(invokeMock).toHaveBeenCalledWith("set_default_app", { media: "video", id: "resolve" });
    await userEvent.click(screen.getByRole("button", { name: "Remove Mine" }));
    expect(invokeMock).toHaveBeenCalledWith("remove_custom_app", { id: "custom-1" });
    await waitFor(() => expect(screen.queryByRole("button", { name: "Remove Mine" })).not.toBeInTheDocument());
  });

  it("added_by_label: a plugin's app says who added it", async () => {
    mountSettings(undefined, "/settings?section=external-apps", () => {
      mockCommand("list_external_apps", () => [
        app("photoshop", "Adobe Photoshop", "editor", ["photo", "raw"]),
        { ...app("plugin:com.darkroom.tools/fancy", "Fancy Editor", "editor", ["photo"]), addedBy: "Darkroom Tools" },
      ]);
      mockCommand("get_default_apps", () => ({ photo: null, raw: null, video: null }));
    });
    expect(await screen.findByText("added by Darkroom Tools")).toBeInTheDocument();
    expect(screen.getAllByText(/^added by /)).toHaveLength(1);
    expect(screen.getByRole("combobox", { name: "What Fancy Editor is sent" })).toBeInTheDocument();
  });

  it("says so when no editor is found and still offers Choose another app…", async () => {
    mountSettings(undefined, "/settings?section=external-apps", () => {
      mockCommand("list_external_apps", () => []);
      mockCommand("get_default_apps", () => ({ photo: null, raw: null, video: null }));
    });
    expect(await screen.findByText("No photo or video editor was found on this computer.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Choose another app…" })).toBeInTheDocument();
  });
});
