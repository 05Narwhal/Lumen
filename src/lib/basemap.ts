// The offline base map: Natural Earth land, lakes and country borders, plus shaded relief,
// all bundled (built by scripts/build-map-data.py). No tile server, no network, no key.
//
// Geometry arrives already in Web Mercator, as integer fractions of the world, so drawing
// is one multiply-add per vertex. Vertices are placed in screen space in double precision
// rather than through a canvas transform: a Path2D stores float32, which drifts by whole
// pixels once the world is millions of pixels wide.

import world110 from "@/assets/map/world-110m.json";
import lightUrl from "@/assets/map/relief-light.webp";
import shadeUrl from "@/assets/map/relief-shade.webp";
import { lonToX, latToY, worldSize, type Viewport } from "./mercator";

export interface Ring {
  /** Absolute world fractions, `[x0, y0, x1, y1, …]`, 0..1 on both axes. */
  points: Float64Array;
  /** `[minX, minY, maxX, maxY]` in world fractions, for culling. */
  bounds: [number, number, number, number];
}

export interface BaseLayers {
  land: Ring[];
  lakes: Ring[];
  borders: Ring[];
}

interface Encoded {
  q: number;
  land: number[][];
  lakes: number[][];
  borders: number[][];
}

/** Delta-encoded integer rings (see the build script) to absolute fractions with bounds. */
export function decodeLayer(rings: number[][], q: number): Ring[] {
  return rings.map((enc) => {
    const points = new Float64Array(enc.length);
    let x = 0;
    let y = 0;
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (let i = 0; i < enc.length; i += 2) {
      x = i === 0 ? enc[0] : x + enc[i];
      y = i === 0 ? enc[1] : y + enc[i + 1];
      const fx = x / q;
      const fy = y / q;
      points[i] = fx;
      points[i + 1] = fy;
      if (fx < minX) minX = fx;
      if (fx > maxX) maxX = fx;
      if (fy < minY) minY = fy;
      if (fy > maxY) maxY = fy;
    }
    return { points, bounds: [minX, minY, maxX, maxY] };
  });
}

const decodeAll = (e: Encoded): BaseLayers => ({
  land: decodeLayer(e.land, e.q),
  lakes: decodeLayer(e.lakes, e.q),
  borders: decodeLayer(e.borders, e.q),
});

/** Screen position of world fraction `f` is `f * scale + (x, y)`. */
export function worldToScreen(v: Viewport): { scale: number; x: number; y: number } {
  const scale = worldSize(v.zoom);
  return { scale, x: v.width / 2 - lonToX(v.lon) * scale, y: v.height / 2 - latToY(v.lat) * scale };
}

/** Which whole-world offsets are visible: more than one near the antimeridian or zoomed out. */
export function worldCopies(v: Viewport): number[] {
  const t = worldToScreen(v);
  const first = Math.floor(-t.x / t.scale);
  const last = Math.ceil((v.width - t.x) / t.scale) - 1;
  const out: number[] = [];
  for (let k = first; k <= last; k++) out.push(k);
  return out;
}

// ---- Data, loaded on demand ----------------------------------------------------------

/** Below this zoom the 110 m set is sharper than the screen; above it, 50 m. */
const FINE_ZOOM = 3;

/**
 * The relief is about 20 km a pixel: crisp at world and continent scale, a smudge at city
 * scale. It fades out between these zooms rather than blurring into dirt.
 */
const RELIEF_FULL_UNTIL = 4;
const RELIEF_GONE_AT = 7;

export function reliefFade(zoom: number): number {
  return Math.max(0, Math.min(1, (RELIEF_GONE_AT - zoom) / (RELIEF_GONE_AT - RELIEF_FULL_UNTIL)));
}

const coarse = decodeAll(world110 as Encoded);
let fine: BaseLayers | null = null;
let fineRequested = false;
let relief: { shade: HTMLImageElement; light: HTMLImageElement } | null = null;
let reliefReady = 0;

