import { useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { toast } from "@/components/toast";
import { invalidateLibraryData } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import { createStore } from "@/lib/store";

/** Optimistic favorite state by id until the next library change arrives. */
export const favoriteOverrides = createStore<{ map: ReadonlyMap<number, boolean> }>({ map: new Map() });

export function clearFavoriteOverrides() {
  favoriteOverrides.set({ map: new Map() });
}

function setOverrides(ids: number[], value: boolean | undefined) {
  favoriteOverrides.set((s) => {
    const map = new Map(s.map);
    for (const id of ids) {
      if (value === undefined) map.delete(id);
      else map.set(id, value);
    }
    return { map };
  });
}

export function useFavorites() {
  const qc = useQueryClient();

  /**
   * The whole optimistic round trip for one change. Undo is the same change with the
   * value flipped, so it runs through here too — writing only to the backend left the
   * override saying "favorite" and the tile's star stayed lit after undoing.
   */
  const apply = useCallback(
    async (ids: number[], value: boolean) => {
      setOverrides(ids, value);
      try {
        await invoke("set_favorite", { ids, value });
      } catch (e) {
        setOverrides(ids, undefined);
        toast({ message: isAppError(e) ? e.message : "Could not update favorites." });
        return false;
      }
      // Drop the override once the refetched rows carry the change, so the store cannot
      // shadow the catalogue indefinitely. It was never cleared before, which meant the
      // first favourite of a session pinned that tile's star for good.
      await invalidateLibraryData(qc);
      setOverrides(ids, undefined);
      return true;
    },
    [qc],
  );

  const setFavorite = useCallback(
    async (ids: number[], value: boolean, opts: { silent?: boolean } = {}) => {
      if (!(await apply(ids, value)) || opts.silent) return;
      const n = ids.length;
      toast({
        message: value
          ? n === 1
            ? "Added to Favorites"
            : `Added ${n} to Favorites`
          : n === 1
            ? "Removed from Favorites"
            : `Removed ${n} from Favorites`,
        undo: () => void apply(ids, !value),
      });
    },
    [apply],
  );

  return { setFavorite };
}
