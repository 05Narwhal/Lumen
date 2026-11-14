// Pure geometry for rubber-band selection and arrow-key navigation. No DOM, so the
// awkward cases (a band straddling two rows, an arrow key at the end of a row) are
// unit-testable without a browser.

import type { LayoutRow } from "@/types/generated/LayoutRow";

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** A rectangle from two corners, in any order. */
export function normalizeRect(ax: number, ay: number, bx: number, by: number): Rect {
  return { x: Math.min(ax, bx), y: Math.min(ay, by), w: Math.abs(ax - bx), h: Math.abs(ay - by) };
}

/** Overlap, not containment: brushing a tile is enough to select it. Edges do not touch. */
export function intersects(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

/** Where a row sits in canvas coordinates. The grid supplies `top` from the virtualizer. */
export interface PlacedRow {
  row: LayoutRow;
  top: number;
}

const tileRect = (r: PlacedRow, t: LayoutRow["tiles"][number]): Rect => ({ x: t.x, y: r.top, w: t.w, h: t.h });

/** Ids of the tiles the band touches, in row order. */
export function idsInMarquee(rows: PlacedRow[], band: Rect): number[] {
  const ids: number[] = [];
  for (const placed of rows) {
    if (placed.row.kind === "header") continue;
    for (const t of placed.row.tiles) {
      if (intersects(band, tileRect(placed, t))) ids.push(t.id);
    }
  }
  return ids;
}

type Dir = "up" | "down" | "left" | "right";

/**
 * The tile an arrow key moves to. Left and right walk the flattened order, so they cross
 * row boundaries; up and down pick the tile whose horizontal centre is nearest in the
 * neighbouring row, which is what a justified grid makes people expect.
 */
export function neighbourId(rows: PlacedRow[], from: number, dir: Dir): number | null {
  const tileRows = rows.filter((r) => r.row.kind !== "header" && r.row.tiles.length > 0);
  let ri = -1;
  let ti = -1;
  for (let i = 0; i < tileRows.length; i++) {
    const j = tileRows[i].row.tiles.findIndex((t) => t.id === from);
    if (j >= 0) {
      ri = i;
      ti = j;
      break;
    }
  }
  if (ri < 0) return null;
  const row = tileRows[ri].row;

  if (dir === "left") {
    if (ti > 0) return row.tiles[ti - 1].id;
    const prev = tileRows[ri - 1];
    return prev ? prev.row.tiles[prev.row.tiles.length - 1].id : null;
  }
  if (dir === "right") {
    if (ti < row.tiles.length - 1) return row.tiles[ti + 1].id;
    const next = tileRows[ri + 1];
    return next ? next.row.tiles[0].id : null;
  }

  const target = tileRows[dir === "up" ? ri - 1 : ri + 1];
  if (!target) return null;
  const current = row.tiles[ti];
  const centre = current.x + current.w / 2;
  let best = target.row.tiles[0];
  let bestDistance = Number.POSITIVE_INFINITY;
  for (const t of target.row.tiles) {
    const d = Math.abs(t.x + t.w / 2 - centre);
    if (d < bestDistance) {
      bestDistance = d;
      best = t;
    }
  }
  return best.id;
}

/** How far to scroll when the pointer drags near an edge, in pixels per frame. */
export function autoScrollStep(pointerY: number, viewportHeight: number, edge = 40, speed = 12): number {
  if (pointerY < edge) return -speed;
  if (pointerY > viewportHeight - edge) return speed;
  return 0;
}
