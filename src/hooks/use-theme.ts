import { useMantineColorScheme } from "@mantine/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { invoke } from "@/lib/ipc";
import { oklchToRgb } from "@/lib/oklch";
import { applyPluginThemes, PLUGIN_THEMES_KEY, resolveTheme, useInstalledThemes } from "@/lib/plugin-theme";
import { queryKeys } from "@/lib/query-keys";
import type { Settings } from "@/types/generated/Settings";
import { useSettings } from "./use-settings";

/**
 * Mirrors appearance settings onto the document root; the only place that does.
 *
 * Two details here are load-bearing, and between them they were why every settings-backed
 * control in the app appeared to have no animation:
 *
 * `keepTransitions` stops Mantine injecting `*, *::before, *::after { transition: none
 * !important }` into the document head for 10ms on each colour-scheme write. That
 * stylesheet cancels every transition running anywhere in the app, and the app writes the
 * scheme far more often than the scheme actually changes.
 *
 * The scheme is then only written when it really differs. `setColorScheme` is not
 * memoised by Mantine, so it is a new function on every render; keeping it in a dependency
 * array re-ran this effect on every settings change, which is exactly when a control has
 * an animation in flight. Measured in the real webview: the thumb reported
 * `transitionrun:transform` on frame 0 and `transitioncancel:transform` on frame 2.
 *
 * An installed theme (a `.lumentheme`, Round 26) keeps `data-theme` on its built-in base and adds
 * `data-plugin-theme="<id>/theme"`; the CSS for it is one `<style id="plugin-themes">`, filled
 * at start and whenever `plugins://changed`. One that is gone shows the default theme. Rust
 * changes the theme setting itself (`theme_apply`): `app://settings-changed` refreshes it here.
 */
export function useApplyTheme() {
  const { settings } = useSettings();
  const { setColorScheme } = useMantineColorScheme({ keepTransitions: true });
  const queryClient = useQueryClient();
  const plugins = useInstalledThemes();
  // Bumped each time the plugin CSS has been (re)written, so the boot colours are read from it.
  const [cssVersion, setCssVersion] = useState(0);
  const choice = settings?.appearance?.theme;
  // A plugin choice waits for the list; a built-in one never does.
  const resolved = choice === undefined || (typeof choice !== "string" && plugins === undefined) ? null : resolveTheme(choice, plugins ?? []);
  const base = resolved?.base;
  const pluginTheme = resolved?.plugin ?? null;
  const scheme = settings?.appearance?.scheme;
  const reduceMotion = settings?.appearance?.reduceMotion;
  const appliedScheme = useRef<string | null>(null);
  const apply = useRef(setColorScheme);
  apply.current = setColorScheme;

  useEffect(() => {
    let live = true;
    const refresh = () => void applyPluginThemes().then(() => live && setCssVersion((v) => v + 1));
    refresh();
    const changed = listen("plugins://changed", () => {
      void queryClient.invalidateQueries({ queryKey: PLUGIN_THEMES_KEY });
      refresh();
    });
    const settingsChanged = listen<Settings>("app://settings-changed", (e) => {
      if (e.payload) queryClient.setQueryData(queryKeys.settings, e.payload);
      else void queryClient.invalidateQueries({ queryKey: queryKeys.settings });
    });
    return () => {
      live = false;
      void changed.then((f) => f()).catch(() => {});
      void settingsChanged.then((f) => f()).catch(() => {});
    };
  }, [queryClient]);

  useEffect(() => {
    if (!base || !scheme) return;
    const root = document.documentElement;
    root.dataset.theme = base;
    // Read by index.html's inline script on the next launch (Round 24 §6.3).
    try {
      localStorage.setItem(THEME_KEY, base);
    } catch {
      // Storage may be unavailable; the next launch starts on the default theme until settings load.
    }
    if (pluginTheme) root.dataset.pluginTheme = pluginTheme;
    else delete root.dataset.pluginTheme;
    root.dataset.reducedMotion = String(reduceMotion ?? false);
    if (appliedScheme.current !== scheme) {
      appliedScheme.current = scheme;
      apply.current(scheme);
    }
    // The next launch paints its boot screen (index.html) in these colours before any script
    // or stylesheet has loaded, instead of a white window.
    const frame = requestAnimationFrame(() => rememberBootColours(root));
    return () => cancelAnimationFrame(frame);
  }, [base, pluginTheme, scheme, reduceMotion, cssVersion]);
}

/**
 * The built-in theme in use, read by index.html before any script runs so the splash (and a
 * start that sticks before settings load) is already in the person's theme, not the default
 * (Round 24 §6.3). The key is shared with that script.
 */
export const THEME_KEY = "lumen.theme";

/** Read by the inline script in index.html; the key is shared with it. */
export const BOOT_COLOURS_KEY = "lumen.boot-colours";

function rememberBootColours(root: HTMLElement) {
  const css = getComputedStyle(root);
  const bg = css.getPropertyValue("--bg-base").trim();
  const fg = css.getPropertyValue("--fg-primary").trim();
  const accent = css.getPropertyValue("--accent").trim();
  if (bg) paintNativeWindow(bg);
  if (!bg || !fg) return;
  try {
    localStorage.setItem(BOOT_COLOURS_KEY, JSON.stringify({ bg, fg, accent, scheme: root.dataset.mantineColorScheme }));
  } catch {
    // Storage may be unavailable; the boot screen falls back to the OS light/dark colours.
  }
}

/**
 * The native window's own background follows the theme. A maximized borderless window keeps
 * 1 px free at the bottom for an auto-hiding taskbar (tao), and that strip shows this colour
 * rather than the page: the fixed dark one Rust paints at launch read as a black line.
 */
function paintNativeWindow(bg: string) {
  const rgb = oklchToRgb(bg);
  if (!rgb) return;
  try {
    void getCurrentWindow()
      .setBackgroundColor([...rgb, 255])
      .catch(() => {});
    // The strip itself is non-client area: Rust paints it (`window_edge.rs`).
    void invoke("paint_window_edge", { r: rgb[0], g: rgb[1], b: rgb[2] }).catch(() => {});
  } catch {
    // Not running inside Tauri (browser shim): there is no native window to paint.
  }
}
