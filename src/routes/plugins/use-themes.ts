import { useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router";
import { toast } from "@/components/toast";
import { storeThemeMenu } from "@/lib/menus/plugins";
import type { MenuItem } from "@/lib/menus/parts";
import { useSettings } from "@/hooks/use-settings";
import { isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import { themeApply, themeInstall, themeKeep, themeRemove, themesList, themeTry, themeTryEnd } from "@/lib/theme-store";
import { createThemeTry, type TryState } from "@/lib/theme-try";
import { REMOVE_UNDO_MS } from "@/lib/theme-manage";
import type { InstalledTheme } from "@/types/generated/InstalledTheme";

/** A theme tile's button: *Get* installs, *Keep* installs the theme being tried, *Use* makes an installed theme the app's, *In use* is the current one. */
export type ThemeAction = { label: "Get" | "Keep" | "Use" | "In use"; disabled: boolean };

const message = (e: unknown, fallback: string) => (isAppError(e) ? e.message : fallback);

/** The installed themes, refetched on `plugins://changed` (themes are announced the same way). */
function useInstalledThemes() {
  const [themes, setThemes] = useState<InstalledTheme[]>([]);
  const reload = useCallback(() => {
    themesList()
      .then((t) => setThemes(Array.isArray(t) ? t : []))
      .catch(() => {});
  }, []);
  useEffect(() => {
    reload();
    const un = listen("plugins://changed", reload);
    return () => void un.then((f) => f()).catch(() => {});
  }, [reload]);
  return { themes, reload };
}

/**
 * Try for the store pages: shows a theme for 30 s or until the person leaves, then puts the
 * theme in use back. The settings cache is refreshed by hand after `theme_apply`, because Rust
 * changes the setting behind the page's back.
 */
function useTry() {
  const qc = useQueryClient();
  const { settings, update } = useSettings();
  const live = useRef({ settings, update, qc });
  live.current = { settings, update, qc };
  const [state, setState] = useState<TryState | null>(null);
  const ctl = useRef<ReturnType<typeof createThemeTry> | null>(null);
  if (!ctl.current) {
    ctl.current = createThemeTry(
      {
        tryTheme: themeTry,
        apply: async (id) => {
          await themeApply(id);
          await live.current.qc.invalidateQueries({ queryKey: queryKeys.settings });
        },
        end: themeTryEnd,
        keep: themeKeep,
        current: () => live.current.settings?.appearance.theme,
        restore: (back) => live.current.update((s) => ({ ...s, appearance: { ...s.appearance, theme: back } })),
      },
      setState,
    );
  }
  // Leaving the page puts the previous theme back.
  useEffect(() => () => void ctl.current?.end().catch(() => {}), []);
  return { state, ctl: ctl.current };
}

/** What Get / Use / Try mean for each store theme, and running them (shared by the tab and the page). */
export function useThemeActions() {
  const { themes: listed, reload } = useInstalledThemes();
  const { settings, update } = useSettings();
  const navigate = useNavigate();
  const [busyId, setBusyId] = useState<string | null>(null);
  // *Remove* hides the theme at once and deletes it when the undo toast runs out.
  const [hidden, setHidden] = useState<ReadonlySet<string>>(new Set());
  const themes = listed.filter((t) => !hidden.has(t.id));
  const { state: trying, ctl } = useTry();
  const choice = settings?.appearance.theme;
  const currentId = choice && typeof choice === "object" ? choice.plugin : null;

  const actionOf = (id: string): ThemeAction => {
    if (id === trying?.storeId) return { label: "Keep", disabled: false };
    if (id === currentId) return { label: "In use", disabled: true };
    return { label: themes.some((t) => t.id === id) ? "Use" : "Get", disabled: false };
  };

  const guard = async (id: string, work: () => Promise<void>, fallback: string) => {
    if (busyId) return;
    setBusyId(id);
    try {
      await work();
    } catch (e) {
      toast({ message: message(e, fallback) });
    } finally {
      setBusyId(null);
      reload();
    }
  };

  /** *Get*: installs. *Use*: applies. While the theme is being tried, the button is *Keep*. */
  const act = (id: string, name: string) => {
    const a = actionOf(id);
    if (a.disabled) return Promise.resolve();
    return guard(
      id,
      async () => {
        if (trying?.storeId === id) {
          await ctl.keep();
          toast({ message: `${name} is installed and in use.` });
        } else if (a.label === "Use") {
          await themeApply(id);
        } else {
          await themeInstall(id);
          toast({ message: `${name} is installed.`, action: { label: "Use", run: () => void themeApply(id).catch(() => {}) } });
        }
      },
      "Could not install that theme.",
    );
  };

  const tryTheme = (id: string) => guard(id, () => ctl.start(id), "Could not try that theme.");
  /** Removes an installed theme with the 6 s undo; one in use hands the app back to its base theme (and gets its place on undo). */
  const remove = (id: string, name: string) => {
    const t = listed.find((x) => x.id === id);
    if (!t) return;
    const setHide = (on: boolean) =>
      setHidden((h) => {
        const next = new Set(h);
        if (on) next.add(id);
        else next.delete(id);
        return next;
      });
    const wasInUse = currentId === id;
    setHide(true);
    if (wasInUse) update((s) => ({ ...s, appearance: { ...s.appearance, theme: t.base } }));
    const timer = setTimeout(() => void themeRemove(id).catch(() => setHide(false)).finally(reload), REMOVE_UNDO_MS);
    toast({
      message: `Removed ${name}`,
      durationMs: REMOVE_UNDO_MS,
      undo: () => {
        clearTimeout(timer);
        setHide(false);
        if (wasInUse && choice) update((s) => ({ ...s, appearance: { ...s.appearance, theme: choice } }));
      },
    });
  };

  /** The right-click / ⋯ menu of a theme. */
  const menuOf = (th: { id: string; name: string }): MenuItem[] =>
    storeThemeMenu(
      { action: actionOf(th.id).label, installed: themes.some((x) => x.id === th.id) },
      {
        primary: () => void act(th.id, th.name),
        tryIt: () => void tryTheme(th.id),
        details: () => navigate(`/themes/${encodeURIComponent(th.id)}`),
        remove: () => remove(th.id, th.name),
      },
    );
  return { themes, currentId, busyId, trying, actionOf, act, tryTheme, menuOf, keep: () => ctl.keep().catch(() => {}), revert: () => ctl.end().catch(() => {}) };
}
