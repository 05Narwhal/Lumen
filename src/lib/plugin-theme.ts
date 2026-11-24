import { useQuery } from "@tanstack/react-query";
import { invoke } from "@/lib/ipc";
import type { InstalledTheme } from "@/types/generated/InstalledTheme";
import type { ThemeChoice } from "@/types/generated/ThemeChoice";
import type { ThemeId } from "@/types/generated/ThemeId";

export const PLUGIN_THEMES_KEY = ["installed-themes"] as const;

/** The one theme a `.lumentheme` carries: `appearance.theme` is `{plugin: <id>, theme: "theme"}`. */
export const THEME_SLOT = "theme";

/** The theme a missing or disabled plugin theme falls back to. */
const DEFAULT_THEME: ThemeId = "gallery";

/**
 * Fills `<style id="plugin-themes">` with the CSS Rust wrote for the installed themes
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
 * for none). An installed theme sits on its own base; one that is gone is the default theme,
 * unadorned.
 */
export function resolveTheme(choice: ThemeChoice, themes: InstalledTheme[]): { base: ThemeId; plugin: string | null } {
  if (typeof choice === "string") return { base: choice, plugin: null };
  const meta = choice.theme === THEME_SLOT ? themes.find((m) => m.id === choice.plugin) : undefined;
  return meta ? { base: meta.base, plugin: `${meta.id}/${THEME_SLOT}` } : { base: DEFAULT_THEME, plugin: null };
}

/**
 * The installed themes, and the one being tried (it is not installed, but the page draws it).
 * `undefined` while the first answer is on its way; an empty list when the command fails (a
 * theme that can't be listed is a theme that isn't there).
 */
export function useInstalledThemes(): InstalledTheme[] | undefined {
  const q = useQuery({
    queryKey: PLUGIN_THEMES_KEY,
    queryFn: async () => {
      const [list, tried] = await Promise.all([invoke<InstalledTheme[]>("themes_list"), invoke<InstalledTheme | null>("theme_tried").catch(() => null)]);
      const all = Array.isArray(list) ? list : [];
      return tried ? [...all.filter((t) => t.id !== tried.id), tried] : all;
    },
    staleTime: Infinity,
    retry: false,
  });
  return q.isError ? [] : q.data;
}
