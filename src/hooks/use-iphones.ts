import { checkRoom } from "@/lib/import-space";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { IphonesUpdate } from "@/types/generated/IphonesUpdate";

/**
 * Connected iPhones, live: the watcher in Rust always runs and pushes `device://iphones`, so a
 * phone plugged in shows up in the sidebar at once.
 */
export function useIphones() {
  const qc = useQueryClient();
  const q = useQuery({
    queryKey: queryKeys.iphones,
    queryFn: () => invoke<IphonesUpdate>("list_iphones"),
    staleTime: 5_000,
  });

  const startImport = useCallback(
    async (udid: string) => {
      const room = await checkRoom({ kind: "iphone", id: udid });
      if (room === false) return null;
      const id = await invoke<number>("start_iphone_import", { udid, limitBytes: room });
      qc.invalidateQueries({ queryKey: queryKeys.batches });
      return id;
    },
    [qc],
  );

  return {
    devices: q.data?.devices ?? [],
    /** False when Apple's device drivers are missing, which the UI explains. */
    usbmuxdAvailable: q.data?.usbmuxdAvailable ?? true,
    isLoading: q.isLoading,
    startImport,
  };
}
