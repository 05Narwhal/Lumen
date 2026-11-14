import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "@/components/toast";
import { invalidateLibraryData } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { AlbumCollection } from "@/types/generated/AlbumCollection";
import type { CollectionUndo } from "@/types/generated/CollectionUndo";

const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });

/** Collections (Round 5A): folders of albums. Moves and deletes offer Undo; nothing confirms. */
export function useAlbumCollections() {
  const qc = useQueryClient();
  const list = useQuery({
    queryKey: queryKeys.albumCollections,
    queryFn: () => invoke<AlbumCollection[]>("list_album_collections"),
    staleTime: 30_000,
  });
  const refresh = () => {
    void qc.invalidateQueries({ queryKey: queryKeys.albums });
    void invalidateLibraryData(qc);
  };
  const nameOf = (id: number | null) => (id == null ? null : (qc.getQueryData<AlbumCollection[]>(queryKeys.albumCollections)?.find((c) => c.id === id)?.name ?? "the collection"));

  const create = useMutation({
    mutationFn: (args: { name: string; parentId: number | null }) => invoke<number>("create_album_collection", args),
    onSuccess: refresh,
    onError: (e) => fail(e, "Could not create the collection."),
  });
  const rename = useMutation({
    mutationFn: (args: { id: number; name: string }) => invoke<void>("rename_album_collection", args),
    onSuccess: refresh,
    onError: (e) => fail(e, "Could not rename the collection."),
  });
  const moveAlbum = useMutation({
    mutationFn: (args: { albumId: number; collectionId: number | null; albumName: string }) =>
      invoke<number | null>("move_album_to_collection", { albumId: args.albumId, collectionId: args.collectionId }),
    onSuccess: (was, args) => {
      refresh();
      if (was === args.collectionId) return;
      const into = nameOf(args.collectionId);
      toast({
        message: into ? `Moved “${args.albumName}” to ${into}` : `Took “${args.albumName}” out of ${nameOf(was)}`,
        undo: () => void invoke("move_album_to_collection", { albumId: args.albumId, collectionId: was }).then(refresh),
      });
    },
    onError: (e) => fail(e, "Could not move the album."),
  });
  const moveCollection = useMutation({
    mutationFn: (args: { id: number; into: number | null; name: string }) => invoke<number | null>("move_album_collection", { id: args.id, into: args.into }),
    onSuccess: (was, args) => {
      refresh();
      if (was === args.into) return;
      const into = nameOf(args.into);
      toast({
        message: into ? `Moved “${args.name}” to ${into}` : `Moved “${args.name}” to the top`,
        undo: () => void invoke("move_album_collection", { id: args.id, into: was }).then(refresh),
      });
    },
    onError: (e) => fail(e, "Could not move the collection."),
  });
  const remove = useMutation({
    mutationFn: (id: number) => invoke<CollectionUndo>("delete_album_collection", { id }),
    onSuccess: (undo) => {
      refresh();
      const held = undo.albums.length + undo.collections.length;
      toast({
        message: held > 0 ? `Deleted “${undo.collection.name}”; what it held moved up` : `Deleted “${undo.collection.name}”`,
        undo: () => void invoke("restore_album_collection", { undo }).then(refresh),
      });
    },
    onError: (e) => fail(e, "Could not delete the collection."),
  });

  return {
    collections: list.data ?? [],
    isLoading: list.isLoading,
    create: create.mutateAsync,
    rename: rename.mutateAsync,
    moveAlbum: moveAlbum.mutateAsync,
    moveCollection: moveCollection.mutateAsync,
    remove: remove.mutateAsync,
  };
}
