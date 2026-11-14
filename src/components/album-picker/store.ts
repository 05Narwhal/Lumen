import { createStore } from "@/lib/store";

export const albumPickerStore = createStore<{ ids: number[] | null }>({ ids: null });
export const openAlbumPicker = (ids: number[]) => albumPickerStore.set({ ids });
export const closeAlbumPicker = () => albumPickerStore.set({ ids: null });
