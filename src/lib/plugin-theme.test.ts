import { beforeEach, describe, expect, it } from "vitest";
import { applyPluginThemes, resolveTheme } from "@/lib/plugin-theme";
import type { InstalledTheme } from "@/types/generated/InstalledTheme";
import { mockCommand } from "@/test/tauri-mock";

const NORD: InstalledTheme = {
  id: "a.b",
  name: "Nord",
  author: "Nobody",
  version: "1.0.0",
  description: "Arctic blues.",
  license: "MIT",
  base: "darkroom",
  defaultScheme: "dark",
  layout: false,
  hasIcon: false,
  tokens: {
    light: { bg: "#fff", surface: "#fff", sidebar: "#fff", fg: "#000", fg2: "#555", accent: "#00f", placeholder: "#eee", radius: "8px", gap: "4px" },
    dark: { bg: "#000", surface: "#111", sidebar: "#111", fg: "#fff", fg2: "#aaa", accent: "#0af", placeholder: "#222", radius: "8px", gap: "4px" },
  },
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

  it("an installed theme sits on its own base", () => {
    expect(resolveTheme({ plugin: "a.b", theme: "theme" }, [NORD])).toEqual({ base: "darkroom", plugin: "a.b/theme" });
  });

  it("an installed theme that is gone is the default theme, with no plugin attribute", () => {
    expect(resolveTheme({ plugin: "a.b", theme: "theme" }, [])).toEqual({ base: "gallery", plugin: null });
    expect(resolveTheme({ plugin: "c.d", theme: "theme" }, [NORD])).toEqual({ base: "gallery", plugin: null });
  });
});
