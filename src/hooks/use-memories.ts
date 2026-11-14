import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { toast } from "@/components/toast";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { MemorySummary } from "@/types/generated/MemorySummary";

/** Trips, events and "on this day", regenerated in Rust after every import. */
export function useMemories({ includeHidden = false }: { includeHidden?: boolean } = {}) {
  const q = useQuery({
    queryKey: queryKeys.memories(includeHidden),
    queryFn: () => invoke<MemorySummary[]>("list_memories", { includeHidden }),
  });
  return { memories: q.data ?? [], isLoading: q.isLoading };
}

export function useMemory(id: number) {
  return useQuery({ queryKey: queryKeys.memory(id), queryFn: () => invoke<MemorySummary>("get_memory", { id }), retry: false });
}

/** Hiding is reversible, so it never asks: it toasts with Undo. */
export function useHideMemory() {
  const qc = useQueryClient();
  return useCallback(
    async (m: MemorySummary, hidden: boolean) => {
      const set = async (value: boolean) => {
        await invoke("set_memory_hidden", { id: m.id, hidden: value });
        await Promise.all([qc.invalidateQueries({ queryKey: ["memories"] }), qc.invalidateQueries({ queryKey: queryKeys.memory(m.id) })]);
      };
      await set(hidden);
      toast({ message: hidden ? "Memory hidden" : "Memory shown again", undo: () => void set(!hidden) });
    },
    [qc],
  );
}

export const MEMORY_KIND = { trip: 0, event: 1, onThisDay: 2 } as const;
