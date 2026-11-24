import { describe, expect, it, vi } from "vitest";
import { labels } from "./index";
import { pluginMenu, storePluginMenu, storeThemeMenu } from "./plugins";

const a = () => ({ details: vi.fn(), settings: vi.fn(), log: vi.fn(), reload: vi.fn(), remove: vi.fn() });

describe("pluginMenu", () => {
  it("menu_items_by_mode: Log only in Developer mode, Reload only for dev plugins", () => {
    expect(labels(pluginMenu({ developerMode: false, dev: false, hasSettings: true }, a()))).toEqual(["Details", "Settings", "—", "Remove"]);
    expect(labels(pluginMenu({ developerMode: true, dev: false, hasSettings: true }, a()))).toEqual(["Details", "Settings", "Log", "—", "Remove"]);
    expect(labels(pluginMenu({ developerMode: true, dev: true, hasSettings: false }, a()))).toEqual(["Details", "Log", "Reload", "—", "Remove"]);
  });
  it("calls the action", () => {
    const acts = a();
    const items = pluginMenu({ developerMode: false, dev: false, hasSettings: false }, acts);
    const last = items[items.length - 1];
    if ("onSelect" in last) last.onSelect();
    expect(acts.remove).toHaveBeenCalled();
  });
});

describe("store menus", () => {
  const sp = () => ({ primary: vi.fn(), details: vi.fn(), settings: vi.fn(), remove: vi.fn() });
  const st = () => ({ primary: vi.fn(), tryIt: vi.fn(), details: vi.fn(), remove: vi.fn() });
  it("plugin: Remove only once installed", () => {
    expect(labels(storePluginMenu({ action: "Get", installed: false, hasSettings: false }, sp()))).toEqual(["Get", "Details"]);
    expect(labels(storePluginMenu({ action: "Open", installed: true, hasSettings: true }, sp()))).toEqual(["Details", "Settings", "—", "Remove"]);
    expect(labels(storePluginMenu({ action: "Update", installed: true, hasSettings: false }, sp()))).toEqual(["Update", "Details", "—", "Remove"]);
  });
  it("theme: Try only before it is installed, Remove only after", () => {
    expect(labels(storeThemeMenu({ action: "Get", installed: false }, st()))).toEqual(["Get", "Try", "Details"]);
    expect(labels(storeThemeMenu({ action: "Use", installed: true }, st()))).toEqual(["Use", "Details", "—", "Remove"]);
    expect(labels(storeThemeMenu({ action: "In use", installed: true }, st()))).toEqual(["Details", "—", "Remove"]);
  });
  it("remove calls the action", () => {
    const acts = sp();
    const items = storePluginMenu({ action: "Open", installed: true, hasSettings: false }, acts);
    const last = items[items.length - 1];
    if ("onSelect" in last) last.onSelect();
    expect(acts.remove).toHaveBeenCalled();
  });
});
