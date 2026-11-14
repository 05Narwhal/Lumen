import { useQuery } from "@tanstack/react-query";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { MacDiskInfo } from "@/types/generated/MacDiskInfo";

/** Mac-formatted disks the watcher has opened (Windows); live via `device://macdisks`. */
export function useMacDisks() {
  const q = useQuery({ queryKey: queryKeys.macDisks, queryFn: () => invoke<MacDiskInfo[]>("list_mac_disks"), staleTime: Infinity });
  return { disks: q.data ?? [] };
}