let version = 0;
const listeners = new Set<() => void>();
const changed = () => {
  version++;
  for (const l of listeners) l();
};

/** For `useSyncExternalStore`: redraw when a detail level or the relief finishes loading. */
export const subscribeBaseMap = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};
export const baseMapVersion = () => version;

function layersFor(zoom: number): BaseLayers {
  if (zoom < FINE_ZOOM) return coarse;
  if (!fineRequested) {
    fineRequested = true;
    void import("@/assets/map/world-50m.json").then((m) => {
      fine = decodeAll(m.default as Encoded);
      changed();
    });
  }
  return fine ?? coarse;
}

function reliefImages() {
  if (!relief) {
    const load = (src: string) => {
      const img = new Image();
      img.decoding = "async";
      img.onload = () => {
        reliefReady++;
        if (reliefReady === 2) changed();
      };
      img.src = src;
      return img;
    };
    relief = { shade: load(shadeUrl), light: load(lightUrl) };
  }
  return reliefReady === 2 ? relief : null;
}

// ---- Drawing -------------------------------------------------------------------------

/** Resolve a colour or number token on the canvas, which cannot read CSS variables itself. */
function token(el: HTMLElement, name: string, fallback: string): string {
  return getComputedStyle(el).getPropertyValue(name).trim() || fallback;
}

/**
 * Add every ring that reaches the viewport to `path`, once per visible world copy.
 * Vertices closer than about half a pixel to the previous one are skipped, so a
 * zoomed-out view of the 50 m coastline costs what it looks like, not what it contains.
 */
function trace(path: Path2D, rings: Ring[], v: Viewport, copies: number[], close: boolean) {
  const t = worldToScreen(v);
  const s = t.scale;
  const pad = 2;
  for (const k of copies) {
    const ox = t.x + k * s;
    for (const r of rings) {
      const [bx0, by0, bx1, by1] = r.bounds;
      if (bx1 * s + ox < -pad || bx0 * s + ox > v.width + pad) continue;
      if (by1 * s + t.y < -pad || by0 * s + t.y > v.height + pad) continue;
      const p = r.points;
      let lx = p[0] * s + ox;
      let ly = p[1] * s + t.y;
      path.moveTo(lx, ly);
      const n = p.length;
      for (let i = 2; i < n; i += 2) {
        const x = p[i] * s + ox;
        const y = p[i + 1] * s + t.y;
        if (i < n - 2 && Math.abs(x - lx) + Math.abs(y - ly) < 0.6) continue;
        path.lineTo(x, y);
        lx = x;
        ly = y;
      }
      if (close) path.closePath();
    }
  }
}

/**
 * Both relief masks, tinted with the theme's colours at the theme's strengths and
 * composited into one layer. Built once per theme rather than per frame: re-tinting at
 * canvas size on every pan step cost more than drawing all the vectors together.
 */
let tinted: { key: string; layer: HTMLCanvasElement } | null = null;

function reliefLayer(images: { shade: HTMLImageElement; light: HTMLImageElement }, masks: [string, number][]): HTMLCanvasElement | null {
  const key = JSON.stringify(masks);
  if (tinted?.key === key) return tinted.layer;
  const n = images.shade.naturalWidth;
  const layer = tinted?.layer ?? document.createElement("canvas");
  layer.width = n;
  layer.height = n;
  const lc = layer.getContext("2d");
  const tmp = document.createElement("canvas");
  tmp.width = n;
  tmp.height = n;
  const tc = tmp.getContext("2d");
  if (!lc || !tc) return null;
  for (const [img, [colour, alpha]] of [images.shade, images.light].map((img, i) => [img, masks[i]] as const)) {
    if (alpha <= 0) continue;
    // Colour lands only where the mask has coverage ("source-in"), then joins the layer.
    tc.globalCompositeOperation = "source-over";
    tc.clearRect(0, 0, n, n);
    tc.drawImage(img, 0, 0);
    tc.globalCompositeOperation = "source-in";
    tc.fillStyle = colour;
    tc.fillRect(0, 0, n, n);
    lc.globalAlpha = alpha;
    lc.drawImage(tmp, 0, 0);
  }
  lc.globalAlpha = 1;
  tinted = { key, layer };
  return layer;
}

