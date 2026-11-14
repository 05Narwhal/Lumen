import { describe, expect, it, vi } from "vitest";
import { labels } from "./index";
import { pluginPresetMenu, presetsMenu } from "./presets";

describe("presetsMenu", () => {
  it("offers Lightroom's own folder only when it exists", () => {
    const a = { save: vi.fn(), importFiles: vi.fn(), importFolder: vi.fn(), importFromLightroom: vi.fn() };
    expect(labels(presetsMenu({ lightroomFolder: null }, a))).toEqual(["Save preset…", "—", "Import Lightroom presets…", "Import a folder of Lightroom presets…"]);
    const items = presetsMenu({ lightroomFolder: "C:/Users/x/AppData/Roaming/Adobe/CameraRaw/Settings" }, a);
    expect(labels(items)).toEqual(["Save preset…", "—", "Import Lightroom presets…", "Import a folder of Lightroom presets…", "Import from Lightroom"]);
    const last = items[items.length - 1];
    if ("onSelect" in last) last.onSelect();
    expect(a.importFromLightroom).toHaveBeenCalled();
  });
});

describe("pluginPresetMenu", () => {
  it("only duplicates: a plugin's preset is read-only", () => {
    const duplicate = vi.fn();
    const items = pluginPresetMenu({ duplicate });
    expect(labels(items)).toEqual(["Duplicate to my presets"]);
    const first = items[0];
    if ("onSelect" in first) first.onSelect();
    expect(duplicate).toHaveBeenCalled();
  });
});
