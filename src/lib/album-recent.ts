/**
 * The albums something was last added to (Round 20 §5), newest first, kept per viewer in
 * `localStorage`. `useAlbums().addTo` writes it, so the picker, the sidebar drop and pick mode
 * all count.
 */
import type { AlbumSummary } from "@/types/generated/AlbumSummary";

export const RECENT_ALBUMS_KEY = "lumen.albums.recent";
/** More than the picker shows, so ids of albums that were deleted or went smart do not starve it. */
const KEPT = 10;

export function readRecentAlbumIds(): number[] {
  try {
    const v = JSON.parse(localStorage.getItem(RECENT_ALBUMS_KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((x): x is number => typeof x === "number") : [];
  } catch {
    return [];
  }
}

/** Puts `id` first. A refusing storage costs only the memory. */
export function rememberAlbum(id: number) {
  const next = [id, ...readRecentAlbumIds().filter((x) => x !== id)].slice(0, KEPT);
  try {
    localStorage.setItem(RECENT_ALBUMS_KEY, JSON.stringify(next));
  } catch {
    // Private windows refuse.
  }
}

/** Up to `n` of `ids` (newest first) that are still ordinary albums; smart albums take no items. */
export function recentAlbums(ids: number[], albums: AlbumSummary[], n = 3): AlbumSummary[] {
  const byId = new Map(albums.map((a) => [a.id, a]));
  const out: AlbumSummary[] = [];
  for (const id of ids) {
    const a = byId.get(id);
    if (a && a.kind === 0 && !out.includes(a)) out.push(a);
    if (out.length === n) break;
  }
  return out;
}
