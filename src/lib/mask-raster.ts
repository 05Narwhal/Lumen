import type { Mask } from "@/types/generated/Mask";
import type { MaskShape } from "@/types/generated/MaskShape";
import type { Recipe } from "@/types/generated/Recipe";

/**
 * Masks in the webview (Phase 8e): the same maths as `photos-media::develop::masks`, so the
 * overlay Develop paints over the photo is exactly what Rust renders (held to
 * `mask-golden.json`, which a Rust test writes). Rust renders the pixels; this only draws the
 * red tint and maps pointer positions into the unedited photo's coordinates.
 */

/** Points are stored as 0–`MASK_GRID` of the unedited photo's sides. */
export const MASK_GRID = 4096;

/**
 * The unedited, display-oriented photo (in its pixels) mapped into a rendered frame:
 * `x' = m0·x + m1·y + m2`, `y' = m3·x + m4·y + m5`; `base` is the photo's size.
 */
export interface FrameMap {
  m: [number, number, number, number, number, number];
  base: [number, number];
}

type Affine = FrameMap["m"];

function then(f: FrameMap, a: Affine): FrameMap {
  const m = f.m;
  return {
    m: [
      a[0] * m[0] + a[1] * m[3],
      a[0] * m[1] + a[1] * m[4],
      a[0] * m[2] + a[1] * m[5] + a[2],
      a[3] * m[0] + a[4] * m[3],
      a[3] * m[1] + a[4] * m[4],
      a[3] * m[2] + a[4] * m[5] + a[5],
    ],
    base: f.base,
  };
}

export const identityMap = (w: number, h: number): FrameMap => ({ m: [1, 0, 0, 0, 1, 0], base: [w, h] });

/** `FrameMap::new`: the renderer's flips, quarter turns, straighten and crop, in its order. */
export function frameMap(w: number, h: number, r: Pick<Recipe, "flip_h" | "flip_v" | "rotate" | "straighten" | "crop">): FrameMap {
  let f = identityMap(w, h);
  if (r.flip_h) f = then(f, [-1, 0, w, 0, 1, 0]);
  if (r.flip_v) f = then(f, [1, 0, 0, 0, -1, h]);
  const turn = ((r.rotate % 360) + 360) % 360;
  if (turn === 90) {
    f = then(f, [0, -1, h, 1, 0, 0]);
    [w, h] = [h, w];
  } else if (turn === 180) f = then(f, [-1, 0, w, 0, -1, h]);
  else if (turn === 270) {
    f = then(f, [0, 1, 0, -1, 0, w]);
    [w, h] = [h, w];
  }
  if (r.straighten !== 0) {
    const t = (r.straighten * Math.PI) / 180;
    const [s, c] = [Math.sin(t), Math.cos(t)];
    const z = Math.abs(c) + Math.abs(s) * Math.max(w / h, h / w);
    const [cx, cy] = [w / 2, h / 2];
    const [a, b, cc, d] = [z * c, -z * s, z * s, z * c];
    f = then(f, [a, b, cx - a * cx - b * cy, cc, d, cy - cc * cx - d * cy]);
  }
  if (r.crop) f = then(f, [1, 0, -Math.round(r.crop.x * w), 0, 1, -Math.round(r.crop.y * h)]);
  return f;
}

/** The same frame drawn `k` times as large (a preview of a full-size frame, the screen). */
export const scaledMap = (f: FrameMap, k: number): FrameMap => then(f, [k, 0, 0, 0, k, 0]);

/** A point of the unedited photo (0–1) in frame pixels. */
export function toFrame(f: FrameMap, u: number, v: number): [number, number] {
  const [x, y] = [u * f.base[0], v * f.base[1]];
  const m = f.m;
  return [m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5]];
}

/** A frame pixel back in the unedited photo (0–1): where a pointer lands on the photo. */
export function fromFrame(f: FrameMap, p: [number, number]): [number, number] {
  const m = f.m;
  const det = m[0] * m[4] - m[1] * m[3];
  const dx = p[0] - m[2];
  const dy = p[1] - m[5];
  const x = (m[4] * dx - m[1] * dy) / det;
  const y = (-m[3] * dx + m[0] * dy) / det;
  return [x / f.base[0], y / f.base[1]];
}

