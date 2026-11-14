import { describe, expect, it } from "vitest";
import type { AlbumCollection } from "@/types/generated/AlbumCollection";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";
import { buildAlbumTree, countRows, visibleRows } from "./album-tree";

const col = (id: number, name: string, parentId: number | null = null) => ({ id, name, parentId }) as AlbumCollection;
const alb = (id: number, name: string, collectionId: number | null = null, kind = 0) => ({ id, name, kind, collectionId }) as AlbumSummary;

const collections = [col(1, "Trips"), col(2, "Munich", 1), col(3, "Empty"), col(4, "Smart only")];
const albums = [alb(10, "BMW", 2), alb(11, "Beer", 2), alb(12, "Loose"), alb(13, "Lisbon", 1), alb(14, "Best", 4, 1)];

describe("buildAlbumTree", () => {
  it("lists collections then albums per level, without smart albums or empty collections", () => {
    const tree = buildAlbumTree(collections, albums);
    expect(tree.map((n) => n.key)).toEqual(["c1", "a12"]);
    const trips = tree[0];
    if (trips.kind !== "collection") throw new Error("collection expected");
    expect(trips.albums).toBe(3);
    expect(trips.children.map((n) => n.key)).toEqual(["c2", "a13"]);
    expect(countRows(tree)).toBe(6);
  });

  it("walks only open collections", () => {
    const tree = buildAlbumTree(collections, albums);
    expect(visibleRows(tree, () => false).map((r) => r.node.key)).toEqual(["c1", "a12"]);
    const open = visibleRows(tree, (id) => id === 1);
    expect(open.map((r) => r.node.key)).toEqual(["c1", "c2", "a13", "a12"]);
    expect(open[1].parent).toBe("c1");
  });

  it("survives a collection that is its own ancestor", () => {
    const loop = [col(1, "A", 2), col(2, "B", 1)];
    expect(() => buildAlbumTree(loop, [alb(1, "x", 1)])).not.toThrow();
  });
});
