import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "@/components/toast";
import { rememberAlbum } from "@/lib/album-recent";
import { invalidateLibraryData } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";
import type { AssetQuery } from "@/types/generated/AssetQuery";

const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });

export function useAlbums() {
  const qc = useQueryClient();
  const albums = useQuery({
    queryKey: queryKeys.albums,
    queryFn: () => invoke<AlbumSummary[]>("list_albums"),
    staleTime: 30_000,
  });
  const refresh = () => {
    qc.invalidateQueries({ queryKey: queryKeys.albums });
    invalidateLibraryData(qc);
  };

  const create = useMutation({
    // `hides`: an album whose photos leave the rest of the library (school work, say).
    mutationFn: async (args: string | { name: string; hides: boolean }) => {
      const { name, hides } = typeof args === "string" ? { name: args, hides: false } : args;
      const album = await invoke<AlbumSummary>("create_album", { name, parentId: null });
      if (hides) await invoke<void>("set_album_hides", { id: album.id, hides: true });
      return album;
    },
    onSuccess: refresh,
    onError: (e) => fail(e, "Could not create the album."),
  });
  const createSmart = useMutation({
    mutationFn: (args: { name: string; query: AssetQuery }) => invoke<AlbumSummary>("create_smart_album", args),
    onSuccess: refresh,
    onError: (e) => fail(e, "Could not create the smart album."),
  });
  const updateSmart = useMutation({
    mutationFn: (args: { id: number; name: string; query: AssetQuery }) => invoke<void>("update_smart_album", args),
    onSuccess: refresh,
    onError: (e) => fail(e, "Could not save the smart album."),
  });
  const rename = useMutation({
    mutationFn: (args: { id: number; name: string }) => invoke<void>("rename_album", args),
    onSuccess: refresh,
    onError: (e) => fail(e, "Could not rename the album."),
  });
  const remove = useMutation({
    mutationFn: (id: number) => invoke<void>("delete_album", { id }),
    onSuccess: refresh,
    onError: (e) => fail(e, "Could not delete the album."),
  });
  const setHides = useMutation({
    mutationFn: (args: { id: number; hides: boolean; name: string }) => invoke<void>("set_album_hides", { id: args.id, hides: args.hides }),
    onSuccess: (_, args) => {
      refresh();
      toast({
        message: args.hides ? `“${args.name}” is hidden from the Library` : `“${args.name}” is back in the Library`,
        undo: () => void invoke("set_album_hides", { id: args.id, hides: !args.hides }).then(refresh),
      });
    },
    onError: (e) => fail(e, "Could not change the album."),
  });
  const addTo = useMutation({
    mutationFn: (args: { albumId: number; assetIds: number[]; albumName?: string }) =>
      invoke<number>("add_to_album", { albumId: args.albumId, assetIds: args.assetIds }),
    onSuccess: (added, args) => {
      refresh();
      rememberAlbum(args.albumId);
      const name = args.albumName ?? "album";
      // Into an album that hides its photos, they leave the Library: say so.
      const hides = qc.getQueryData<AlbumSummary[]>(queryKeys.albums)?.find((a) => a.id === args.albumId)?.hidesItems;
      const moved = added === 1 ? "Moved 1 photo out of the Library" : `Moved ${added} photos out of the Library`;
      toast({
        message: added === 0 ? `Already in ${name}` : hides ? moved : `Added ${added} to ${name}`,
        undo:
          added > 0
            ? () => void invoke("remove_from_album", { albumId: args.albumId, assetIds: args.assetIds }).then(refresh)
            : undefined,
      });
    },
    onError: (e) => fail(e, "Could not add to the album."),
  });
  const removeFrom = useMutation({
    mutationFn: (args: { albumId: number; assetIds: number[] }) => invoke<number>("remove_from_album", args),
    onSuccess: (removed, args) => {
      refresh();
      toast({
        message: removed === 1 ? "Removed from album" : `Removed ${removed} from album`,
        undo: () => void invoke("add_to_album", { albumId: args.albumId, assetIds: args.assetIds }).then(refresh),
      });
    },
    onError: (e) => fail(e, "Could not remove from the album."),
  });
  const move = useMutation({
    mutationFn: (args: { albumId: number; assetId: number; before: number | null }) => invoke<void>("move_in_album", args),
    onSuccess: refresh,
  });
  const setCover = useMutation({
    mutationFn: (args: { albumId: number; assetId: number | null }) => invoke<void>("set_album_cover", args),
    onSuccess: () => {
      refresh();
      toast({ message: "Album cover updated" });
    },
  });

  return {
    albums: albums.data ?? [],
    isLoading: albums.isLoading,
    create: create.mutateAsync,
    createSmart: createSmart.mutateAsync,
    updateSmart: updateSmart.mutateAsync,
    rename: rename.mutateAsync,
    setHides: setHides.mutateAsync,
    remove: remove.mutateAsync,
    addTo: addTo.mutateAsync,
    removeFrom: removeFrom.mutateAsync,
    move: move.mutateAsync,
    setCover: setCover.mutateAsync,
  };
}
