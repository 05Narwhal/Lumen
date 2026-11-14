/**
 * The Albums page's search (Round 19 §9): the top search box filters albums and collections
 * by name while the person types. Case- and accent-insensitive; every word typed must start a
 * word of the name (`ital 24` finds *Italy 2024*).
 */
import { albumPath } from "@/lib/album-collections";
import type { AlbumCollection } from "@/types/generated/AlbumCollection";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";

/** Lower case, accents stripped, `-` `_` `.` as spaces: the form names and queries are compared in. */
export function foldName(s: string): string {
  return s
    .normalize("NFD")
    .replace(/\p{M}+/gu, "")
    .toLowerCase()
    .replace(/[-_.'’]+/g, " ")
    .trim();
}

/** True when every word of `query` is the start of a word of `name`. An empty query matches nothing. */
export function matchesName(query: string, name: string): boolean {
  const words = foldName(query).split(/\s+/).filter(Boolean);
  if (words.length === 0) return false;
  const have = foldName(name).split(/\s+/);
  return words.every((w) => have.some((h) => h.startsWith(w)));
}

export type AlbumHit =
  | { kind: "collection"; collection: AlbumCollection; path: string[] }
  | { kind: "album"; album: AlbumSummary; path: string[] };

/** Collections first, then albums; each group keeps the order it came in. `path` is where the hit sits. */
export function searchAlbums(query: string, collections: AlbumCollection[], albums: AlbumSummary[]): AlbumHit[] {
  const byId = new Map(collections.map((c) => [c.id, c]));
  const hits: AlbumHit[] = [];
  for (const collection of collections) {
    if (!matchesName(query, collection.name)) continue;
    const parent = collection.parentId == null ? undefined : byId.get(collection.parentId);
    hits.push({ kind: "collection", collection, path: parent ? albumPath(collections, { collectionId: parent.id }) : [] });
  }
  for (const album of albums) {
    if (matchesName(query, album.name)) hits.push({ kind: "album", album, path: albumPath(collections, album) });
  }
  return hits;
}
