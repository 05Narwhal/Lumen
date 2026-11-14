export type { ThemeChoice } from "@/types/generated/ThemeChoice";
import type { ThemeChoice } from "@/types/generated/ThemeChoice";
import type { PluginThemeMeta } from "@/types/generated/PluginThemeMeta";

export type ThemeId = "gallery" | "darkroom" | "paper" | "glass";
export type SchemeChoice = "light" | "dark" | "auto";

export interface ThemeMeta {
  id: ThemeId;
  name: string;
  tagline: string;
  defaultScheme: "light" | "dark";
}

export const THEMES: ThemeMeta[] = [
  { id: "gallery", name: "Gallery", tagline: "Pure monochrome. The photos are the only colour.", defaultScheme: "light" },
  { id: "darkroom", name: "Darkroom", tagline: "Dark surfaces, floating panels, one warm accent.", defaultScheme: "dark" },
  { id: "paper", name: "Paper", tagline: "Warm, editorial, generous whitespace.", defaultScheme: "light" },
  { id: "glass", name: "Glass", tagline: "Frosted layers over a soft colour wash.", defaultScheme: "light" },
];

/** What a theme choice is called: a built-in's name, or the plugin theme's own. */
export function themeName(choice: ThemeChoice, plugins: PluginThemeMeta[] = []): string {
  if (typeof choice === "string") return THEMES.find((t) => t.id === choice)?.name ?? choice;
  return plugins.find((m) => m.plugin === choice.plugin && m.id === choice.theme)?.name ?? choice.theme;
}
