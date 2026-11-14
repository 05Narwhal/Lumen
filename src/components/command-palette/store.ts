import { createStore } from "@/lib/store";

/** A second list the palette shows instead of the commands (*Open with…*, Round 22 lane C). */
export interface PaletteSub {
  title: string;
  items: { id: string; label: string; iconUrl?: string; run: () => void }[];
}

export const paletteStore = createStore<{ open: boolean; sub?: PaletteSub | null }>({ open: false, sub: null });
export const openPalette = () => paletteStore.set({ open: true, sub: null });
export const openPaletteSub = (sub: PaletteSub) => paletteStore.set({ open: true, sub });
export const closePalette = () => paletteStore.set({ open: false, sub: null });
