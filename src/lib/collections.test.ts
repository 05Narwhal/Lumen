import { describe, expect, it } from "vitest";
import { EMPTY_QUERY } from "@/hooks/use-search";
import type { AssetQuery } from "@/types/generated/AssetQuery";
import { collectionFor, collectionForTags, collectionLink, KIND_LABELS, queryToTags } from "./collections";

const raw: AssetQuery = { ...EMPTY_QUERY, kind: "raw" };

describe("collections", () => {
  it("links to the Library with the collection as a search tag (Round 7)", () => {
    const url = new URL(collectionLink(raw), "http://x");
    expect(url.pathname).toBe("/library");
    expect(JSON.parse(url.searchParams.get("tags")!)).toEqual([{ label: "RAW", value: { type: "kind", kind: "raw" } }]);
    const videos = new URL(collectionLink({ ...EMPTY_QUERY, mediaType: 1 }), "http://x");
    expect(JSON.parse(videos.searchParams.get("tags")!)).toEqual([{ label: "Videos", value: { type: "media", media: 1 } }]);
    // A query tags cannot say keeps its filters.
    expect(collectionLink({ ...EMPTY_QUERY, personIds: [3] })).toContain("filters=");
  });

  it("names the collection a page's tags are", () => {
    expect(collectionForTags([{ label: "RAW", value: { type: "kind", kind: "raw" } }])?.label).toBe("RAW");
    expect(collectionForTags([{ label: "Favorites", value: { type: "favorites" } }])?.label).toBe("Favorites");
    expect(collectionForTags([{ label: "x", value: { type: "kind", kind: "raw" } }, { label: "Favorites", value: { type: "favorites" } }])).toBeUndefined();
    expect(queryToTags({ minRating: 4, favorites: true })?.map((t) => t.label)).toEqual(["Favorites", "4 stars and up"]);
  });

  it("names the collection a query is, and nothing else", () => {
    expect(collectionFor(raw)?.label).toBe("RAW");
    expect(collectionFor({ ...EMPTY_QUERY, favorites: true })?.label).toBe("Favorites");
    expect(collectionFor({ ...raw, minRating: 3 })).toBeUndefined();
    expect(collectionFor(EMPTY_QUERY)).toBeUndefined();
  });

  it("has a label for every kind", () => {
    for (const k of ["live", "raw", "hdr", "screenshot", "panorama", "selfie"] as const) expect(KIND_LABELS[k]).toBeTruthy();
  });
});
