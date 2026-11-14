import { useQuery } from "@tanstack/react-query";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { FoundPhotosLibrary } from "@/types/generated/FoundPhotosLibrary";

/**
 * Apple Photos libraries Lumen found by itself: in Pictures, at the top of each drive and on
 * connected Mac disks (Round 9B). Refetched when drives or Mac disks come and go
 * (`events.ts`), so the Import screen offers Apple Photos only while there is one.
 */
export function useAppleLibraries() {
  return useQuery({
    queryKey: queryKeys.appleLibraries,
    queryFn: () => invoke<FoundPhotosLibrary[]>("find_apple_photos_libraries"),
    staleTime: 60_000,
  });
}

/** Where a found library opens: its own page, or its Mac disk's (which lists it). */
export function appleLibraryLink(lib: FoundPhotosLibrary): string {
  return lib.disk ? `/connected/mac/${encodeURIComponent(lib.disk)}` : `/connected/photos/${encodeURIComponent(lib.path)}`;
}
