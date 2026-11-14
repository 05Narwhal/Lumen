import { afterEach, describe, expect, it } from "vitest";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";
import { readRecentAlbumIds, recentAlbums, rememberAlbum } from "./album-recent";

const album = (id: number, kind = 0) => ({ id, name: `A${id}`, kind }) as AlbumSummary;

afterEach(() => localStorage.clear());

describe("recentAlbums", () => {
  it("keeps the order of the ids, three at most, skipping the gone and the smart", () => {
    const albums = [album(1), album(2), album(3, 1), album(4), album(5), album(6)];
    expect(recentAlbums([9, 3, 5, 5, 1, 4, 2], albums).map((a) => a.id)).toEqual([5, 1, 4]);
    expect(recentAlbums([], albums)).toEqual([]);
  });
});

describe("rememberAlbum", () => {
  it("puts the album first once and keeps a bounded list", () => {
    for (let i = 1; i <= 12; i++) rememberAlbum(i);
    rememberAlbum(5);
    const ids = readRecentAlbumIds();
    expect(ids[0]).toBe(5);
    expect(ids.filter((x) => x === 5)).toHaveLength(1);
    expect(ids).toHaveLength(10);
  });
});
