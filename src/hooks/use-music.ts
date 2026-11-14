import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "@/components/toast";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { MusicQuery } from "@/types/generated/MusicQuery";
import type { MusicTrack } from "@/types/generated/MusicTrack";
import type { SavedTrack } from "@/types/generated/SavedTrack";
import { useSettings } from "./use-settings";

const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });

/** The tracks in `<library>/music`: saved from Jamendo, or dropped in by the user. */
export function useLibraryMusic() {
  const q = useQuery({
    queryKey: queryKeys.libraryMusic,
    queryFn: () => invoke<SavedTrack[]>("list_library_music"),
    staleTime: 30_000,
  });
  return { tracks: q.data ?? [], isLoading: q.isLoading };
}

/**
 * Searching Jamendo. Disabled until a client id is in Settings — Lumen makes no request
 * at all before then, which is what "the network is opt-in" means.
 */
export function useMusicSearch(query: MusicQuery, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.musicSearch(query),
    queryFn: () => invoke<MusicTrack[]>("music_search", { query }),
    enabled,
    staleTime: 60_000,
    retry: false,
  });
}

/** Whether the music browser may talk to Jamendo at all. */
export function useMusicEnabled() {
  const { settings } = useSettings();
  return Boolean(settings?.music?.jamendoClientId?.trim());
}

export function useSaveMusic() {
  const qc = useQueryClient();
  const save = useMutation({
    mutationFn: (track: MusicTrack) => invoke<SavedTrack>("music_download", { track }),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.libraryMusic }),
    onError: (e) => fail(e, "Could not save that track."),
  });
  const remove = useMutation({
    mutationFn: (path: string) => invoke<void>("delete_library_music", { path }),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.libraryMusic }),
    onError: (e) => fail(e, "Could not remove that track."),
  });
  return { save: save.mutateAsync, remove: remove.mutateAsync, saving: save.isPending };
}
