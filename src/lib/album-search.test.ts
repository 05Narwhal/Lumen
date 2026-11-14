import { describe, expect, it } from "vitest";
import { foldName, matchesName, searchAlbums } from "./album-search";
import type { AlbumCollection } from "@/types/generated/AlbumCollection";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";

const col = (id: number, name: string, parentId: number | null = null) => ({ id, name, parentId }) as AlbumCollection;
const alb = (id: number, name: string, collectionId: number | null = null) => ({ id, name, collectionId }) as AlbumSummary;

describe("matchesName", () => {
  it("ignores case and accents", () => {
    expect(matchesName("zurich", "Zürich 2024")).toBe(true);
    expect(matchesName("ÉTÉ", "Fête de l'été")).toBe(true);
  });
  it("matches word prefixes, not the middle of a word", () => {
    expect(matchesName("ital", "Italy 2024")).toBe(true);
    expect(matchesName("taly", "Italy 2024")).toBe(false);
    expect(matchesName("ital 20", "Italy 2024")).toBe(true);
    expect(matchesName("2024 italy", "Italy 2024")).toBe(true);
    expect(matchesName("ital 2023", "Italy 2024")).toBe(false);
  });
  it("treats dots and dashes as spaces and an empty query as nothing", () => {
    expect(matchesName("6.7.", "6.7. Hike")).toBe(true);
    expect(matchesName("hike", "Day-hike")).toBe(true);
    expect(matchesName("  ", "Anything")).toBe(false);
  });
  it("folds", () => expect(foldName("  Ça-va ")).toBe("ca va"));
});

describe("searchAlbums", () => {
  const collections = [col(1, "Trips"), col(2, "Europe", 1), col(3, "Italy notes", 2)];
  const albums = [alb(10, "Italy 2024", 2), alb(11, "Rome", null), alb(12, "Italian food", 3)];
  it("lists collections first, each with the path it sits in", () => {
    const hits = searchAlbums("ital", collections, albums);
    expect(hits.map((h) => (h.kind === "album" ? `a${h.album.id}` : `c${h.collection.id}`))).toEqual(["c3", "a10", "a12"]);
    expect(hits.map((h) => h.path)).toEqual([["Trips", "Europe"], ["Trips", "Europe"], ["Trips", "Europe", "Italy notes"]]);
  });
  it("a top-level collection and a loose album have no path", () => {
    const hits = searchAlbums("trips rome", collections, albums);
    expect(hits).toEqual([]);
    expect(searchAlbums("rome", collections, albums)[0].path).toEqual([]);
    expect(searchAlbums("trips", collections, albums)[0].path).toEqual([]);
  });
});
