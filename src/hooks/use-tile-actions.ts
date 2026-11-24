import { useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { useInRouterContext, useNavigate } from "react-router";
import { pickFolder } from "@/lib/dialogs";
import { openAlbumPicker } from "@/components/album-picker/store";
import type { MenuItem } from "@/components/context-menu";
import { toast } from "@/components/toast";
import { isMac } from "@/components/window-controls";
import { invalidateLibraryData } from "@/lib/events";
import { openDevelop } from "@/lib/develop-source";
import { requestFaceTagging } from "@/lib/face-tagging";
import { appsFor, chooseAnotherApp, defaultFor, editIn, useExternalApps, type MediaFacts } from "@/lib/external-apps";
import { invoke, isAppError } from "@/lib/ipc";
import { photoMenu } from "@/lib/menus";
import { useMenuContributions } from "@/lib/plugin-slots";
import { rotatePhotos } from "@/lib/rotate";
import { inHiddenFolder, moveToHidden } from "@/lib/vault";
import { clearSelection } from "./use-selection";
import { useFavorites } from "./use-favorites";
import { useRatings } from "./use-ratings";

/** The actions every tile surface shares: context menu, selection bar, viewer. */
export function useTileActions() {
  const qc = useQueryClient();
  const { setFavorite } = useFavorites();
  const { setRating } = useRatings();
  const { apps, defaults } = useExternalApps();
  // Every grid renders under the router; a hook test without one simply gets no *Develop*. The
  // branch never changes for a mounted component, so the hook order stays stable.
  const navigate = useInRouterContext() ? useNavigate() : null;
  const contributions = useMenuContributions("library.selection-menu", navigate);

  const trash = useCallback(
    async (ids: number[]) => {
      try {
        await invoke("trash_assets", { ids });
        clearSelection();
        invalidateLibraryData(qc);
        toast({
          message: ids.length === 1 ? "Moved to Trash" : `Moved ${ids.length} to Trash`,
          undo: () => void invoke("restore_assets", { ids }).then(() => invalidateLibraryData(qc)),
        });
      } catch (e) {
        toast({ message: isAppError(e) ? e.message : "Could not move to Trash." });
      }
    },
    [qc],
  );

  const rotate = useCallback((ids: number[], dir: 1 | -1) => rotatePhotos(qc, ids, dir), [qc]);

  const reveal = useCallback((id: number) => {
    invoke("reveal_asset", { id }).catch((e: unknown) =>
      toast({ message: isAppError(e) ? e.message : "Could not open the file manager." }),
    );
  }, []);

  // Copies the originals into a folder the user picks. Copies, never moves.
  const copyTo = useCallback(async (ids: number[]) => {
    const dir = await pickFolder({ title: "Copy files to…" });
    if (typeof dir !== "string") return;
    try {
      const n = await invoke<number>("copy_assets_to", { ids, dir });
      clearSelection();
      toast({ message: n === 1 ? "Copied 1 file" : `Copied ${n} files` });
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not copy the files." });
    }
  }, []);

  const menuItemsFor = useCallback(
    (ids: number[], ctx: { favorite: boolean; rating?: number; onOpen?: (id: number) => void; /** What the photos are, for the apps that fit (Round 22 §9). */ media?: MediaFacts[] }): MenuItem[] =>
      photoMenu(
        ids,
        {
          favorite: ctx.favorite,
          rating: ctx.rating,
          inHidden: inHiddenFolder(),
          mac: isMac(),
          contributions,
          openWith: ctx.media && { apps: appsFor(apps, ctx.media), defaultApp: defaultFor(ctx.media, apps, defaults), mac: isMac() },
        },
        {
          open: ctx.onOpen,
          // Stills only: a video has nothing to develop.
          develop:
            navigate && ctx.media?.length === 1 && ctx.media[0].mediaType !== 1
              ? (id) => openDevelop(navigate, {}, id)
              : undefined,
          favorite: (ids, on) => void setFavorite(ids, on),
          rate: (ids, stars) => void setRating(ids, stars, ids.length === 1 ? (ctx.rating ?? 0) : undefined),
          addToAlbum: openAlbumPicker,
          tagPeople: ctx.onOpen
            ? (id) => {
                requestFaceTagging(id);
                ctx.onOpen!(id);
              }
            : undefined,
          rotate: (ids, dir) => void rotate(ids, dir),
          reveal,
          copyTo: (ids) => void copyTo(ids),
          openWith: {
            editIn: (ids, appId) => {
              const app = apps.find((a) => a.id === appId);
              if (app) void editIn(qc, ids, app);
            },
            chooseApp: (ids) => void chooseAnotherApp(qc, ids),
          },
          moveToHidden: (ids) => void moveToHidden(qc, ids),
          trash: (ids) => void trash(ids),
        },
      ),
    [setFavorite, setRating, reveal, trash, copyTo, rotate, qc, apps, defaults, navigate, contributions],
  );

  return { trash, reveal, copyTo, rotate, menuItemsFor, setFavorite };
}
