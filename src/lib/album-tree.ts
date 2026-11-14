/**
 * The collection tree of *Add to album* (Round 20 §5): collections then albums per level in the
 * Albums page's order, smart albums left out (nothing can be added to them), and a collection
 * holding no ordinary album left out too. Pure, so the picker's keys walk the same rows it draws.
 */
import { contentsOf } from "@/lib/album-collections";
import type { AlbumCollection } from "@/types/generated/AlbumCollection";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";

export type AlbumTreeNode =
  | { key: string; kind: "collection"; collection: AlbumCollection; albums: number; children: AlbumTreeNode[] }
  | { key: string; kind: "album"; album: AlbumSummary };

export const collectionKey = (id: number) => `c${id}`;
export const albumKey = (id: number) => `a${id}`;

export function buildAlbumTree(collections: AlbumCollection[], albums: AlbumSummary[], parent: number | null = null, seen = new Set<number>()): AlbumTreeNode[] {
  const { collections: cs, albums: as } = contentsOf(collections, albums.filter((a) => a.kind === 0), parent);
  const nodes: AlbumTreeNode[] = [];
  for (const collection of cs) {
    if (seen.has(collection.id)) continue; // a cycle in bad data must not hang the picker
    const children = buildAlbumTree(collections, albums, collection.id, new Set(seen).add(collection.id));
    if (children.length === 0) continue;
    const inside = children.reduce((n, c) => n + (c.kind === "album" ? 1 : c.albums), 0);
    nodes.push({ key: collectionKey(collection.id), kind: "collection", collection, albums: inside, children });
  }
  for (const album of as) nodes.push({ key: albumKey(album.id), kind: "album", album });
  return nodes;
}

/** Every row of the tree, open or not, counted: the picker starts fully open when this is small. */
export function countRows(nodes: AlbumTreeNode[]): number {
  return nodes.reduce((n, node) => n + 1 + (node.kind === "collection" ? countRows(node.children) : 0), 0);
}

/** The rows on screen in reading order, each with the key of the collection it sits in. */
export function visibleRows(nodes: AlbumTreeNode[], isOpen: (collectionId: number) => boolean, parent: string | null = null): { node: AlbumTreeNode; parent: string | null }[] {
  const out: { node: AlbumTreeNode; parent: string | null }[] = [];
  for (const node of nodes) {
    out.push({ node, parent });
    if (node.kind === "collection" && isOpen(node.collection.id)) out.push(...visibleRows(node.children, isOpen, node.key));
  }
  return out;
}
