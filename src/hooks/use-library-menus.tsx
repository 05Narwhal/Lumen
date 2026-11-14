import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useState, type ReactNode } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { confirm, promptName } from "@/components/confirm-dialog";
import { CollectionPicker } from "@/components/collection-picker";
import { openCoverEditor } from "@/components/cover-editor";
import { photoItems } from "@/components/cover-stack";
import { PhotoPicker } from "@/components/photo-picker";
import { toast } from "@/components/toast";
import { useAlbumCollections } from "@/hooks/use-album-collections";
import { useAlbums } from "@/hooks/use-albums";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { openDevelop } from "@/lib/develop-source";
import { invoke, isAppError } from "@/lib/ipc";
import { collectionCovers } from "@/lib/album-collections";
import { duplicateAlbum, duplicateSlideshow, exportAlbum, exportScope, exportSlideshow, scopeIds, shuffleAlbumCover } from "@/lib/library-actions";
import { albumMenu, collectionMenu, setMenu, slideshowMenu, type MenuItem } from "@/lib/menus";
import { queryKeys } from "@/lib/query-keys";
import { setViewerSequence } from "@/lib/viewer-sequence";
import type { AlbumCollection } from "@/types/generated/AlbumCollection";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";
import type { LayoutScope } from "@/types/generated/LayoutScope";
import type { SlideshowDetail } from "@/types/generated/SlideshowDetail";
import type { SlideshowSummary } from "@/types/generated/SlideshowSummary";

const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });

/**
 * What a menu does with a whole set of photos it never loaded (Round 4G): export them, or
 * play them in the viewer four seconds apiece, as a memory's *Play* does.
 */
export function useSetActions() {
  const [, setParams] = useSearchParams();
  const playIds = useCallback(
    (ids: number[]) => {
      if (ids.length === 0) return;
      setViewerSequence(ids);
      setParams((p) => {
        p.set("asset", String(ids[0]));
        p.set("play", "1");
        return p;
      });
    },
    [setParams],
  );
  const playScope = useCallback(
    async (scope: LayoutScope) => {
      try {
        playIds(await scopeIds(scope));
      } catch (e) {
        fail(e, "Could not start the slideshow.");
      }
    },
    [playIds],
  );
  return { exportScope, playScope, playIds };
}

/**
 * A slideshow row's menu (Open · Duplicate · Rename… · — · Delete), through the commands the
 * editor already uses, so nothing here touches how a show is made or rendered (Phase F).
 * `remove` is `useSlideshows().remove`, which offers Undo.
 */
export function useSlideshowMenu(remove: (id: number) => Promise<unknown>) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  return useCallback(
    (show: SlideshowSummary): MenuItem[] =>
      slideshowMenu({
        open: () => navigate(`/slideshows/${show.id}`),
        export: () => void exportSlideshow(show.id),
        duplicate: () => void duplicateSlideshow(qc, show.id),
        rename: async () => {
          const name = await promptName({ title: "Rename slideshow", initial: show.name });
          if (!name) return;
          try {
            const { doc } = await invoke<SlideshowDetail>("get_slideshow", { id: show.id });
            await invoke("save_slideshow", { id: show.id, name, doc });
            await qc.invalidateQueries({ queryKey: queryKeys.slideshows });
            await qc.invalidateQueries({ queryKey: queryKeys.slideshow(show.id) });
          } catch (e) {
            fail(e, "Could not rename the slideshow.");
          }
        },
        delete: () => void remove(show.id),
      }),
    [navigate, remove, qc],
  );
}

/**
 * The sidebar's own entries: Open everywhere, plus Open in Develop for Library and Favorites
 * and Start slideshow for Favorites (a route the Favorites collection card opens).
 */
export function usePrimaryMenu() {
  const navigate = useNavigate();
  const { playScope } = useSetActions();
  return useCallback(
    (to: string): MenuItem[] => {
      const open = () => navigate(to);
      if (to === "/library") {
        return setMenu({ open, develop: () => openDevelop(navigate, { scope: LIBRARY_SCOPE, only: null, label: "Library" }) });
      }
      if (to === "/favorites") {
        const favorites = { ...LIBRARY_SCOPE, favorites: true };
        return setMenu({
          open,
          develop: () => openDevelop(navigate, { scope: favorites, only: null, label: "Favorites" }),
          slideshow: () => void playScope(favorites),
        });
      }
      return setMenu({ open });
    },
    [navigate, playScope],
  );
}

/**
 * One album's menu wherever it is shown (`/albums`, the sidebar), with the album actions that
 * need asking: a name to rename to, a cover picked from its photos, and the confirm before
 * deleting one that holds photos. Render `element` once beside the menus.
 */
