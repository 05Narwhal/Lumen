import { describe, expect, it } from "vitest";
import type { AlbumCollection } from "@/types/generated/AlbumCollection";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";
import type { CoverStyle } from "@/types/generated/CoverStyle";
import { albumLabel, albumPath, collectionCovers, collectionPath, contentsOf, moveTargets } from "./album-collections";

const col = (id: number, name: string, parentId: number | null = null): AlbumCollection => ({ id, name, parentId, createdAt: 0, updatedAt: 0, coverStyle: null });
const album = (id: number, collectionId: number | null, coverAssetId: number | null = id * 10): AlbumSummary => ({
  id,
  name: `A${id}`,
  kind: 0,
  parentId: null,
  coverAssetId,
  itemCount: 1,
  createdAt: 0,
  updatedAt: 0,
  query: null,
  hidesItems: false,
  collectionId,
  coverStyle: null,
  previewIds: coverAssetId == null ? [] : [coverAssetId],
});

// Trips (1) ⊃ Italy (2) ⊃ Rome (3); Family (4) at the top.
const cols = [col(1, "Trips"), col(2, "Italy", 1), col(3, "Rome", 2), col(4, "Family")];
const albums = [album(10, null), album(11, 2), album(12, 3), album(13, 1), album(14, 3, null)];

describe("album collections", () => {
  it("lists what sits directly in a collection, or at the top", () => {
    const top = contentsOf(cols, albums, null);
    expect(top.collections.map((c) => c.id)).toEqual([4, 1]);
    expect(top.albums.map((a) => a.id)).toEqual([10]);
    const trips = contentsOf(cols, albums, 1);
    expect(trips.collections.map((c) => c.id)).toEqual([2]);
    expect(trips.albums.map((a) => a.id)).toEqual([13]);
  });

  it("names the way down to a collection", () => {
    expect(collectionPath(cols, 3).map((c) => c.name)).toEqual(["Trips", "Italy", "Rome"]);
    expect(collectionPath(cols, 99)).toEqual([]);
  });

  it("offers every collection but itself and its own as a place to move into", () => {
    expect(moveTargets(cols, { collection: 2 }).map((t) => t.collection.id)).toEqual([4, 1]);
    // An album can go anywhere, shown as a tree: each target with its depth.
    expect(moveTargets(cols, { album: 10 }).map((t) => [t.collection.name, t.depth])).toEqual([
      ["Family", 0],
      ["Trips", 0],
      ["Italy", 1],
      ["Rome", 2],
    ]);
  });

  it("covers a collection with the first covers of the albums anywhere inside it", () => {
    const photo = (id: number) => ({ kind: "photo", id });
    expect(collectionCovers(cols, albums, 1)).toEqual([photo(130), photo(110), photo(120)]);
    expect(collectionCovers(cols, albums, 4)).toEqual([]);
  });

  // Distinct album covers (2026-11-09): a cover is what the album's own card shows in front.
  it("covers a collection with its albums' front photos and icon covers, at most three", () => {
    const book: CoverStyle = { kind: "icon", shape: "book", icon: "heart", color: "swatch-2" };
    const box: CoverStyle = { kind: "icon", shape: "box", icon: "star", color: "swatch-4" };
    const trips = [col(1, "Trips"), { ...col(2, "Italy", 1), coverStyle: box }, col(3, "Rome", 1)];
    const inside = [
      { ...album(13, 1), coverStyle: book },
      { ...album(15, 1), previewIds: [7, 8, 9] },
      album(11, 2), // inside Italy, whose own icon stands for it
      album(16, 3),
      album(17, 3),
    ];
    expect(collectionCovers(trips, inside, 1)).toEqual([
      { kind: "icon", style: book },
      { kind: "photo", id: 7 },
      { kind: "icon", style: box },
    ]);
    // Without prints yet, the album's cover photo stands in.
    expect(collectionCovers([col(1, "Trips")], [{ ...album(18, 1), previewIds: [] }], 1)).toEqual([{ kind: "photo", id: 180 }]);
  });
});

describe("albumPath", () => {
  it("lists the collections from the top down, nothing for a loose album", () => {
    expect(albumPath(cols, album(1, 3))).toEqual(["Trips", "Italy", "Rome"]);
    expect(albumLabel(cols, album(1, 3))).toBe("A1 (Trips › Italy › Rome)");
    expect(albumPath(cols, album(2, null))).toEqual([]);
    expect(albumLabel(cols, album(2, null))).toBe("A2");
  });
});
