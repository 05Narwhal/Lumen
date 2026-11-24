import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { MemoryRouter, useLocation } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { ContextMenuHost } from "@/components/context-menu";
import { closePluginInstall, pluginInstallStore } from "@/components/plugin-install/store";
import { Toaster } from "@/components/toast";
import { toastStore } from "@/components/toast/store";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import { PluginsSection } from "./plugins";
import { SearchContext } from "./search";
import { GROUPS } from "./sections";

const info = (over: Partial<PluginInfo> = {}): PluginInfo =>
  ({
    id: "studio.jl.takeout",
    name: "Google Takeout",
    version: "1.2.0",
    author: "JL Studios",
    description: "",
    license: "MIT",
    trust: { type: "jlStudios" },
    kinds: ["importer", "presets"],
    permissions: [],
    enabled: true,
    dev: false,
    state: { type: "ready" },
    iconUrl: "photos://plugin-icon/takeout",
    settings: [],
    homepage: null,
    detectIds: [],
    sections: [],
    assets: [],
    ...over,
  }) as PluginInfo;

const calls = (name: string) => invokeMock.mock.calls.filter(([c]) => c === name);

function mount(list: PluginInfo[], developerMode = false) {
  let devMode = developerMode;
  mockCommand("list_plugins", () => list);
  mockCommand("get_plugin_developer_mode", () => devMode);
  mockCommand("set_plugin_enabled", () => undefined);
  mockCommand("remove_plugin", () => "tok-1");
  mockCommand("undo_remove_plugin", () => undefined);
  mockCommand("set_plugin_developer_mode", (a) => {
    devMode = (a as { on: boolean }).on;
  });
  mockCommand("load_dev_plugin", () => info({ dev: true }));
  return render(
    <Providers>
      <MemoryRouter>
        <PluginsSection />
        <ContextMenuHost />
        <Toaster />
      </MemoryRouter>
    </Providers>,
  );
}

