import { useCallback, useState } from "react";
import { toast } from "@/components/toast";
import { useSettings } from "@/hooks/use-settings";
import { useInstalledThemes } from "@/lib/plugin-theme";
import { themeRemove } from "@/lib/theme-store";
import type { InstalledTheme } from "@/types/generated/InstalledTheme";

/** How long a removed theme can be brought back (the app's undo window). */
export const REMOVE_UNDO_MS = 6000;

/**
 * Settings → Appearance's installed themes. *Remove* hides the theme and shows the 6 s undo
 * toast; the files go only when the time is up (`theme_remove`), so *Undo* really brings
 * everything back. A removed theme that was in use hands the app to its base theme at once and
 * gets its place back on undo.
 */
export function useInstalledThemeList() {
  const all = useInstalledThemes();
  const { settings, update } = useSettings();
  const [hidden, setHidden] = useState<ReadonlySet<string>>(new Set());
  const themes = (all ?? []).filter((t) => !hidden.has(t.id));
  const choice = settings?.appearance.theme;

  const remove = useCallback(
    (t: InstalledTheme) => {
      const wasInUse = !!choice && typeof choice === "object" && choice.plugin === t.id;
      const setHide = (on: boolean) =>
        setHidden((h) => {
          const next = new Set(h);
          if (on) next.add(t.id);
          else next.delete(t.id);
          return next;
        });
      setHide(true);
      if (wasInUse) update((s) => ({ ...s, appearance: { ...s.appearance, theme: t.base } }));
      const timer = setTimeout(() => void themeRemove(t.id).catch(() => setHide(false)), REMOVE_UNDO_MS);
      toast({
        message: `Removed ${t.name}`,
        durationMs: REMOVE_UNDO_MS,
        undo: () => {
          clearTimeout(timer);
          setHide(false);
          if (wasInUse && choice) update((s) => ({ ...s, appearance: { ...s.appearance, theme: choice } }));
        },
      });
    },
    [choice, update],
  );

  return { themes, remove };
}
