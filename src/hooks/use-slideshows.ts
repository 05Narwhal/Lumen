import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "@/components/toast";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { SlideshowDetail } from "@/types/generated/SlideshowDetail";
import type { SlideshowDoc } from "@/types/generated/SlideshowDoc";
import type { SlideshowSummary } from "@/types/generated/SlideshowSummary";

const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });

/** Every slideshow, and the ways one is made or removed. */
export function useSlideshows() {
  const qc = useQueryClient();
  const slideshows = useQuery({
    queryKey: queryKeys.slideshows,
    queryFn: () => invoke<SlideshowSummary[]>("list_slideshows"),
    staleTime: 30_000,
  });
  const refresh = () => qc.invalidateQueries({ queryKey: queryKeys.slideshows });

  const create = useMutation({
    mutationFn: (args: { name: string; assetIds: number[] }) => invoke<number>("create_slideshow", args),
    onSuccess: refresh,
    onError: (e) => fail(e, "Could not make the slideshow."),
  });
  const createFromMemory = useMutation({
    mutationFn: (memoryId: number) => invoke<number>("create_slideshow_from_memory", { memoryId }),
    onSuccess: refresh,
    onError: (e) => fail(e, "Could not make the slideshow."),
  });
  /** Deleting is reversible, so it never confirms: the document comes back for Undo. */
  const remove = useMutation({
    mutationFn: (id: number) => invoke<SlideshowDetail | null>("delete_slideshow", { id }),
    onSuccess: (removed) => {
      refresh();
      toast({
        message: "Slideshow deleted",
        undo: removed
          ? () =>
              void invoke<number>("create_slideshow", { name: removed.name, assetIds: removed.doc.clips.flatMap((c) => [c.asset_id, ...c.extra]), arrange: false })
                .then((id) => invoke("save_slideshow", { id, name: removed.name, doc: removed.doc }))
                .then(refresh)
                .catch((e: unknown) => fail(e, "Could not bring the slideshow back."))
          : undefined,
      });
    },
    onError: (e) => fail(e, "Could not delete the slideshow."),
  });

  return {
    slideshows: slideshows.data ?? [],
    isLoading: slideshows.isLoading,
    create: create.mutateAsync,
    createFromMemory: createFromMemory.mutateAsync,
    remove: remove.mutateAsync,
  };
}

/** One slideshow's name and document, for the editor. */
export function useSlideshow(id: number) {
  return useQuery({
    queryKey: queryKeys.slideshow(id),
    queryFn: () => invoke<SlideshowDetail>("get_slideshow", { id }),
    retry: false,
  });
}

/** Saves the document as it is now; the editor debounces the calls. */
export function saveSlideshow(id: number, name: string, doc: SlideshowDoc) {
  return invoke<void>("save_slideshow", { id, name, doc });
}
