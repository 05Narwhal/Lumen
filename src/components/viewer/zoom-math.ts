// Pure zoom/pan maths for the viewer, kept DOM-free so it is unit-testable.

export interface Transform {
  scale: number;
  x: number;
  y: number;
}

export interface Size {
  width: number;
  height: number;
}

export const MIN_SCALE = 1;

/** Largest useful zoom: at least 4× fit, or twice true 1:1 for very large originals. */
export function maxScale(oneToOne: number): number {
  return Math.max(4, oneToOne * 2);
}

/** Scale at which one image pixel is one screen pixel. */
export function oneToOneScale(natural: Size | null, fitted: Size): number {
  if (!natural || fitted.width <= 0) return 2;
  return Math.max(MIN_SCALE, natural.width / fitted.width);
}

export function clampScale(scale: number, oneToOne: number): number {
  return Math.min(maxScale(oneToOne), Math.max(MIN_SCALE, scale));
}

/**
 * Keep the image inside the box: an axis where the scaled image is smaller than the box
 * is centred (offset 0); otherwise the offset is limited so no edge leaves the box.
 * `fitted` is the image's rendered size at scale 1, centred in `box`.
 */
export function clampPan(t: Transform, box: Size, fitted: Size): Transform {
  const w = fitted.width * t.scale;
  const h = fitted.height * t.scale;
  const maxX = Math.max(0, (w - box.width) / 2);
  const maxY = Math.max(0, (h - box.height) / 2);
  return { scale: t.scale, x: Math.min(maxX, Math.max(-maxX, t.x)), y: Math.min(maxY, Math.max(-maxY, t.y)) };
}

/** Zoom by `factor` around a point `(cx, cy)` given relative to the box centre, then clamp. */
export function zoomAround(t: Transform, factor: number, cx: number, cy: number, box: Size, fitted: Size, oneToOne: number): Transform {
  const scale = clampScale(t.scale * factor, oneToOne);
  const k = scale / t.scale;
  const next = { scale, x: cx - (cx - t.x) * k, y: cy - (cy - t.y) * k };
  return scale === MIN_SCALE ? { scale: MIN_SCALE, x: 0, y: 0 } : clampPan(next, box, fitted);
}
