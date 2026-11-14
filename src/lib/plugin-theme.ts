import { useQuery } from "@tanstack/react-query";
import { invoke } from "@/lib/ipc";
import type { PluginThemeMeta } from "@/types/generated/PluginThemeMeta";
import type { ThemeChoice } from "@/types/generated/ThemeChoice";
import type { ThemeId } from "@/types/generated/ThemeId";

export const PLUGIN_THEMES_KEY = ["plugin-themes"] as const;

/** The theme a missing or disabled plugin theme falls back to. */
const DEFAULT_THEME: ThemeId = "gallery";

/**
 * Fills `<style id="plugin-themes">` with the CSS Rust wrote for the enabled plugin themes
 * (creating the tag the first time). Called at start and on `plugins://changed`. A failure keeps
 * whatever is there: themes are never worth an error on the page.
 */
export async function applyPluginThemes(): Promise<void> {
  try {
    const css = await invoke<string>("plugin_theme_css");
    let el = document.getElementById("plugin-themes");
    if (!el) {
      el = document.createElement("style");
      el.id = "plugin-themes";
      document.head.appendChild(el);
    }
    if (el.textContent !== css) el.textContent = css;
  } catch {
    // Keep what is there.
  }
}

/**
 * The built-in theme the page is drawn on and the `data-plugin-theme` value to set (`null`
 * for none). A plugin theme whose plugin is off or gone is the default theme, unadorned.
 */
export function resolveTheme(choice: ThemeChoice, plugins: PluginThemeMeta[]): { base: ThemeId; plugin: string | null } {
  if (typeof choice === "string") return { base: choice, plugin: null };
  const meta = plugins.find((m) => m.plugin === choice.plugin && m.id === choice.theme);
  return meta ? { base: meta.base, plugin: `${meta.plugin}/${meta.id}` } : { base: DEFAULT_THEME, plugin: null };
}

/**
 * The enabled plugin themes. `undefined` while the first answer is on its way; an empty list
 * when the command fails (a theme that can't be listed is a theme that isn't there).
 */
export function usePluginThemes(): PluginThemeMeta[] | undefined {
  const q = useQuery({
    queryKey: PLUGIN_THEMES_KEY,
    queryFn: () => invoke<PluginThemeMeta[]>("list_plugin_themes"),
    staleTime: Infinity,
    retry: false,
  });
  return q.isError ? [] : q.data;
}
