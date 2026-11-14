import type { QueryClient } from "@tanstack/react-query";
import { toast } from "@/components/toast";
import { clearSelection } from "@/hooks/use-selection";
import { invalidateLibraryData } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";

const items = (n: number) => (n === 1 ? "1 photo" : `${n.toLocaleString()} photos`);

/**
 * Moves photos into the Hidden folder: they leave every other view. The toast's Undo works
 * without the password for a few seconds (the backend remembers exactly what just moved).
 */
export async function moveToHidden(qc: QueryClient, ids: number[]): Promise<void> {
  if (ids.length === 0) return;
  try {
    const moved = await invoke<number[]>("move_to_vault", { ids });
    clearSelection();
    void invalidateLibraryData(qc);
    toast({
      message: `Moved ${items(ids.length)} to Hidden`,
      undo: () => void invoke("move_from_vault", { ids: moved }).then(() => invalidateLibraryData(qc)),
    });
  } catch (e) {
    toast({ message: isAppError(e) ? e.message : "Could not hide those photos." });
  }
}

/** Moves photos from the Hidden folder back into the library (the folder is open). */
export async function moveToLibrary(qc: QueryClient, ids: number[]): Promise<void> {
  if (ids.length === 0) return;
  try {
    const moved = await invoke<number[]>("move_from_vault", { ids });
    clearSelection();
    void invalidateLibraryData(qc);
    toast({
      message: `Moved ${items(ids.length)} back to the Library`,
      undo: () => void invoke("move_to_vault", { ids: moved }).then(() => invalidateLibraryData(qc)),
    });
  } catch (e) {
    toast({ message: isAppError(e) ? e.message : "Could not move those photos." });
  }
}

/** Whether the page is the Hidden folder, where "Move to Hidden" makes no sense. */
export const inHiddenFolder = () => typeof window !== "undefined" && window.location.pathname.startsWith("/hidden");