/** A displacement in the unedited photo's pixels, in frame pixels. */
const vector = (f: FrameMap, dx: number, dy: number): [number, number] => [f.m[0] * dx + f.m[1] * dy, f.m[3] * dx + f.m[4] * dy];

/** Frame pixels per unedited-photo pixel. */
export const mapScale = (f: FrameMap) => Math.sqrt(Math.abs(f.m[0] * f.m[4] - f.m[1] * f.m[3]));

/** The unedited photo's long edge in frame pixels: what a brush radius is a share of. */
export const longEdge = (f: FrameMap) => Math.max(f.base[0], f.base[1]) * mapScale(f);

/** A point (0–1) on the stored 0–4096 grid. */
export const quantise = (p: [number, number]): [number, number] => [
  Math.round(Math.min(1, Math.max(0, p[0])) * MASK_GRID),
  Math.round(Math.min(1, Math.max(0, p[1])) * MASK_GRID),
];

function smoothstep(e0: number, e1: number, x: number) {
  const t = Math.min(1, Math.max(0, (x - e0) / Math.max(e1 - e0, 1e-9)));
  return t * t * (3 - 2 * t);
}

function falloff(r: number, inner: number, outer: number) {
  if (r <= inner) return 1;
  if (r >= outer) return 0;
  return 1 - smoothstep(inner, outer, r);
}

/**
 * A brush under a pixel wide, as Rust renders it (`Prepared::new`): the same disc seen through a
 * one-pixel box, so its edge widens by half a pixel each side and its flow falls with its width.
 * A 1 px line then marks its pixels at least half however it falls between pixel centres.
 */
function thinBrush(r: number, inner: number, flow: number) {
  return r < 1 ? { r: r + 0.5, inner: Math.max(0, inner - 0.5), flow: flow * Math.min(1, 2 * r) } : { r, inner, flow };
}

/** Dab centres every `r/4` along a stroke, in frame pixels. */
export function dabs(f: FrameMap, r: number, points: [number, number][]): [number, number][] {
  const spacing = Math.max(r / 4, 0.5);
  const centres = points.map((p) => toFrame(f, p[0] / MASK_GRID, p[1] / MASK_GRID));
  const out: [number, number][] = [centres[0]];
  for (let i = 1; i < centres.length; i++) {
    const [a, b] = [centres[i - 1], centres[i]];
    const steps = Math.max(1, Math.ceil(Math.hypot(b[0] - a[0], b[1] - a[1]) / spacing));
    for (let k = 1; k <= steps; k++) {
      const t = k / steps;
      out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
    }
  }
  return out;
}

/** A stored AI mask's bytes, as `photos://ai-mask/<asset>/<hash>` serves them (8-bit weights). */
export interface AiRaster {
  width: number;
  height: number;
  weights: ArrayLike<number>;
}

/** The photo's rasters by hash. A shape whose raster isn't here weighs nothing. */
export type AiRasters = Readonly<Record<string, AiRaster | undefined>>;

/** `sample_raster` in Rust: bilinear between cell centres through the map, held at the edge. */
function sampleRaster(raster: AiRaster, f: FrameMap, w: number, h: number, s: Float32Array) {
  const { width: rw, height: rh, weights } = raster;
  if (rw === 0 || rh === 0 || weights.length !== rw * rh) return;
  for (let y = 0; y < h; y++) {
    const [u0, v0] = fromFrame(f, [0, y + 0.5]);
    const [u1, v1] = fromFrame(f, [1, y + 0.5]);
    for (let x = 0; x < w; x++) {
      const t = x + 0.5;
      const gx = Math.min(rw - 1, Math.max(0, (u0 + (u1 - u0) * t) * rw - 0.5));
      const gy = Math.min(rh - 1, Math.max(0, (v0 + (v1 - v0) * t) * rh - 0.5));
      const [x0, y0] = [Math.floor(gx), Math.floor(gy)];
      const [x1, y1] = [Math.min(x0 + 1, rw - 1), Math.min(y0 + 1, rh - 1)];
      const [tx, ty] = [gx - x0, gy - y0];
      const top = weights[y0 * rw + x0] + tx * (weights[y0 * rw + x1] - weights[y0 * rw + x0]);
      const bottom = weights[y1 * rw + x0] + tx * (weights[y1 * rw + x1] - weights[y1 * rw + x0]);
      s[y * w + x] = (top + ty * (bottom - top)) / 255;
    }
  }
}

