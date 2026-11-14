import type { Recipe } from "@/types/generated/Recipe";

/** A rectangle in 0..1 of a picture: left, top, width, height. */
export type Rect = [number, number, number, number];
type Point = [number, number];

/**
 * Faces are stored in 0..1 of the **unedited** photo; the viewer shows the edited one. These
 * map between the two through the renderer's geometry (`photos-media::develop::geometry`):
 * flips, quarter turns clockwise, straighten (rotated about the centre and zoomed so no
 * corner leaves the photo), then the crop. Points are carried in a frame `aspect` wide and 1
 * high, so straightening a non-square photo turns it the way the renderer does.
 */
interface Frame {
  w: number;
  h: number;
}

function straightenZoom(deg: number, f: Frame): number {
  const t = (deg * Math.PI) / 180;
  return Math.abs(Math.cos(t)) + Math.abs(Math.sin(t)) * Math.max(f.w / f.h, f.h / f.w);
}

const turns = (r: Recipe) => (((r.rotate % 360) + 360) % 360) as 0 | 90 | 180 | 270;

/** Unedited 0..1 → shown 0..1. */
export function toShown(p: Point, r: Recipe, aspect: number): Point {
  let f: Frame = { w: aspect, h: 1 };
  let [x, y] = [p[0] * f.w, p[1] * f.h];
  if (r.flip_h) x = f.w - x;
  if (r.flip_v) y = f.h - y;
  switch (turns(r)) {
    case 90:
      [x, y] = [f.h - y, x];
      f = { w: f.h, h: f.w };
      break;
    case 180:
      [x, y] = [f.w - x, f.h - y];
      break;
    case 270:
      [x, y] = [y, f.w - x];
      f = { w: f.h, h: f.w };
      break;
  }
  if (r.straighten) {
    // The renderer samples source = R(output − c) / z + c; this is its inverse.
    const t = (r.straighten * Math.PI) / 180;
    const [s, c] = [Math.sin(t), Math.cos(t)];
    const z = straightenZoom(r.straighten, f);
    const [dx, dy] = [x - f.w / 2, y - f.h / 2];
    [x, y] = [(c * dx - s * dy) * z + f.w / 2, (s * dx + c * dy) * z + f.h / 2];
  }
  let [u, v] = [x / f.w, y / f.h];
  if (r.crop) [u, v] = [(u - r.crop.x) / r.crop.w, (v - r.crop.y) / r.crop.h];
  return [u, v];
}

/** Shown 0..1 → unedited 0..1. */
export function toOriginal(p: Point, r: Recipe, aspect: number): Point {
  const t90 = turns(r) === 90 || turns(r) === 270;
  const f: Frame = t90 ? { w: 1, h: aspect } : { w: aspect, h: 1 };
  let [u, v] = p;
  if (r.crop) [u, v] = [r.crop.x + u * r.crop.w, r.crop.y + v * r.crop.h];
  let [x, y] = [u * f.w, v * f.h];
  if (r.straighten) {
    const t = (r.straighten * Math.PI) / 180;
    const [s, c] = [Math.sin(t), Math.cos(t)];
    const z = straightenZoom(r.straighten, f);
    const [dx, dy] = [(x - f.w / 2) / z, (y - f.h / 2) / z];
    [x, y] = [c * dx + s * dy + f.w / 2, -s * dx + c * dy + f.h / 2];
  }
  // Back through the turn into the flipped, unturned frame (`aspect` wide).
  const [W, H] = [aspect, 1];
  switch (turns(r)) {
    case 90:
      [x, y] = [y, H - x];
      break;
    case 180:
      [x, y] = [W - x, H - y];
      break;
    case 270:
      [x, y] = [W - y, x];
      break;
  }
  if (r.flip_h) x = W - x;
  if (r.flip_v) y = H - y;
  return [x / W, y / H];
}

function bounds(points: Point[]): Rect {
  const xs = points.map((p) => p[0]);
  const ys = points.map((p) => p[1]);
  const [x0, y0] = [Math.min(...xs), Math.min(...ys)];
  return [x0, y0, Math.max(...xs) - x0, Math.max(...ys) - y0];
}

const corners = ([x, y, w, h]: Rect): Point[] => [
  [x, y],
  [x + w, y],
  [x, y + h],
  [x + w, y + h],
];

/** A stored face's box on the shown photo (the box around its corners). */
export function rectToShown(rect: Rect, r: Recipe | null | undefined, aspect: number): Rect {
  return r ? bounds(corners(rect).map((p) => toShown(p, r, aspect))) : rect;
}

/** A box drawn on the shown photo, in the unedited photo's 0..1, clamped to it. */
export function rectToOriginal(rect: Rect, r: Recipe | null | undefined, aspect: number): Rect {
  const [x, y, w, h] = r ? bounds(corners(rect).map((p) => toOriginal(p, r, aspect))) : rect;
  const [x0, y0] = [Math.max(0, x), Math.max(0, y)];
  return [x0, y0, Math.min(1, x + w) - x0, Math.min(1, y + h) - y0];
}

/** Whether any of a box is on the shown photo (a face cropped away is not drawn). */
export function visible([x, y, w, h]: Rect): boolean {
  return x < 1 && y < 1 && x + w > 0 && y + h > 0;
}
