import { useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { toast } from "@/components/toast";
import { invalidateLibraryData } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import { createStore } from "@/lib/store";

/** Optimistic ratings by id, until the next library change arrives. */
export const ratingOverrides = createStore<{ map: ReadonlyMap<number, number> }>({ map: new Map() });

export function useRatings() {
  const qc = useQueryClient();
  const setRating = useCallback(
    async (ids: number[], rating: number, previous?: number) => {
      if (ids.length === 0) return;
      const next = new Map(ratingOverrides.get().map);
      for (const id of ids) next.set(id, rating);
      ratingOverrides.set({ map: next });
      try {
        await invoke("set_rating", { ids, rating });
        invalidateLibraryData(qc);
        if (previous !== undefined && ids.length === 1) {
          toast({
            message: rating === 0 ? "Rating cleared" : `Rated ${rating} star${rating === 1 ? "" : "s"}`,
            undo: () => void invoke("set_rating", { ids, rating: previous }).then(() => invalidateLibraryData(qc)),
          });
        }
      } catch (e) {
        const revert = new Map(ratingOverrides.get().map);
        for (const id of ids) revert.delete(id);
        ratingOverrides.set({ map: revert });
        toast({ message: isAppError(e) ? e.message : "Could not set the rating." });
      }
    },
    [qc],
  );
  return { setRating };
}
