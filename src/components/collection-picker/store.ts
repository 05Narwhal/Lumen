import { createStore } from "@/lib/store";

/** The album the palette's *Move this album to collection…* is moving (Round 5A). */
export interface MovingAlbum {
  id: number;
  name: string;
  collectionId: number | null;
}

export const collectionPickerStore = createStore<{ album: MovingAlbum | null }>({ album: null });
export const openCollectionPicker = (album: MovingAlbum) => collectionPickerStore.set({ album });
export const closeCollectionPicker = () => collectionPickerStore.set({ album: null });
