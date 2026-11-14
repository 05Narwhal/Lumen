import type { CurvePoint } from "@/lib/tone-curve";

/** Closest two points may come, horizontally. */
export const MIN_GAP = 0.01;
/** At most this many points, as Rust keeps (`curve_points`). */
export const MAX_POINTS = 16;

const clamp01 = (v: number) => Math.min(1, Math.max(0, v));

/** The points as edited: sorted, with (0, 0) and (1, 1) added when the ends are missing. */
export function withEnds(points: CurvePoint[]): CurvePoint[] {
  const p: CurvePoint[] = [...points].sort((a, b) => a[0] - b[0]);
  if (p.length === 0 || p[0][0] > 1e-4) p.unshift([0, 0]);
  if (p[p.length - 1][0] < 1 - 1e-4) p.push([1, 1]);
  return p;
}

/** An end point sits on x = 0 or x = 1 and only moves up and down. */
export const isEnd = (points: CurvePoint[], i: number) => i === 0 || i === points.length - 1;

/** Add a point at (x, y), unless it would crowd a neighbour. Returns the points and its index. */
export function addPoint(points: CurvePoint[], x: number, y: number): { points: CurvePoint[]; index: number } | null {
  const p = withEnds(points);
  if (p.length >= MAX_POINTS) return null;
  const nx = clamp01(x);
  if (p.some((q) => Math.abs(q[0] - nx) < MIN_GAP)) return null;
  const added: CurvePoint = [nx, clamp01(y)];
  const next = [...p, added].sort((a, b) => a[0] - b[0]);
  return { points: next, index: next.findIndex((q) => q[0] === nx) };
}

/** Move point `i`: never past its neighbours (so the order holds), ends only vertically. */
export function movePoint(points: CurvePoint[], i: number, x: number, y: number): CurvePoint[] {
  const p = points.map((q) => [...q] as CurvePoint);
  const nx = isEnd(p, i) ? p[i][0] : Math.min(p[i + 1][0] - MIN_GAP, Math.max(p[i - 1][0] + MIN_GAP, x));
  p[i] = [nx, clamp01(y)];
  return p;
}

/** Remove point `i`; the ends stay. */
export function removePoint(points: CurvePoint[], i: number): CurvePoint[] {
  return isEnd(points, i) ? points : points.filter((_, j) => j !== i);
}

/** A drag this far outside the box takes the point off. */
export const isDraggedOff = (x: number, y: number) => x < -0.08 || x > 1.08 || y < -0.12 || y > 1.12;
