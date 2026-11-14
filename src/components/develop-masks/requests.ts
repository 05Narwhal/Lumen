import { createStore } from "@/lib/store";
import type { AiMaskKind } from "@/types/generated/AiMaskKind";

/**
 * The palette asking the open Masks panel for a new AI mask (Phase 8f), like `autoRequests`:
 * a counter so asking twice for the same target still runs twice.
 */
export const aiMaskRequests = createStore<{ n: number; kind: Exclude<AiMaskKind, "person"> | null }>({ n: 0, kind: null });

export const requestAiMask = (kind: Exclude<AiMaskKind, "person">) => aiMaskRequests.set((r) => ({ n: r.n + 1, kind }));
