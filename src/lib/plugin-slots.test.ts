import { describe, expect, it, vi } from "vitest";
import {
  contributionsFor,
  liveContributions,
  paletteContributions,
  runContribution,
  scopeFor,
  scopeOfSlot,
  slotIcon,
  SLOT_ICONS,
} from "./plugin-slots";
import type { PluginContribution } from "@/types/generated/PluginContribution";
import type { PluginInfo } from "@/types/generated/PluginInfo";

const plugin = (id: string, contributions: PluginContribution[], over: Partial<PluginInfo> = {}): PluginInfo =>
  ({
    id,
    name: `Plugin ${id}`,
    enabled: true,
    state: { type: "ready" },
    contributions,
    ...over,
  }) as PluginInfo;

const c = (slot: PluginContribution["slot"], label: string, action = "go", icon: string | null = null): PluginContribution => ({
  slot,
  label,
  icon,
  action,
});

describe("plugin slots", () => {
  it("lists_what_every_ready_plugin_adds_to_a_slot, naming no plugin in code", () => {
    const plugins = [
      plugin("a", [c("map.toolbar", "Find"), c("viewer.menu", "Look")]),
      plugin("b", [c("map.toolbar", "Other", "x", "search")]),
    ];
    const toolbar = contributionsFor(plugins, "map.toolbar");
    expect(toolbar.map((t) => [t.pluginId, t.label])).toEqual([
      ["a", "Find"],
      ["b", "Other"],
    ]);
    expect(contributionsFor(plugins, "viewer.menu").map((t) => t.label)).toEqual(["Look"]);
    expect(contributionsFor(plugins, "library.selection-menu")).toEqual([]);
  });

  it("a_plugin_that_is_off_stopped_broken_or_too_new_contributes_nothing", () => {
    const entries = [c("map.toolbar", "Find")];
    const plugins = [
      plugin("off", entries, { enabled: false, state: { type: "off" } }),
      plugin("stopped", entries, { state: { type: "stopped", reason: "3 traps" } }),
      plugin("problem", entries, { state: { type: "problem", reason: "x" } }),
      plugin("new", entries, { state: { type: "needsNewerLumen" } }),
      plugin("flag", entries, { enabled: false }),
      plugin("ok", entries),
    ];
    expect(liveContributions(plugins).map((t) => t.pluginId)).toEqual(["ok"]);
    expect(liveContributions([])).toEqual([]);
  });

  it("every_contribution_also_has_a_palette_entry, once", () => {
    const plugins = [
      plugin("a", [c("map.toolbar", "Find"), c("palette", "Scan"), c("viewer.menu", "Look")]),
      plugin("b", [c("library.selection-menu", "Check"), c("viewer.menu", "Check")]),
    ];
    const rows = paletteContributions(plugins);
    // Same plugin + label in two slots is one palette entry; the same label from another plugin is not.
    expect(rows.map((r) => [r.pluginId, r.label])).toEqual([
      ["a", "Find"],
      ["a", "Scan"],
      ["a", "Look"],
      ["b", "Check"],
    ]);
    expect(rows.map((r) => r.id)).toEqual(["plugin:a:map.toolbar:Find", "plugin:a:palette:Scan", "plugin:a:viewer.menu:Look", "plugin:b:library.selection-menu:Check"]);
  });

  it("palette_entries_of_a_removed_plugin_are_gone", () => {
    const before = [plugin("a", [c("palette", "Scan")])];
    expect(paletteContributions(before)).toHaveLength(1);
    expect(paletteContributions([])).toHaveLength(0);
  });

  it("an_icon_is_a_known_tabler_name_or_none", () => {
    expect(slotIcon("map-pin")).toBeTruthy();
    expect(slotIcon(null)).toBeNull();
    expect(slotIcon("skull")).toBeNull();
    expect(SLOT_ICONS).toContain("map-pin");
  });

  it("open_review_navigates_to_the_review_page_of_that_plugin", async () => {
    const navigate = vi.fn();
    const [t] = liveContributions([plugin("jl.x", [c("palette", "Find", "open-review:places")])]);
    await runContribution(t, { navigate, runAction: vi.fn() });
    expect(navigate).toHaveBeenCalledWith("/review/places?plugin=jl.x");
  });

  it("any_other_action_goes_to_the_plugin_by_name", async () => {
    const runAction = vi.fn().mockResolvedValue("done");
    const [t] = liveContributions([plugin("p", [c("viewer.menu", "Look", "look")])]);
    await runContribution(t, { navigate: vi.fn(), runAction });
    expect(runAction).toHaveBeenCalledWith("p", "look");
  });

  it("a_tool_action_runs_that_tool_on_the_slots_scope", async () => {
    const runTool = vi.fn().mockResolvedValue(undefined);
    const plugins = [plugin("p", [c("map.toolbar", "Find", "tool:find"), c("library.selection-menu", "Find", "tool:find")])];
    const [bar, menu] = liveContributions(plugins);
    await runContribution(bar, { navigate: vi.fn(), runTool });
    expect(runTool).toHaveBeenLastCalledWith({ id: "p", name: "Plugin p" }, "find", { scope: "library" }, expect.any(Function));
    await runContribution(menu, { navigate: vi.fn(), runTool });
    expect(runTool).toHaveBeenLastCalledWith({ id: "p", name: "Plugin p" }, "find", { scope: "selection" }, expect.any(Function));
    // a page that knows better (an album) passes its own scope
    await runContribution(bar, { navigate: vi.fn(), runTool, scope: { scope: "album", album: 3 } });
    expect(runTool).toHaveBeenLastCalledWith({ id: "p", name: "Plugin p" }, "find", { scope: "album", album: 3 }, expect.any(Function));
    expect(scopeOfSlot("viewer.menu")).toEqual({ scope: "selection" });
    expect(scopeOfSlot("palette")).toEqual({ scope: "library" });
  });

  it("a_selection_slot_carries_the_ids_it_was_opened_on, other slots carry none", () => {
    expect(scopeFor("library.selection-menu", [3, 4])).toEqual({ scope: "selection", assets: [3, 4] });
    expect(scopeFor("viewer.menu", [9])).toEqual({ scope: "selection", assets: [9] });
    // Nothing selected, or a slot that is not about a selection: the slot's own scope, no ids.
    expect(scopeFor("viewer.menu", [])).toEqual({ scope: "selection" });
    expect(scopeFor("library.selection-menu", undefined)).toEqual({ scope: "selection" });
    expect(scopeFor("map.toolbar", [1])).toEqual({ scope: "library" });
  });
});
