import type { ThemeChoice } from "@/types/generated/ThemeChoice";

/** How long *Try* shows a theme before putting the previous one back, unless it is kept. */
export const TRY_MS = 30_000;

export interface TryState {
  /** The store id being tried. */
  storeId: string;
  name: string;
  /** `Date.now()` when it reverts by itself. */
  until: number;
}

export interface TryDeps {
  /** Downloads and checks the theme (`theme_try`); the installed id is the one `apply` takes. */
  tryTheme: (storeId: string) => Promise<{ id: string; name: string }>;
  apply: (id: string) => Promise<void>;
  /** `theme_try_end`. */
  end: () => Promise<void>;
  /** `theme_keep`. */
  keep: () => Promise<unknown>;
  /** The theme setting right now. */
  current: () => ThemeChoice | undefined;
  /** Puts a theme setting back. */
  restore: (choice: ThemeChoice) => void | Promise<void>;
}

/**
 * *Try*: shows a store theme without installing it and puts the previous theme back when the
 * person leaves, asks to, or 30 seconds pass; *Keep* installs it and stays. The theme in use
 * before the first try is the one that comes back, however many themes are tried in a row.
 */
export function createThemeTry(deps: TryDeps, onChange: (s: TryState | null) => void = () => {}, ms = TRY_MS) {
  let state: TryState | null = null;
  let previous: ThemeChoice | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const set = (s: TryState | null) => {
    state = s;
    onChange(s);
  };
  const stop = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  };

  async function end() {
    if (!state) return;
    stop();
    const back = previous;
    set(null);
    previous = undefined;
    // The setting first, so the page never falls back to the default theme in between.
    if (back !== undefined) await deps.restore(back);
    await deps.end();
  }

  return {
    state: () => state,
    async start(storeId: string) {
      const first = state === null;
      const before = deps.current();
      const t = await deps.tryTheme(storeId);
      if (first) previous = before;
      await deps.apply(t.id);
      stop();
      timer = setTimeout(() => void end().catch(() => {}), ms);
      set({ storeId, name: t.name, until: Date.now() + ms });
    },
    async keep() {
      if (!state) return;
      stop();
      await deps.keep();
      previous = undefined;
      set(null);
    },
    end,
  };
}
