import { checkRoom } from "@/lib/import-space";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { ImportBatchSummary } from "@/types/generated/ImportBatchSummary";
import type { ImportFailure } from "@/types/generated/ImportFailure";
import type { ImportSkip } from "@/types/generated/ImportSkip";

export const BATCH_RUNNING = 0;
export const BATCH_COMPLETE = 1;
export const BATCH_FAILED = 2;
export const BATCH_CANCELLED = 3;
/** Cut short (device unplugged, disk nearly full, app closed); importing again finishes it. */
export const BATCH_INTERRUPTED = 4;

export function useImport() {
  const qc = useQueryClient();
  const batches = useQuery({
    queryKey: queryKeys.batches,
    queryFn: () => invoke<ImportBatchSummary[]>("list_import_batches", { limit: 50 }),
    staleTime: 5_000,
  });
  const start = useMutation({
    /** `asAlbums`: each folder with photos becomes an album, each with subfolders a collection. */
    mutationFn: async ({ path, recursive = true, asAlbums = false }: { path: string; recursive?: boolean; asAlbums?: boolean }) => {
      const room = await checkRoom({ kind: "folder", id: path, recursive });
      if (room === false) return null;
      return invoke<number>("start_folder_import", { path, recursive, limitBytes: room, ...(asAlbums ? { asAlbums } : {}) });
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.batches }),
  });
  const startFiles = useMutation({
    mutationFn: async (paths: string[]) => {
      const room = await checkRoom({ kind: "files", paths });
      if (room === false) return null;
      return invoke<number>("start_files_import", { paths, limitBytes: room });
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.batches }),
  });
  const cancel = useMutation({
    mutationFn: (batchId: number) => invoke<void>("cancel_import", { batchId }),
  });
  return {
    batches: batches.data ?? [],
    isLoading: batches.isLoading,
    startFolderImport: start.mutateAsync,
    startFilesImport: startFiles.mutateAsync,
    cancelImport: cancel.mutateAsync,
  };
}

/** Every file an import did not bring in, with the reason (Round 19). */
export function useImportSkips(batchId: number | null) {
  return useQuery({
    queryKey: queryKeys.skips(batchId ?? -1),
    queryFn: () => invoke<ImportSkip[]>("get_import_skips", { batchId }),
    enabled: batchId !== null,
  });
}

export function useImportFailures(batchId: number | null) {
  return useQuery({
    queryKey: queryKeys.failures(batchId ?? -1),
    queryFn: () => invoke<ImportFailure[]>("get_import_failures", { batchId }),
    enabled: batchId !== null,
  });
}
