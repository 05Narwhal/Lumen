import { createStore } from "@/lib/store";

export const quickLookStore = createStore<{ id: number | null }>({ id: null });
export const openQuickLook = (id: number) => quickLookStore.set({ id });
export const closeQuickLook = () => quickLookStore.set({ id: null });
export const toggleQuickLook = (id: number) =>
  quickLookStore.set((s) => ({ id: s.id === id ? null : id }));