/** One shape's weights over the frame. */
function shapeWeights(shape: MaskShape, f: FrameMap, w: number, h: number, rasters?: AiRasters): Float32Array {
  const s = new Float32Array(w * h);
  if (shape.kind === "ai") {
    const raster = rasters?.[shape.raster];
    if (raster) sampleRaster(raster, f, w, h, s);
    return s;
  }
  if (shape.kind === "linear") {
    const a = toFrame(f, shape.from[0], shape.from[1]);
    const b = toFrame(f, shape.to[0], shape.to[1]);
    const d = [b[0] - a[0], b[1] - a[1]];
    const len2 = d[0] * d[0] + d[1] * d[1];
    if (len2 < 1e-9) return s;
    for (let y = 0; y < h; y++)
      for (let x = 0; x < w; x++) {
        const t = ((x + 0.5 - a[0]) * d[0] + (y + 0.5 - a[1]) * d[1]) / len2;
        s[y * w + x] = 1 - smoothstep(0, 1, t);
      }
    return s;
  }
  if (shape.kind === "radial") {
    const c = toFrame(f, shape.center[0], shape.center[1]);
    const t = (shape.angle * Math.PI) / 180;
    const [sa, ca] = [Math.sin(t), Math.cos(t)];
    const [rx, ry] = [shape.radius[0] * f.base[0], shape.radius[1] * f.base[1]];
    const e1 = vector(f, ca * rx, sa * rx);
    const e2 = vector(f, -sa * ry, ca * ry);
    const l1 = Math.max(e1[0] * e1[0] + e1[1] * e1[1], 1e-9);
    const l2 = Math.max(e2[0] * e2[0] + e2[1] * e2[1], 1e-9);
    const inner = 1 - shape.feather;
    for (let y = 0; y < h; y++)
      for (let x = 0; x < w; x++) {
        const [dx, dy] = [x + 0.5 - c[0], y + 0.5 - c[1]];
        const q1 = (dx * e1[0] + dy * e1[1]) / l1;
        const q2 = (dx * e2[0] + dy * e2[1]) / l2;
        const v = falloff(Math.sqrt(q1 * q1 + q2 * q2), inner, 1);
        s[y * w + x] = shape.inside ? v : 1 - v;
      }
    return s;
  }
  const r0 = shape.radius * longEdge(f);
  if (shape.points.length === 0 || r0 <= 0) return s;
  const { r, inner, flow } = thinBrush(r0, r0 * (1 - shape.feather), shape.flow);
  const keep = new Float32Array(w * h).fill(1);
  for (const d of dabs(f, r0, shape.points)) {
    const x0 = Math.max(0, Math.floor(d[0] - r));
    const x1 = Math.min(w, Math.ceil(d[0] + r));
    const y0 = Math.max(0, Math.floor(d[1] - r));
    const y1 = Math.min(h, Math.ceil(d[1] + r));
    for (let y = y0; y < y1; y++)
      for (let x = x0; x < x1; x++) {
        const v = falloff(Math.hypot(x + 0.5 - d[0], y + 0.5 - d[1]), inner, r);
        if (v > 0) keep[y * w + x] *= 1 - flow * v;
      }
  }
  for (let i = 0; i < s.length; i++) s[i] = 1 - keep[i];
  return s;
}

/** The shapes combined in order (add = union, subtract = × (1 − s)), before invert and opacity. */
function combined(mask: Mask, f: FrameMap, w: number, h: number, rasters?: AiRasters): Float32Array {
  const m = new Float32Array(w * h);
  for (const shape of mask.shapes) {
    const s = shapeWeights(shape, f, w, h, rasters);
    for (let i = 0; i < m.length; i++) m[i] = shape.add ? 1 - (1 - m[i]) * (1 - s[i]) : m[i] * (1 - s[i]);
  }
  return m;
}

const finish = (mask: Pick<Mask, "invert" | "opacity">, v: number) => (mask.invert ? 1 - v : v) * mask.opacity;

