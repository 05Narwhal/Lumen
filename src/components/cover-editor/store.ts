import type { CoverItem } from "@/components/cover-stack";
import { createStore } from "@/lib/store";
import type { CoverStyle } from "@/types/generated/CoverStyle";

/** What the cover editor is changing: an album or a collection, and how it looks now. */
export interface CoverTarget {
  kind: "album" | "collection";
  id: number;
  name: string;
  style: CoverStyle | null;
  /** The prints shown when it has no icon cover (the preview's *Photos* side). */
  photos: CoverItem[];
}

export const coverEditorStore = createStore<{ target: CoverTarget | null }>({ target: null });
export const openCoverEditor = (target: CoverTarget) => coverEditorStore.set({ target });
export const closeCoverEditor = () => coverEditorStore.set({ target: null });