describe("Settings → Plugins", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    closePluginInstall();
    toastStore.set({ toasts: [] });
    vi.mocked(openDialog).mockReset();
    vi.mocked(openDialog).mockResolvedValue(null);
  });

  it("sixth_group_after_connections", () => {
    const ids = GROUPS.map((g) => g.id);
    expect(ids).toHaveLength(6);
    expect(ids.indexOf("plugins")).toBe(ids.indexOf("connections") + 1);
  });

  it("empty_state", async () => {
    mount([]);
    expect(await screen.findByText("No plugins yet.")).toBeInTheDocument();
    vi.mocked(openDialog).mockResolvedValueOnce("C:/x/a.lumenplugin");
    await userEvent.click(screen.getByRole("button", { name: "Install from file…" }));
    await waitFor(() => expect(pluginInstallStore.get().path).toBe("C:/x/a.lumenplugin"));
    expect(vi.mocked(openDialog).mock.calls[0][0]).toMatchObject({ filters: [{ extensions: ["lumenplugin"] }] });
  });

  it("row_contents", async () => {
    const { container } = mount([info()]);
    expect(await screen.findByText("Google Takeout")).toBeInTheDocument();
    expect(screen.getByText("1.2.0")).toBeInTheDocument();
    expect(screen.getByText("✓ From JL Studios")).toBeInTheDocument();
    expect(screen.getByText("Importer")).toBeInTheDocument();
    expect(screen.getByText("Presets")).toBeInTheDocument();
    expect(container.querySelector("img")).toHaveAttribute("src", "photos://plugin-icon/takeout");
    expect(screen.getByRole("switch", { name: "Turn Google Takeout on" })).toBeChecked();
  });

  it("shows an empty box when there is no icon", async () => {
    const { container } = mount([info({ iconUrl: null })]);
    await screen.findByText("Google Takeout");
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector(".settings__plugin-icon--empty")).not.toBeNull();
  });

  it("toggle_calls_command", async () => {
    mount([info()]);
    await userEvent.click(await screen.findByRole("switch", { name: "Turn Google Takeout on" }));
    await waitFor(() => expect(calls("set_plugin_enabled")[0][1]).toEqual({ id: "studio.jl.takeout", on: false }));
  });

  it("remove_with_undo", async () => {
    mount([info()]);
    await userEvent.click(await screen.findByRole("button", { name: "More for Google Takeout" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Remove" }));
    await waitFor(() => expect(calls("remove_plugin")).toHaveLength(1));
    expect(screen.queryByRole("dialog")).toBeNull();
    await waitFor(() => expect(toastStore.get().toasts).toHaveLength(1));
    expect(toastStore.get().toasts[0].durationMs).toBe(6000);
    await userEvent.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() => expect(calls("undo_remove_plugin")[0][1]).toEqual({ token: "tok-1" }));
  });

  it("a row opens the plugin's page, and Back returns to the list", async () => {
    mockCommand("plugin_detect", () => []);
    mount([info({ description: "Reads Takeout folders." })]);
    await userEvent.click(await screen.findByRole("button", { name: "Google Takeout" }));
    expect(await screen.findByText("Reads Takeout folders.")).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Back to plugins" }));
    expect(await screen.findByText("Add a plugin")).toBeInTheDocument();
  });

  it("the menu's Details opens the page", async () => {
    mockCommand("plugin_detect", () => []);
    mount([info({ description: "Reads Takeout folders." })]);
    await userEvent.click(await screen.findByRole("button", { name: "More for Google Takeout" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Details" }));
    expect(await screen.findByText("Reads Takeout folders.")).toBeInTheDocument();
  });

  it("developer_mode_and_load_folder", async () => {
    mount([info()]);
    expect(await screen.findByRole("switch", { name: "Developer mode" })).not.toBeChecked();
    expect(screen.queryByRole("button", { name: "Load a folder…" })).toBeNull();
    await userEvent.click(screen.getByRole("switch", { name: "Developer mode" }));
    await waitFor(() => expect(calls("set_plugin_developer_mode")[0][1]).toEqual({ on: true }));
    vi.mocked(openDialog).mockResolvedValueOnce("C:/dev/my-plugin");
    await userEvent.click(await screen.findByRole("button", { name: "Load a folder…" }));
    expect(vi.mocked(openDialog).mock.calls[0][0]).toMatchObject({ directory: true });
    await waitFor(() => expect(calls("load_dev_plugin")[0][1]).toEqual({ folder: "C:/dev/my-plugin" }));
  });

  it("dev plugin shows the Dev badge, Reload and Log", async () => {
    mount([info({ dev: true })], true);
    expect(await screen.findByText("Dev")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "More for Google Takeout" }));
    const menu = await screen.findByRole("menu");
    expect(within(menu).getByRole("menuitem", { name: "Reload" })).toBeInTheDocument();
    expect(within(menu).getByRole("menuitem", { name: "Log" })).toBeInTheDocument();
  });

  it("problem_row", async () => {
    mount([info({ state: { type: "problem", reason: "Its theme has a broken token." } })]);
    expect(await screen.findByText("Its theme has a broken token.")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Turn Google Takeout on" })).toBeDisabled();
  });

  it("shows why a plugin stopped", async () => {
    mount([info({ state: { type: "stopped", reason: "ran out of time" } })]);
    expect(await screen.findByText(/Stopped working: ran out of time/)).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Turn Google Takeout on" })).toBeDisabled();
  });

  it("needs_newer_lumen_row", async () => {
    mount([info({ state: { type: "needsNewerLumen" } })]);
    expect(await screen.findByText("Needs a newer Lumen")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Turn Google Takeout on" })).toBeDisabled();
  });

  // Round 24 lane C
  it("auto_update_switch_and_update_all", async () => {
    const status = { offline: false, hasCache: true, checkedAt: null, autoUpdate: true, updates: [{ id: "a.b", name: "Alpha", from: "1.0.0", to: "1.1.0", needsReview: false }], updated: [{ id: "c.d", name: "Beta", version: "2.0.0" }] };
    mockCommand("store_status", () => ({ ...status }));
    mockCommand("store_set_auto_update", (a) => {
      status.autoUpdate = (a as { on: boolean }).on;
    });
    mockCommand("store_update", () => ({ path: null, installed: true }));
    mount([info()]);
    expect(await screen.findByText("Updated Beta to 2.0.0")).toBeInTheDocument();
    expect(screen.queryByText("Update all")).toBeNull();
    await userEvent.click(screen.getByRole("switch", { name: "Update plugins automatically" }));
    await waitFor(() => expect(calls("store_set_auto_update")[0][1]).toEqual({ on: false }));
    await userEvent.click(await screen.findByRole("button", { name: "Update all" }));
    await waitFor(() => expect(calls("store_update")[0][1]).toEqual({ id: "a.b" }));
  });
});

describe("Settings search over plugin controls", () => {
  const sampler = info({
    id: "local.sampler",
    name: "Sampler",
    sections: [
      {
        title: "Scanning",
        description: null,
        controls: [
          { kind: "slider", key: "depth", label: "Scan depth", help: "How many folders deep.", value: 3, choices: [], min: 1, max: 8, step: 1, requires: null, app: null, action: null, body: null },
          { kind: "toggle", key: "deep", label: "Deep scan", help: null, value: false, choices: [], min: null, max: null, step: null, requires: null, app: null, action: null, body: null },
        ],
      },
    ],
  });
  const Where = () => <output data-testid="where">{useLocation().search}</output>;
  const search = (q: string) => {
    mockCommand("list_plugins", () => [sampler]);
    mockCommand("get_plugin_developer_mode", () => false);
    return render(
      <Providers>
        <MemoryRouter initialEntries={["/settings?section=plugins"]}>
          <SearchContext.Provider value={q}>
            <PluginsSection />
          </SearchContext.Provider>
          <Where />
        </MemoryRouter>
      </Providers>,
    );
  };

  it("finds a control by its label and jumps to the plugin's page", async () => {
    search("depth");
    expect(await screen.findByRole("button", { name: "Open Sampler" })).toBeInTheDocument();
    expect(document.querySelector(".settings-row__label")?.textContent).toBe("Scan depth");
    expect(screen.getAllByRole("button", { name: "Open Sampler" })).toHaveLength(1);
    await userEvent.click(screen.getByRole("button", { name: "Open Sampler" }));
    expect(screen.getByTestId("where").textContent).toBe("?section=plugins&plugin=local.sampler");
  });

  it("finds a control by its help text", async () => {
    search("folders deep");
    expect(await screen.findByRole("button", { name: "Open Sampler" })).toBeInTheDocument();
    expect(document.querySelector(".settings-row__label")?.textContent).toBe("Scan depth");
  });
});