export function useAlbumMenu(): { menuFor: (album: AlbumSummary, extra?: { editRules?: () => void }) => MenuItem[]; element: ReactNode } {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const { rename, updateSmart, remove, setHides, setCover } = useAlbums();
  const { moveAlbum } = useAlbumCollections();
  const [coverFor, setCoverFor] = useState<AlbumSummary | null>(null);
  const [moving, setMoving] = useState<AlbumSummary | null>(null);

  const menuFor = useCallback(
    (album: AlbumSummary, extra: { editRules?: () => void } = {}) =>
      albumMenu(album, {
        open: () => navigate(`/albums/${album.id}`),
        rename: async () => {
          const name = await promptName({ title: "Rename album", initial: album.name });
          if (!name) return;
          if (album.kind === 1 && album.query) await updateSmart({ id: album.id, name, query: album.query });
          else await rename({ id: album.id, name });
        },
        editRules: extra.editRules,
        setCover: () => setCoverFor(album),
        editCover: () => openCoverEditor({ kind: "album", id: album.id, name: album.name, style: album.coverStyle, photos: photoItems(album.previewIds) }),
        shuffleCover: () => void shuffleAlbumCover(qc, album.id),
        setHides: (hides) => void setHides({ id: album.id, hides, name: album.name }),
        duplicate: () => void duplicateAlbum(qc, album.id),
        export: () => void exportAlbum(album.id),
        moveToCollection: () => setMoving(album),
        removeFromCollection: () => void moveAlbum({ albumId: album.id, collectionId: null, albumName: album.name }),
        delete: async () => {
          if (album.itemCount > 0) {
            const ok = await confirm({
              title: `Delete “${album.name}”?`,
              body: `The album and its ordering are removed. Its ${album.itemCount} photos stay in the library.`,
              confirmLabel: "Delete album",
              danger: true,
            });
            if (!ok) return;
          }
          await remove(album.id);
        },
      }),
    [navigate, rename, updateSmart, setHides, qc, remove, moveAlbum],
  );

  const picker = moving && (
    <CollectionPicker
      title={`Move “${moving.name}” to…`}
      moving={{ album: moving.id }}
      current={moving.collectionId}
      onClose={() => setMoving(null)}
      onPick={(collectionId) => {
        const album = moving;
        setMoving(null);
        void moveAlbum({ albumId: album.id, collectionId, albumName: album.name });
      }}
    />
  );
  const cover = coverFor && (
    <PhotoPicker
      title={`Choose a cover for “${coverFor.name}”`}
      confirmLabel="Use as cover"
      scope={{ ...LIBRARY_SCOPE, albumId: coverFor.id }}
      onClose={() => setCoverFor(null)}
      onPick={(ids) => {
        const album = coverFor;
        setCoverFor(null);
        if (ids.length) void setCover({ albumId: album.id, assetId: ids[ids.length - 1] });
      }}
    />
  );
  return {
    menuFor,
    element: (
      <>
        {cover}
        {picker}
      </>
    ),
  };
}

/** A collection's menu (Round 5A), shared by its card on `/albums` and its sidebar row. */
export function useCollectionMenu(): { menuFor: (collection: AlbumCollection) => MenuItem[]; element: ReactNode } {
  const navigate = useNavigate();
  const { collections, create, rename, moveCollection, remove } = useAlbumCollections();
  const { albums } = useAlbums();
  const [moving, setMoving] = useState<AlbumCollection | null>(null);

  const menuFor = useCallback(
    (c: AlbumCollection) =>
      collectionMenu(
        { inside: c.parentId != null },
        {
          open: () => navigate(`/albums?c=${c.id}`),
          newInside: async () => {
            const name = await promptName({ title: `New collection in “${c.name}”`, initial: "" });
            if (name) await create({ name, parentId: c.id });
          },
          rename: async () => {
            const name = await promptName({ title: "Rename collection", initial: c.name });
            if (name) await rename({ id: c.id, name });
          },
          editCover: () =>
            openCoverEditor({ kind: "collection", id: c.id, name: c.name, style: c.coverStyle, photos: collectionCovers(collections, albums, c.id) }),
          move: () => setMoving(c),
          moveOut: () => void moveCollection({ id: c.id, into: null, name: c.name }),
          delete: () => void remove(c.id),
        },
      ),
    [navigate, create, rename, moveCollection, remove, collections, albums],
  );

  const element = moving && (
    <CollectionPicker
      title={`Move “${moving.name}” to…`}
      moving={{ collection: moving.id }}
      current={moving.parentId}
      onClose={() => setMoving(null)}
      onPick={(into) => {
        const c = moving;
        setMoving(null);
        void moveCollection({ id: c.id, into, name: c.name });
      }}
    />
  );
  return { menuFor, element };
}
