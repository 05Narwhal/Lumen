// Actions shared by a context menu and its palette entry (Round 4G): the same function,
// whichever way the user asked. Each fails softly, with a toast, when its object has gone
// (an album deleted elsewhere, a memory regenerated away).

import type { QueryClient } from "@tanstack/react-query";
import { openCollectionPicker } from "@/components/collection-picker";
import { openCoverEditor } from "@/components/cover-editor";
import { photoItems } from "@/components/cover-stack";
import { promptName } from "@/components/confirm-dialog";
import { openExportSheet } from "@/components/export-sheet";
import { toast } from "@/components/toast";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { askExportFolder } from "@/lib/export-folder";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import { collectionCovers } from "@/lib/album-collections";
import type { AlbumCollection } from "@/types/generated/AlbumCollection";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";
import type { LayoutScope } from "@/types/generated/LayoutScope";
import type { MemorySummary } from "@/types/generated/MemorySummary";
import type { SlideshowDetail } from "@/types/generated/SlideshowDetail";

const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });

/** The id in `/<section>/<id>` when that is the page open now, for palette entries. */
export function openId(section: string, path = window.location.pathname): number | null {
  const m = new RegExp(`^/${section}/(\\d+)(?:/|$)`).exec(path);
  return m ? Number(m[1]) : null;
}

/** The connected device open now (`/connected/{kind}/{id}`), for palette entries. */
export function openDevice(path = window.location.pathname): { kind: string; id: string } | null {
  const m = /^\/connected\/(volume|iphone|mac)\/([^/]+)/.exec(path);
  return m ? { kind: m[1], id: decodeURIComponent(m[2]) } : null;
}

/** *Eject* on the card open now. */
export async function ejectVolume(mount: string) {
  try {
    await invoke("eject_volume", { mount });
    toast({ message: "It can be removed safely" });
  } catch (e) {
    fail(e, "The drive could not be ejected.");
  }
}

/** Every photo a scope selects, or a toast when there are none. */
export async function scopeIds(scope: LayoutScope): Promise<number[]> {
  const ids = await invoke<number[]>("scope_ids", { scope });
  if (ids.length === 0) toast({ message: "There are no photos in it." });
  return ids;
}

export async function exportScope(scope: LayoutScope) {
  try {
    const ids = await scopeIds(scope);
    if (ids.length) openExportSheet(ids);
  } catch (e) {
    fail(e, "Could not export these photos.");
  }
}

/** A slideshow as a video (Round 6: every export asks where to save). From the editor, a
 *  slideshow row's menu and its ⋯ button. */
export async function exportSlideshow(id: number) {
  const destDir = await askExportFolder("Export slideshow to…");
  if (destDir === null) return;
  await invoke("export_slideshow", { id, destDir }).catch((e: unknown) => toast({ message: isAppError(e) ? e.message : "Could not start the export." }));
}

export const exportAlbum = (id: number) => exportScope({ ...LIBRARY_SCOPE, albumId: id });

export async function duplicateAlbum(qc: QueryClient, id: number) {
  const refresh = () => qc.invalidateQueries({ queryKey: queryKeys.albums });
  try {
    const copy = await invoke<AlbumSummary>("duplicate_album", { id });
    await refresh();
    toast({ message: `Made “${copy.name}”`, undo: () => void invoke("delete_album", { id: copy.id }).then(refresh) });
  } catch (e) {
    fail(e, "Could not duplicate the album.");
  }
}

/** The collection open on `/albums?c=<id>` now, for palette entries (Round 5A). */
export function openCollection(location: { pathname: string; search: string } = window.location): number | null {
  if (location.pathname !== "/albums") return null;
  const c = new URLSearchParams(location.search).get("c");
  return c ? Number(c) : null;
}

/** *New collection…*: inside the collection open now, else at the top. */
export async function newCollection(qc: QueryClient, parentId: number | null = openCollection()) {
  const name = await promptName({ title: "New collection", initial: "", confirmLabel: "Create" });
  if (!name) return;
  try {
    await invoke<number>("create_album_collection", { name, parentId });
    await qc.invalidateQueries({ queryKey: queryKeys.albums });
  } catch (e) {
    fail(e, "Could not create the collection.");
  }
}