/** Draw the part of `layer` (a whole-world square) that the viewport shows. */
function paintWorldImage(ctx: CanvasRenderingContext2D, layer: HTMLCanvasElement, v: Viewport, copies: number[]) {
  const t = worldToScreen(v);
  const f = layer.width / t.scale;
  for (const k of copies) {
    // Only the visible slice: a destination the size of the whole world would be
    // millions of pixels wide when zoomed in.
    const wx = t.x + k * t.scale;
    const x0 = Math.max(0, wx);
    const y0 = Math.max(0, t.y);
    const x1 = Math.min(v.width, wx + t.scale);
    const y1 = Math.min(v.height, t.y + t.scale);
    if (x1 <= x0 || y1 <= y0) continue;
    ctx.drawImage(layer, (x0 - wx) * f, (y0 - t.y) * f, (x1 - x0) * f, (y1 - y0) * f, x0, y0, x1 - x0, y1 - y0);
  }
}

/** Paint the world into a canvas at `v`. Shared by the map and the viewer's mini-map. */
export function drawBaseMap(canvas: HTMLCanvasElement, v: Viewport, opts: { relief?: boolean } = {}) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const dpr = window.devicePixelRatio || 1;
  const w = Math.max(1, Math.round(v.width * dpr));
  const h = Math.max(1, Math.round(v.height * dpr));
  // Resizing reallocates the backing store, so only when the size really changed; a pan
  // step repaints everything below anyway.
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w;
    canvas.height = h;
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

  const water = token(canvas, "--map-water", "#0e0e0e");
  ctx.fillStyle = water;
  ctx.fillRect(0, 0, v.width, v.height);

  const layers = layersFor(v.zoom);
  const copies = worldCopies(v);
  const land = new Path2D();
  trace(land, layers.land, v, copies, true);
  ctx.fillStyle = token(canvas, "--map-land", "#1a1a1a");
  ctx.fill(land, "evenodd");

  const fade = opts.relief === false ? 0 : reliefFade(v.zoom);
  const images = fade > 0 ? reliefImages() : null;
  if (images) {
    const strength = (name: string) => Number.parseFloat(token(canvas, name, "0")) || 0;
    const layer = reliefLayer(images, [
      [token(canvas, "--map-shade", "#000"), strength("--map-shade-strength")],
      [token(canvas, "--map-light", "#fff"), strength("--map-light-strength")],
    ]);
    // No clip to the land: the masks are already empty over open sea (flat ground in the
    // source falls inside the build script's dead zone), lakes are painted over them
    // next, and clipping to the 50 m coastline tripled the cost of a frame.
    if (layer) {
      ctx.globalAlpha = fade;
      paintWorldImage(ctx, layer, v, copies);
      ctx.globalAlpha = 1;
    }
  }

  const lakes = new Path2D();
  trace(lakes, layers.lakes, v, copies, true);
  ctx.fillStyle = water;
  ctx.fill(lakes, "evenodd");

  // Borders under the coastline, thinner and quieter: they divide the land, the coast
  // gives it its shape.
  const borders = new Path2D();
  trace(borders, layers.borders, v, copies, false);
  ctx.lineJoin = "round";
  ctx.lineCap = "round";
  ctx.strokeStyle = token(canvas, "--map-border", "#555");
  ctx.lineWidth = v.zoom < FINE_ZOOM ? 0.6 : 0.9;
  ctx.stroke(borders);

  ctx.strokeStyle = token(canvas, "--map-coast", "#666");
  ctx.lineWidth = 1;
  ctx.stroke(land);
  ctx.stroke(lakes);
}
