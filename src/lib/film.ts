import { toast } from "@/components/toast";
import { askExportFolder } from "./export-folder";
import { invoke, isAppError } from "./ipc";
import { createStore } from "./store";

/** The memory whose film is playing (Round 4F), with the picture to show while it is made. */
export interface FilmRequest {
  memoryId: number;
  title: string;
  coverAssetId: number | null;
}

export const filmStore = createStore<FilmRequest | null>(null);

/** *Play* on a memory: its film opens over everything (`FilmPlayerHost`). */
export function playFilm(request: FilmRequest) {
  filmStore.set(request);
}

export function closeFilm() {
  filmStore.set(null);
}

/**
 * *Export video* on a memory (Round 6): asks where to save, then its film (made first if it
 * never was) is exported in the background, the activity ring showing how far. Nothing is
 * added to Slideshows.
 */
export async function exportMemoryFilm(memoryId: number) {
  const destDir = await askExportFolder("Export memory to…");
  if (destDir === null) return;
  try {
    await invoke("export_memory_film", { memoryId, destDir });
  } catch (e) {
    toast({ message: isAppError(e) ? e.message : "The video could not be made." });
  }
}