/** *Move this album to collection…* for the album open now. */
export async function moveAlbumToCollection(id: number) {
  try {
    const album = await invoke<AlbumSummary>("get_album", { id });
    openCollectionPicker({ id: album.id, name: album.name, collectionId: album.collectionId });
  } catch (e) {
    fail(e, "That album is gone.");
  }
}

/** *Change this album's cover…* (Julian's 2026-11-08 list) for the album open now. */
export async function editAlbumCover(id: number) {
  try {
    const album = await invoke<AlbumSummary>("get_album", { id });
    openCoverEditor({ kind: "album", id, name: album.name, style: album.coverStyle, photos: photoItems(album.previewIds) });
  } catch (e) {
    fail(e, "That album is gone.");
  }
}

/** *Shuffle cover* (distinct album covers, 2026-11-09): new automatic prints for an album. */
export async function shuffleAlbumCover(qc: QueryClient, id: number) {
  try {
    await invoke<void>("shuffle_album_cover", { id });
    await qc.invalidateQueries({ queryKey: queryKeys.albums });
  } catch (e) {
    fail(e, "That album is gone.");
  }
}

/** *Change this collection's cover…* for the collection open now. */
export async function editCollectionCover(id: number) {
  try {
    const [collections, albums] = await Promise.all([invoke<AlbumCollection[]>("list_album_collections"), invoke<AlbumSummary[]>("list_albums")]);
    const c = collections.find((k) => k.id === id);
    if (!c) return fail(null, "That collection is gone.");
    openCoverEditor({ kind: "collection", id, name: c.name, style: c.coverStyle, photos: collectionCovers(collections, albums, id) });
  } catch (e) {
    fail(e, "That collection is gone.");
  }
}

/** *Back up now* (Round 5A): the open library, to its backup destination. */
export async function backUpNow(qc: QueryClient) {
  try {
    await invoke<void>("start_backup");
    toast({ message: "Backing up…" });
    await qc.invalidateQueries({ queryKey: queryKeys.backup });
  } catch (e) {
    fail(e, "Could not start the backup.");
  }
}

/** Stops the running backup; the next one picks up where it stopped. */
export async function stopBackup(qc: QueryClient) {
  try {
    await invoke<void>("cancel_backup");
    await qc.invalidateQueries({ queryKey: queryKeys.backup });
  } catch (e) {
    fail(e, "Could not stop the backup.");
  }
}

export async function duplicateSlideshow(qc: QueryClient, id: number) {
  const refresh = () => qc.invalidateQueries({ queryKey: queryKeys.slideshows });
  try {
    const { name, doc } = await invoke<SlideshowDetail>("get_slideshow", { id });
    const copyName = `${name} copy`;
    const assetIds = doc.clips.flatMap((c) => [c.asset_id, ...c.extra]);
    const copy = await invoke<number>("create_slideshow", { name: copyName, assetIds, arrange: false });
    await invoke("save_slideshow", { id: copy, name: copyName, doc });
    await refresh();
    toast({ message: `Made “${copyName}”`, undo: () => void invoke("delete_slideshow", { id: copy }).then(refresh) });
  } catch (e) {
    fail(e, "Could not duplicate the slideshow.");
  }
}

/** Hiding a memory is reversible, so it never asks: it toasts with Undo. */
export async function setMemoryHidden(qc: QueryClient, m: Pick<MemorySummary, "id">, hidden: boolean) {
  const set = async (value: boolean) => {
    await invoke("set_memory_hidden", { id: m.id, hidden: value });
    await Promise.all([qc.invalidateQueries({ queryKey: ["memories"] }), qc.invalidateQueries({ queryKey: queryKeys.memory(m.id) })]);
  };
  try {
    await set(hidden);
    toast({ message: hidden ? "Memory hidden" : "Memory shown again", undo: () => void set(!hidden) });
  } catch (e) {
    fail(e, "This memory is gone.");
  }
}
