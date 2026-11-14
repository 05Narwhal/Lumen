import { beforeEach, describe, expect, it } from "vitest";
import { applyPluginThemes, resolveTheme } from "@/lib/plugin-theme";
import type { PluginThemeMeta } from "@/types/generated/PluginThemeMeta";
import { mockCommand } from "@/test/tauri-mock";

const NORD: PluginThemeMeta = {
  plugin: "a.b",
  id: "nord",
  name: "Nord",
  tagline: "Arctic blues.",
  base: "darkroom",
  defaultScheme: "dark",
};

describe("applyPluginThemes", () => {
  beforeEach(() => document.getElementById("plugin-themes")?.remove());

  it("fills_one_style_tag", async () => {
    mockCommand("plugin_theme_css", () => "[data-plugin-theme] { --accent: red; }");
    await applyPluginThemes();
    await applyPluginThemes();
    const tags = document.querySelectorAll("style#plugin-themes");
    expect(tags).toHaveLength(1);
    expect(tags[0].textContent).toBe("[data-plugin-theme] { --accent: red; }");
    expect(tags[0].parentElement).toBe(document.head);
  });

  it("follows the CSS when it changes and empties when the last theme goes", async () => {
    mockCommand("plugin_theme_css", () => "a{}");
    await applyPluginThemes();
    mockCommand("plugin_theme_css", () => "");
    await applyPluginThemes();
    expect(document.getElementById("plugin-themes")?.textContent).toBe("");
  });

  it("keeps what is there when the command fails, and never throws", async () => {
    mockCommand("plugin_theme_css", () => "a{}");
    await applyPluginThemes();
    mockCommand("plugin_theme_css", () => {
      throw { code: "unknown", message: "no" };
    });
    await expect(applyPluginThemes()).resolves.toBeUndefined();
    expect(document.getElementById("plugin-themes")?.textContent).toBe("a{}");
  });
});

describe("resolveTheme", () => {
  it("a built-in theme is its own base", () => {
    expect(resolveTheme("paper", [])).toEqual({ base: "paper", plugin: null });
  });

  it("a plugin theme sits on its own base", () => {
    expect(resolveTheme({ plugin: "a.b", theme: "nord" }, [NORD])).toEqual({ base: "darkroom", plugin: "a.b/nord" });
  });

  it("a plugin theme that is off or gone is the default theme, with no plugin attribute", () => {
    expect(resolveTheme({ plugin: "a.b", theme: "nord" }, [])).toEqual({ base: "gallery", plugin: null });
    expect(resolveTheme({ plugin: "a.b", theme: "other" }, [NORD])).toEqual({ base: "gallery", plugin: null });
  });
});
