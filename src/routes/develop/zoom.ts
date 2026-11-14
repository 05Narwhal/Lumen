// Develop's zoom (Round 4J): pure maths, so the canvas only wires events to it.
//
// A level is photo pixels per *device* pixel (100 % = one to one, as in every photo editor);
// `scaleOf` turns a state into CSS pixels per photo pixel for layout. A pan is where the
// view's top-left corner sits on the photo, in photo pixels; negative when the photo is
// smaller than the view and centred in it.

export type ZoomState = { mode: "fit" } | { mode: "fill" } | { mode: "level"; level: number };
export type Size = { w: number; h: number };
export type Pan = { x: number; y: number };

/** The levels the zoom menu and `Ctrl+=` / `Ctrl+−` step through. */
export const LEVELS = [0.25, 0.5, 1, 2, 4];
export const MIN_LEVEL = 0.05;
export const MAX_LEVEL = 4;

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** CSS pixels per photo pixel. */
export function scaleOf(z: ZoomState, photo: Size, view: Size, dpr = 1): number {
  if (photo.w <= 0 || photo.h <= 0) return 1;
  if (z.mode === "fit") return Math.min(view.w / photo.w, view.h / photo.h);
  if (z.mode === "fill") return Math.max(view.w / photo.w, view.h / photo.h);
  return z.level / dpr;
}

/** The level a state shows at: device pixels per photo pixel. */
export const levelOf = (z: ZoomState, photo: Size, view: Size, dpr = 1) => (z.mode === "level" ? z.level : scaleOf(z, photo, view, dpr) * dpr);

/** Keeps the view inside the photo, centring the photo along an axis it doesn't fill. */
export function clampPan(pan: Pan, z: ZoomState, photo: Size, view: Size, dpr = 1): Pan {
  const s = scaleOf(z, photo, view, dpr);
  const axis = (v: number, p: number, span: number) => (p <= span ? (p - span) / 2 : clamp(v, 0, p - span));
  return { x: axis(pan.x, photo.w, view.w / s), y: axis(pan.y, photo.h, view.h / s) };
}

/**
 * Zooms by `factor` around `anchor` (CSS pixels in the view): the photo point under it stays
 * under it. The result is always a level, between 5 % and 400 %.
 */
export function zoomAround(z: ZoomState, factor: number, anchor: Pan, pan: Pan, photo: Size, view: Size, dpr = 1): { zoom: ZoomState; pan: Pan } {
  const from = clampPan(pan, z, photo, view, dpr);
  const s0 = scaleOf(z, photo, view, dpr);
  const level = clamp(levelOf(z, photo, view, dpr) * factor, MIN_LEVEL, MAX_LEVEL);
  const zoom: ZoomState = { mode: "level", level };
  const s1 = level / dpr;
  const px = from.x + anchor.x / s0;
  const py = from.y + anchor.y / s0;
  return { zoom, pan: clampPan({ x: px - anchor.x / s1, y: py - anchor.y / s1 }, zoom, photo, view, dpr) };
}

/** Keeps the view's centre on the same photo point across a change of zoom. */
export function panAroundCentre(from: ZoomState, to: ZoomState, pan: Pan, photo: Size, view: Size, dpr = 1): Pan {
  const p = clampPan(pan, from, photo, view, dpr);
  const s0 = scaleOf(from, photo, view, dpr);
  const s1 = scaleOf(to, photo, view, dpr);
  const cx = p.x + view.w / 2 / s0;
  const cy = p.y + view.h / 2 / s0;
  return clampPan({ x: cx - view.w / 2 / s1, y: cy - view.h / 2 / s1 }, to, photo, view, dpr);
}

/** The next level up (`dir` 1) or down (−1): fit, then the `LEVELS` above fit's scale. */
export function nextLevel(z: ZoomState, dir: 1 | -1, photo: Size, view: Size, dpr = 1): ZoomState {
  const fit = levelOf({ mode: "fit" }, photo, view, dpr);
  const now = z.mode === "fit" ? fit : levelOf(z, photo, view, dpr);
  const steps = LEVELS.filter((l) => l > fit + 1e-9);
  if (dir > 0) {
    const up = steps.find((l) => l > now + 1e-9);
    return up !== undefined ? { mode: "level", level: up } : z.mode === "level" ? z : { mode: "level", level: MAX_LEVEL };
  }
  const down = [...steps].reverse().find((l) => l < now - 1e-9);
  return down !== undefined ? { mode: "level", level: down } : { mode: "fit" };
}

/**
 * The zoom factor for a wheel delta (pixels). Exponential, so a touchpad's many small deltas
 * multiply to the same place as one large one; a mouse notch (100 px) is about 22 %.
 */
export const wheelFactor = (deltaY: number) => Math.exp(-deltaY * 0.002);

/**
 * Where the pixels for `level` come from: the preview while its base has enough of them for
 * the photo's long edge, the full-resolution region above that.
 */
export function sourceFor(level: number, baseLongEdge: number, photoLongEdge: number): "preview" | "region" {
  // A base as large as the photo already holds every pixel the region could add.
  if (baseLongEdge >= photoLongEdge) return "preview";
  return photoLongEdge * level <= baseLongEdge + 0.5 ? "preview" : "region";
}

/** The visible part of the photo as 0–1 fractions, for `develop_region`. */
export function visibleRect(pan: Pan, z: ZoomState, photo: Size, view: Size, dpr = 1) {
  const s = scaleOf(z, photo, view, dpr);
  const x = Math.max(0, pan.x);
  const y = Math.max(0, pan.y);
  return { x: x / photo.w, y: y / photo.h, w: Math.min(view.w / s, photo.w - x) / photo.w, h: Math.min(view.h / s, photo.h - y) / photo.h };
}

/** A level as the zoom bar shows it. */
export const percent = (level: number) => `${Math.round(level * 100)} %`;
