import { createStore } from "@/lib/store";
import type { ExportLoss } from "@/types/generated/ExportLoss";

/** Which photos the export sheet is open for; null when closed. */
export const exportSheetStore = createStore<{ ids: number[] | null }>({ ids: null });

export const openExportSheet = (ids: number[]) => exportSheetStore.set({ ids });
export const closeExportSheet = () => exportSheetStore.set({ ids: null });

/** The files an export could not carry everything into, shown by *Details*; null when closed. */
export const exportLossesStore = createStore<{ losses: ExportLoss[] | null }>({ losses: null });

export const openExportLosses = (losses: ExportLoss[]) => exportLossesStore.set({ losses });
export const closeExportLosses = () => exportLossesStore.set({ losses: null });
