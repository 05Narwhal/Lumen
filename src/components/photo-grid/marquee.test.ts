import { describe, expect, it } from "vitest";
import type { LayoutRow } from "@/types/generated/LayoutRow";
import { autoScrollStep, idsInMarquee, intersects, neighbourId, normalizeRect, type PlacedRow } from "./marquee";

const tile = (id: number, x: number, w: number, h = 100) => ({
  id,
  x,
  w,
  h,
  thumbhash: null,
  color: null,
  mediaType: 0,
  durationMs: null,
  renderable: true,
  extension: "jpg",
  isFavorite: false,
  trashedAt: null,
  rating: 0,
  hdr: 0,
  raw: false,
  colorLabel: 0,
  flag: 0,
});

const row = (index: number, tiles: ReturnType<typeof tile>[]): LayoutRow => ({
  index,
  kind: "tiles",
  y: 0,
  height: 100,
  section: 0,
  title: null,
  tiles,
});

const header = (index: number): LayoutRow => ({ index, kind: "header", y: 0, height: 44, section: 0, title: "September 2026", tiles: [] });

// Two rows of three, with a month header above them.
const rows: PlacedRow[] = [
  { row: header(0), top: 0 },
  { row: row(1, [tile(1, 0, 100), tile(2, 110, 200), tile(3, 320, 100)]), top: 44 },
  { row: row(2, [tile(4, 0, 300), tile(5, 310, 100)]), top: 148 },
];

describe("marquee geometry", () => {
  it("normalises a rectangle from any two corners", () => {
    expect(normalizeRect(10, 20, 40, 60)).toEqual({ x: 10, y: 20, w: 30, h: 40 });
    expect(normalizeRect(40, 60, 10, 20)).toEqual({ x: 10, y: 20, w: 30, h: 40 });
    expect(normalizeRect(5, 5, 5, 5)).toEqual({ x: 5, y: 5, w: 0, h: 0 });
  });

  it("treats touching edges as no overlap", () => {
    const a = { x: 0, y: 0, w: 10, h: 10 };
    expect(intersects(a, { x: 5, y: 5, w: 10, h: 10 })).toBe(true);
    expect(intersects(a, { x: 10, y: 0, w: 10, h: 10 })).toBe(false);
    expect(intersects(a, { x: -20, y: 0, w: 10, h: 10 })).toBe(false);
  });

  it("selects every tile the band brushes, across rows and past headers", () => {
    // A thin band down the left edge crosses the first tile of both rows.
    expect(idsInMarquee(rows, { x: 5, y: 50, w: 10, h: 200 })).toEqual([1, 4]);
    // A wide band over the first row only.
    expect(idsInMarquee(rows, { x: 0, y: 44, w: 500, h: 100 })).toEqual([1, 2, 3]);
    // Everything.
    expect(idsInMarquee(rows, { x: 0, y: 0, w: 500, h: 400 })).toEqual([1, 2, 3, 4, 5]);
    // A band in the header's band alone selects nothing.
    expect(idsInMarquee(rows, { x: 0, y: 0, w: 500, h: 40 })).toEqual([]);
    // A zero-size band selects nothing.
    expect(idsInMarquee(rows, { x: 100, y: 100, w: 0, h: 0 })).toEqual([]);
  });

  it("moves left and right through the flattened order", () => {
    expect(neighbourId(rows, 1, "right")).toBe(2);
    expect(neighbourId(rows, 3, "right")).toBe(4); // wraps into the next row
    expect(neighbourId(rows, 4, "left")).toBe(3);
    expect(neighbourId(rows, 1, "left")).toBeNull();
    expect(neighbourId(rows, 5, "right")).toBeNull();
    expect(neighbourId(rows, 99, "right")).toBeNull();
  });

  it("moves up and down to the nearest tile by horizontal centre", () => {
    // Tile 3 spans 320–420 (centre 370); in the next row tile 5 (310–410, centre 360) is nearest.
    expect(neighbourId(rows, 3, "down")).toBe(5);
    // Tile 1 (centre 50) drops onto tile 4 (centre 150) rather than tile 5.
    expect(neighbourId(rows, 1, "down")).toBe(4);
    // Tile 4 (centre 150) rises to tile 2 (110–310, centre 210) — nearer than tile 1 at 50.
    expect(neighbourId(rows, 4, "up")).toBe(2);
    expect(neighbourId(rows, 1, "up")).toBeNull(); // the header row is not a target
    expect(neighbourId(rows, 5, "down")).toBeNull();
  });

  it("auto-scrolls only near the edges", () => {
    expect(autoScrollStep(10, 800)).toBeLessThan(0);
    expect(autoScrollStep(790, 800)).toBeGreaterThan(0);
    expect(autoScrollStep(400, 800)).toBe(0);
  });
});
