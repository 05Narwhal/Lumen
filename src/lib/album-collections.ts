/**
 * Collections (Round 5A): folders of albums. An album is in at most one collection
 * (`AlbumSummary.collectionId`); collections nest (`AlbumCollection.parentId`). Pure helpers
 * over the two lists, so every screen draws the same tree.
 */
import type { CoverItem } from "@/components/cover-stack";
import type { AlbumCollection } from "@/types/generated/AlbumCollection";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";

const byName = (a: AlbumCollection, b: AlbumCollection) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }) || a.id - b.id;

/** The collections and albums directly inside `parent` (`null`: the top). Collections by name; albums keep their order. */
export function contentsOf(collections: AlbumCollection[], albums: AlbumSummary[], parent: number | null) {
  return {
    collections: collections.filter((c) => c.parentId === parent).sort(byName),
    albums: albums.filter((a) => (a.collectionId ?? null) === parent),
  };
}

/** From the top down to `id`, for a breadcrumb. Empty when there is no such collection. */
export function collectionPath(collections: AlbumCollection[], id: number): AlbumCollection[] {
  const byId = new Map(collections.map((c) => [c.id, c]));
  const path: AlbumCollection[] = [];
  for (let at = byId.get(id); at && !path.includes(at); at = at.parentId == null ? undefined : byId.get(at.parentId)) path.unshift(at);
  return path;
}

/** `id` and every collection inside it, at any depth. */
function withDescendants(collections: AlbumCollection[], id: number): Set<number> {
  const out = new Set([id]);
  for (let grew = true; grew; ) {
    grew = false;
    for (const c of collections) {
      if (c.parentId != null && out.has(c.parentId) && !out.has(c.id)) {
        out.add(c.id);
        grew = true;
      }
    }
  }
  return out;
}

/**
 * Where something may move, as a tree in reading order with each collection's depth. A
 * collection never moves into itself or one of its own.
 */
export function moveTargets(collections: AlbumCollection[], moving: { album: number } | { collection: number }) {
  const banned = "collection" in moving ? withDescendants(collections, moving.collection) : new Set<number>();
  const out: { collection: AlbumCollection; depth: number }[] = [];
  const walk = (parent: number | null, depth: number) => {
    for (const c of collections.filter((k) => k.parentId === parent).sort(byName)) {
      if (banned.has(c.id)) continue;
      out.push({ collection: c, depth });
      walk(c.id, depth + 1);
    }
  };
  walk(null, 0);
  return out;
}

/**
 * Up to `n` covers for a collection's card: its own albums first, then those deeper in. Each is
 * what that album's card shows in front — its icon cover, else its front print — and a
 * collection with an icon cover of its own counts once, as that icon.
 */
export function collectionCovers(collections: AlbumCollection[], albums: AlbumSummary[], id: number, n = 3): CoverItem[] {
  const out: CoverItem[] = [];
  const visit = (at: number) => {
    for (const a of albums) {
      if ((a.collectionId ?? null) !== at || out.length >= n) continue;
      const front = a.previewIds[0] ?? a.coverAssetId;
      if (a.coverStyle) out.push({ kind: "icon", style: a.coverStyle });
      else if (front != null) out.push({ kind: "photo", id: front });
    }
    for (const c of collections.filter((k) => k.parentId === at).sort(byName)) {
      if (out.length >= n) break;
      if (c.coverStyle) out.push({ kind: "icon", style: c.coverStyle });
      else visit(c.id);
    }
  };
  visit(id);
  return out;
}

/** How many albums a collection holds, at any depth. */
export function albumCount(collections: AlbumCollection[], albums: AlbumSummary[], id: number): number {
  const inside = withDescendants(collections, id);
  return albums.filter((a) => a.collectionId != null && inside.has(a.collectionId)).length;
}

/** The collections an album sits in, from the top down (`["Trips", "Europe"]`); empty for a loose album. */
export function albumPath(collections: AlbumCollection[], album: Pick<AlbumSummary, "collectionId">): string[] {
  return album.collectionId == null ? [] : collectionPath(collections, album.collectionId).map((c) => c.name);
}

/** Plain-text form for menus and labels: `Italy 2024 (Trips › Europe)`; just the name when loose. */
export function albumLabel(collections: AlbumCollection[], album: Pick<AlbumSummary, "name" | "collectionId">): string {
  const path = albumPath(collections, album);
  return path.length ? `${album.name} (${path.join(" › ")})` : album.name;
}