/**
 * `rasterize_with` in Rust without a guide: shapes in order (add = union, subtract = × (1 − s)),
 * then invert and opacity. An AI shape is its raster's own edge: only Rust refines it.
 */
export function rasterize(mask: Mask, f: FrameMap, w: number, h: number, rasters?: AiRasters): Float32Array {
  const m = combined(mask, f, w, h, rasters);
  if (mask.invert || mask.opacity < 1) for (let i = 0; i < m.length; i++) m[i] = finish(mask, m[i]);
  return m;
}

/**
 * A brush stroke being painted (Phase 8e): the mask's weights with the stroke appended, kept
 * up to date dab by dab as points arrive, so the overlay never re-rasterises the whole mask
 * per pointer move. `dirty` is the box the last `extend` changed (pixels, end exclusive).
 */
export class StrokeTint {
  readonly weights: Float32Array;
  dirty: { x0: number; y0: number; x1: number; y1: number } | null = null;
  private readonly before: Float32Array;
  private readonly keep: Float32Array;
  private stamped: [number, number][] = [];
  private readonly r: number;
  private readonly r0: number;
  private readonly inner: number;
  private readonly flow: number;

  constructor(
    private readonly mask: Mask,
    private readonly f: FrameMap,
    private readonly w: number,
    private readonly h: number,
    private readonly brush: { radius: number; feather: number; flow: number; add: boolean },
    rasters?: AiRasters,
  ) {
    this.before = combined(mask, f, w, h, rasters);
    this.keep = new Float32Array(w * h).fill(1);
    this.weights = new Float32Array(w * h);
    for (let i = 0; i < this.weights.length; i++) this.weights[i] = finish(mask, this.before[i]);
    this.r0 = brush.radius * longEdge(f);
    ({ r: this.r, inner: this.inner, flow: this.flow } = thinBrush(this.r0, this.r0 * (1 - brush.feather), brush.flow));
  }

  /** The stroke is now `points` (0–4096). A longer stroke stamps only its new dabs. */
  extend(points: [number, number][]): void {
    const prefix = this.stamped.length > 0 && this.stamped.length <= points.length && this.stamped.every((p, i) => p[0] === points[i][0] && p[1] === points[i][1]);
    let fresh: [number, number][];
    if (prefix) {
      // `dabs` of the segments not yet stamped, without the first point again.
      const from = this.stamped.length - 1;
      fresh = dabs(this.f, this.r0, points.slice(from)).slice(1);
    } else {
      // A straight (Shift) stroke replaced its points: start the stroke again.
      this.keep.fill(1);
      fresh = points.length ? dabs(this.f, this.r0, points) : [];
    }
    this.stamped = points.map((p) => [p[0], p[1]] as [number, number]);
    this.dirty = prefix ? null : { x0: 0, y0: 0, x1: this.w, y1: this.h };
    const { w, h, r, inner } = this;
    for (const d of fresh) {
      const x0 = Math.max(0, Math.floor(d[0] - r));
      const x1 = Math.min(w, Math.ceil(d[0] + r));
      const y0 = Math.max(0, Math.floor(d[1] - r));
      const y1 = Math.min(h, Math.ceil(d[1] + r));
      if (x0 >= x1 || y0 >= y1) continue;
      for (let y = y0; y < y1; y++)
        for (let x = x0; x < x1; x++) {
          const v = falloff(Math.hypot(x + 0.5 - d[0], y + 0.5 - d[1]), inner, r);
          if (v > 0) this.keep[y * w + x] *= 1 - this.flow * v;
        }
      const b = this.dirty;
      this.dirty = b ? { x0: Math.min(b.x0, x0), y0: Math.min(b.y0, y0), x1: Math.max(b.x1, x1), y1: Math.max(b.y1, y1) } : { x0, y0, x1, y1 };
    }
    const b = this.dirty;
    if (!b) return;
    for (let y = b.y0; y < b.y1; y++)
      for (let x = b.x0; x < b.x1; x++) {
        const i = y * w + x;
        const s = 1 - this.keep[i];
        const m = this.before[i];
        this.weights[i] = finish(this.mask, this.brush.add ? 1 - (1 - m) * (1 - s) : m * (1 - s));
      }
  }
}
