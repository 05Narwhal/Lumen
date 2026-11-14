// The crop rectangle, in units of the frame (0..1 on each axis). Pixel aspects (width /
// height) are converted with the frame's own aspect: a normalised square is only a square
// in pixels when the frame is.

export type Rect = { x: number; y: number; w: number; h: number };
export type Handle = "n" | "s" | "e" | "w" | "ne" | "nw" | "se" | "sw" | "move";

const MIN = 0.05;
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

export const ASPECTS: { label: string; value: number | null | "original" }[] = [
  { label: "Free", value: null },
  { label: "Original", value: "original" },
  { label: "1:1", value: 1 },
  { label: "4:3", value: 4 / 3 },
  { label: "3:2", value: 3 / 2 },
  { label: "16:9", value: 16 / 9 },
];

/** The largest rectangle of pixel `aspect` centred in `r`. */
export function fitAspect(r: Rect, aspect: number, frameAspect: number): Rect {
  const nAspect = aspect / frameAspect;
  let w = r.w;
  let h = w / nAspect;
  if (h > r.h) {
    h = r.h;
    w = h * nAspect;
  }
  return { x: r.x + (r.w - w) / 2, y: r.y + (r.h - h) / 2, w, h };
}

/**
 * `s` after dragging `handle` by `dx`, `dy` (frame units). Never leaves the frame, never
 * smaller than 5%; with an `aspect` the corner opposite the handle stays put and the
 * rectangle shrinks to fit it.
 */
export function dragCrop(s: Rect, handle: Handle, dx: number, dy: number, aspect: number | null, frameAspect: number): Rect {
  if (handle === "move") {
    return { ...s, x: clamp(s.x + dx, 0, 1 - s.w), y: clamp(s.y + dy, 0, 1 - s.h) };
  }
  let left = s.x;
  let top = s.y;
  let right = s.x + s.w;
  let bottom = s.y + s.h;
  if (handle.includes("w")) left = clamp(left + dx, 0, right - MIN);
  if (handle.includes("e")) right = clamp(right + dx, left + MIN, 1);
  if (handle.includes("n")) top = clamp(top + dy, 0, bottom - MIN);
  if (handle.includes("s")) bottom = clamp(bottom + dy, top + MIN, 1);
  let r = { x: left, y: top, w: right - left, h: bottom - top };
  if (aspect) {
    const f = fitAspect(r, aspect, frameAspect);
    const x = handle.includes("w") ? right - f.w : left;
    const y = handle.includes("n") ? bottom - f.h : top;
    r = { x, y, w: f.w, h: f.h };
  }
  return r;
}
