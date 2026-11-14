import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { PhotosLibraryInfo } from "@/types/generated/PhotosLibraryInfo";

/**
 * An Apple Photos library on a Mac disk (`disk` = its id) or on this computer (`disk` =
 * null). Opening one copies its database, which takes seconds on a USB hard drive, so the
 * result is kept for a minute and never retried automatically.
 */
export function usePhotosLibrary(disk: string | null, path: string) {
  const qc = useQueryClient();
  const q = useQuery({
    queryKey: queryKeys.photosLibrary(disk, path),
    queryFn: () => invoke<PhotosLibraryInfo>("get_photos_library", { disk, path }),
    retry: false,
    staleTime: 60_000,
  });
  const startImport = useCallback(async () => {
    const id = await invoke<number>("start_photos_library_import", { disk, path });
    qc.invalidateQueries({ queryKey: queryKeys.batches });
    return id;
  }, [qc, disk, path]);
  return { info: q.data, isLoading: q.isLoading, error: q.error, startImport };
}
