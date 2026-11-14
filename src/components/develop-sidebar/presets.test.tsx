import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { PresetNotesHost } from "@/components/preset-import";
import { ContextMenuHost } from "@/components/context-menu";
import { Toaster } from "@/components/toast";
import { IDENTITY } from "@/hooks/use-edit-session";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { developOf } from "@/lib/develop";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { Presets } from "./presets";

const session = () =>
  ({ draft: IDENTITY, develop: developOf(IDENTITY), setHover: vi.fn(), apply: vi.fn(), goTo: vi.fn() }) as unknown as DevelopSession;

function mount() {
  render(
    <Providers>
      <Presets session={session()} />
      <ContextMenuHost />
      <Toaster />
      <PresetNotesHost />
    </Providers>,
  );
}

describe("Lightroom presets", () => {
  beforeEach(() => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_presets", () => []);
    mockCommand("camera_raw_preset_folder", () => "C:/Users/x/AppData/Roaming/Adobe/CameraRaw/Settings");
    mockCommand("import_lightroom_presets", () => ({
      imported: 34,
      skippedSameName: 0,
      failed: [],
      notConverted: [
        { name: "Portrait / Soft", settings: ["LensProfileEnable", "Mask: Sky"] },
        { name: "Matte", settings: ["CameraProfile"] },
      ],
    }));
  });

  it("imports from Lightroom's own folder and says what it could not convert", async () => {
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "Preset actions" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Import from Lightroom" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("import_lightroom_presets", { paths: ["C:/Users/x/AppData/Roaming/Adobe/CameraRaw/Settings"] }));
    expect(await screen.findByText("Imported 34 Lightroom presets · 3 settings not converted")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Details" }));
    const dialog = await screen.findByRole("dialog", { name: "Settings not converted" });
    expect(within(dialog).getByText("Portrait / Soft")).toBeInTheDocument();
    expect(within(dialog).getByText("LensProfileEnable, Mask: Sky")).toBeInTheDocument();
    expect(within(dialog).getByText("Matte")).toBeInTheDocument();
  });

  it("says when nothing was new", async () => {
    mockCommand("import_lightroom_presets", () => ({ imported: 0, skippedSameName: 3, failed: [], notConverted: [] }));
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "Preset actions" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Import from Lightroom" }));
    expect(await screen.findByText("No new Lightroom presets · 3 already here")).toBeInTheDocument();
  });
});

const dev = { ...developOf(IDENTITY) };
const builtinPreset = { id: -1, key: "builtin:1", name: "Natural", develop: dev, panels: ["basic"], builtin: true, source: "builtin" };
const myPreset = { id: 5, key: "db:5", name: "Moody", develop: dev, panels: ["effects"], builtin: false, source: "mine" };
const fromPlugin = (plugin: string, group: string, file: string, name: string) => ({
  id: 0,
  key: `plugin:${plugin}/${file}`,
  name,
  develop: dev,
  panels: ["basic"],
  builtin: false,
  source: { plugin: { plugin, name: group } },
});

describe("Plugin presets", () => {
  beforeEach(() => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("camera_raw_preset_folder", () => null);
    mockCommand("list_presets", () => [
      builtinPreset,
      myPreset,
      fromPlugin("com.darkroom.tools", "Darkroom Tools", "warm.json", "Warm"),
      fromPlugin("com.other.pack", "Other Pack", "warm.json", "Warm"),
    ]);
  });

  it("plugin_group_read_only: a group titled with the plugin name, no rename or delete", async () => {
    mount();
    expect(await screen.findByText("Darkroom Tools")).toBeInTheDocument();
    expect(screen.getByRole("list", { name: "Darkroom Tools" })).toBeInTheDocument();
    // Two plugins ship a preset of the same name: both are listed, each in its own group.
    expect(within(screen.getByRole("list", { name: "Darkroom Tools" })).getByRole("button", { name: "Warm" })).toBeInTheDocument();
    expect(within(screen.getByRole("list", { name: "Other Pack" })).getByRole("button", { name: "Warm" })).toBeInTheDocument();

    await userEvent.pointer({ keys: "[MouseRight]", target: within(screen.getByRole("list", { name: "Darkroom Tools" })).getByRole("button", { name: "Warm" }) });
    const items = (await screen.findAllByRole("menuitem")).map((i) => i.textContent);
    expect(items).toEqual(["Duplicate to my presets"]);
  });

  it("keeps rename and delete on the person's own presets", async () => {
    mount();
    await userEvent.pointer({ keys: "[MouseRight]", target: await screen.findByRole("button", { name: "Moody" }) });
    expect((await screen.findAllByRole("menuitem")).map((i) => i.textContent)).toEqual(["Rename", "Delete"]);
  });

  it("duplicate_to_my_presets: the menu item duplicates by key and the list refreshes", async () => {
    mockCommand("duplicate_plugin_preset", (a) => ({ ...myPreset, id: 11, key: "db:11", name: String(a?.key) }));
    mount();
    await userEvent.pointer({ keys: "[MouseRight]", target: within(await screen.findByRole("list", { name: "Other Pack" })).getByRole("button", { name: "Warm" }) });
    await userEvent.click(await screen.findByRole("menuitem", { name: "Duplicate to my presets" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("duplicate_plugin_preset", { key: "plugin:com.other.pack/warm.json" }));
    await waitFor(() => expect(invokeMock.mock.calls.filter((c) => c[0] === "list_presets").length).toBeGreaterThan(1));
  });
});
