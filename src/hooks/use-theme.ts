import { useMantineColorScheme } from "@mantine/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { invoke } from "@/lib/ipc";
import { oklchToRgb } from "@/lib/oklch";
import { applyPluginThemes, PLUGIN_THEMES_KEY, resolveTheme, usePluginThemes } from "@/lib/plugin-theme";
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
 * A plugin theme (Round 23) keeps `data-theme` on its built-in base and adds
 * `data-plugin-theme="<plugin>/<id>"`; the CSS for it is one `<style id="plugin-themes">`, filled
 * at start and whenever `plugins://changed`. One whose plugin is off or gone shows the default
 * theme.
 */
export function useApplyTheme() {
  const { settings } = useSettings();
  const { setColorScheme } = useMantineColorScheme({ keepTransitions: true });
  const queryClient = useQueryClient();
  const plugins = usePluginThemes();
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
    const unlisten = listen("plugins://changed", () => {
      void queryClient.invalidateQueries({ queryKey: PLUGIN_THEMES_KEY });
      refresh();
    });
    return () => {
      live = false;
      void unlisten.then((f) => f()).catch(() => {});
    };
  }, [queryClient]);

  useEffect(() => {
    if (!base || !scheme) return;
    const root = document.documentElement;
    root.dataset.theme = base;
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
