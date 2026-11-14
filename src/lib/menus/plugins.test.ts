import { describe, expect, it, vi } from "vitest";
import { labels } from "./index";
import { pluginMenu } from "./plugins";

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
