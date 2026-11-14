import type { QueryClient } from "@tanstack/react-query";
import { toast } from "@/components/toast";
import { invalidateLibraryData } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import type { PreviousRecipe } from "@/types/generated/PreviousRecipe";

/**
 * Turn photos a quarter (`1` clockwise, `-1` anticlockwise). A turn is an edit: the file is
 * untouched, and the toast's Undo puts the old recipes back. The selection stays: a half
 * turn is two presses (Round 4A keeps it, like a rating, rather than clearing it).
 */
export async function rotatePhotos(qc: QueryClient, ids: number[], dir: 1 | -1): Promise<void> {
  if (ids.length === 0) return;
  try {
    const previous = await invoke<PreviousRecipe[]>("rotate_assets", { ids, dir });
    if (previous.length === 0) return;
    void invalidateLibraryData(qc);
    toast({
      message: previous.length === 1 ? "Rotated 1 photo" : `Rotated ${previous.length} photos`,
      undo: () => void invoke("restore_recipes", { previous }).then(() => invalidateLibraryData(qc)),
    });
  } catch (e) {
    toast({ message: isAppError(e) ? e.message : "Could not rotate." });
  }
}
