/**
 * The slideshow document's timing, framing and motion, in TypeScript so the editor can
 * preview it.
 *
 * `photos_core::slideshow` owns these rules for the export; a Rust test writes its answers
 * to `slideshow-golden.json` and `slideshow-timing.test.ts` holds this file to them, so the
 * preview and the MP4 cannot drift apart without a failing test.
 */

import type { FrameKind } from "@/types/generated/FrameKind";
import type { Motion } from "@/types/generated/Motion";
import type { Placement } from "@/types/generated/Placement";
import type { Rect } from "@/types/generated/Rect";
import type { SlideshowDoc } from "@/types/generated/SlideshowDoc";
import type { TransitionKind } from "@/types/generated/TransitionKind";
import type { Timeline } from "@/types/generated/Timeline";

/** A gutter between grid cells, as a fraction of the frame's width. */
export const GRID_GUTTER = 0.015;

/** When each clip starts, how far it overlaps the next, and how long the show is. */
export function timeline(doc: SlideshowDoc): Timeline {
  const overlaps = doc.clips.map((c, i) => {
    const next = doc.clips[i + 1];
    if (!next || c.transition.kind === "none") return 0;
    return Math.min(c.transition.duration_ms, Math.floor(c.duration_ms / 2), Math.floor(next.duration_ms / 2));
  });
  const starts: number[] = [];
  let t = 0;
  for (const [i, clip] of doc.clips.entries()) {
    starts.push(t);
    t += clip.duration_ms - overlaps[i];
  }
  const last = doc.clips.length - 1;
  return { starts, overlaps, totalMs: last < 0 ? 0 : starts[last] + doc.clips[last].duration_ms };
}

/** Smoothstep: motion starts and ends at rest, so a clip never begins with a jolt. */
export function ease(u: number): number {
  const c = Math.min(1, Math.max(0, u));
  return c * c * (3 - 2 * c);
}

/** `[x, y, scale]` at clip-local progress `u` (0 at the clip's start, 1 at its end). */
export function motionAt(motion: Motion, u: number): [number, number, number] {
  if (motion.kind === "none") return [0, 0, 1];
  const e = ease(u);
  return [0, 1, 2].map((k) => motion.from[k] + (motion.to[k] - motion.from[k]) * e) as [number, number, number];
}

const sane = (a: number) => (Number.isFinite(a) && a > 0 ? a : 1);

/** A rectangle of shape `aspect` centred in a box, either covering it or fitting inside. */
function placed(aspect: number, w: number, h: number, x: number, y: number, cover: boolean): Rect {
  const boxAspect = w / h;
  const [rw, rh] = aspect > boxAspect === cover ? [h * aspect, h] : [w, w / aspect];
  return { x: x + (w - rw) / 2, y: y + (h - rh) / 2, w: rw, h: rh };
}

/**
 * What a clip paints, in order — the twin of `photos_core::slideshow::frame_layout`, and
 * checked against it by the golden file. Rectangles are fractions of the frame and may
 * exceed it, which means the frame edge crops them.
 */
export function frameLayout(kind: FrameKind, aspects: number[], frameAspect: number): Placement[] {
  const frame = sane(frameAspect);
  const first = sane(aspects[0] ?? 1);
  const cover = (a: number) => placed(a / frame, 1, 1, 0, 0, true);
  const fit = (a: number) => placed(a / frame, 1, 1, 0, 0, false);
  if (kind === "fill") return [{ photo: 0, rect: cover(first), blurred: false }];
  if (kind === "blur") {
    return [
      { photo: 0, rect: cover(first), blurred: true },
      { photo: 0, rect: fit(first), blurred: false },
    ];
  }
  // Every layout that shows more than one photograph is the same idea: a blurred copy of
  // the first covering the frame, then each photograph fitted whole inside its own box.
  const cells = (boxes: [number, number, number, number][]): Placement[] => {
    const out: Placement[] = [{ photo: 0, rect: cover(first), blurred: true }];
    boxes.slice(0, aspects.length).forEach(([x, y, w, h], i) => {
      out.push({ photo: i, rect: placed(sane(aspects[i]) / frame, w, h, x, y, false), blurred: false });
    });
    return out;
  };
  const half = (1 - GRID_GUTTER) / 2;
  const far = half + GRID_GUTTER;

  if (kind === "quad") {
    return cells([
      [0, 0, half, half],
      [far, 0, half, half],
      [0, far, half, half],
      [far, far, half, half],
    ]);
  }
  if (kind === "feature") {
    const side = 1 - FEATURE_WIDTH - GRID_GUTTER;
    const x = FEATURE_WIDTH + GRID_GUTTER;
    return cells([
      [0, 0, FEATURE_WIDTH, 1],
      [x, 0, side, half],
      [x, far, side, half],
    ]);
  }
  if (kind === "stack") {
    return cells([
      [0, 0, 1, half],
      [0, far, 1, half],
    ]);
  }
  const n = Math.min(Math.max(aspects.length, 1), 3);
  const column = (1 - GRID_GUTTER * (n - 1)) / n;
  const boxes: [number, number, number, number][] = [];
  for (let i = 0; i < n; i += 1) boxes.push([i * (column + GRID_GUTTER), 0, column, 1]);
  return cells(boxes);
}

/** How a layer is masked during a wipe or a circle. */
export type LayerClip =
  | { kind: "rect"; x: number; y: number; w: number; h: number }
  | { kind: "circle"; r: number }
  /** `count` equal bands along `axis`, each filled from its own edge by `p`. */
  | { kind: "stripes"; axis: "x" | "y"; count: number; p: number }
  /** The layer drawn squashed about the frame's centre. */
  | { kind: "scale"; sx: number; sy: number };

/** How many bands a slice transition arrives in. Rust's `hlslice` uses the same count. */
export const SLICE_COUNT = 8;

/** How wide a feature layout's hero is; Rust's FEATURE_WIDTH. */
const FEATURE_WIDTH = 0.62;

/**
 * Every transition, in the order the editor lists them — the twin of Rust's
 * `TransitionKind::ALL`, so the two lists cannot drift apart.
 */
export const TRANSITION_KINDS: TransitionKind[] = [
  "none",
  "crossfade",
  "fadeblack",
  "fadewhite",
  "slide",
  "slideup",
  "slidedown",
  "cover",
  "reveal",
  "wipe",
  "wipeup",
  "corner",
  "vertopen",
  "horzopen",
  "circle",
  "circleclose",
  "squeeze",
  "slices",
  "vslices",
  "zoom",
];

/** The transitions whose effect is on the outgoing picture, so it is painted last. */
const OUTGOING_ON_TOP = new Set<TransitionKind>(["reveal", "circleclose", "squeeze", "corner"]);

/**
 * One picture of the show at time `t`. `x`, `y`, `offsetX` and `offsetY` are fractions of
 * the frame; `scale` is the zoom about its centre, transition included. Layers are drawn
 * in order, so the last is on top.
 */
export type Layer = {
  index: number;
  /** Every photo the clip shows; `placements` indexes into this. */
  assets: number[];
  placements: Placement[];
  /** Clip-local progress, for anything that wants the motion itself. */
  u: number;
  alpha: number;
  x: number;
  y: number;
  scale: number;
  /** Sideways travel during a slide, in frame widths. */
  offsetX: number;
  /** Upward travel during a vertical slide, in frame heights. */
  offsetY: number;
  clip?: LayerClip;
};

const assetsOf = (clip: SlideshowDoc["clips"][number]) => [clip.asset_id, ...clip.extra];

function layerFor(doc: SlideshowDoc, index: number, t: number, starts: number[], aspectOf: (assetId: number) => number): Layer {
  const clip = doc.clips[index];
  const u = clip.duration_ms > 0 ? Math.min(1, Math.max(0, (t - starts[index]) / clip.duration_ms)) : 0;
  const [x, y, scale] = motionAt(clip.motion, u);
  const assets = assetsOf(clip);
  const frameAspect = doc.resolution[0] / Math.max(1, doc.resolution[1]);
  return {
    index,
    assets,
    placements: frameLayout(clip.frame, assets.map(aspectOf), frameAspect),
    u,
    alpha: 1,
    x,
    y,
    scale,
    offsetX: 0,
    offsetY: 0,
  };
}

/**
 * Which clips are visible at `t` milliseconds and how each is drawn. `aspectOf` gives a
 * photo's width / height; without it every photo is assumed to match the frame, which is
 * right for `fill` and harmless before the pictures have loaded.
 */
export function frameAt(
  doc: SlideshowDoc,
  t: number,
  aspectOf: (assetId: number) => number = () => doc.resolution[0] / Math.max(1, doc.resolution[1]),
): { layers: Layer[]; backdrop?: "black" | "white" } {
  if (doc.clips.length === 0) return { layers: [] };
  const { starts, overlaps, totalMs } = timeline(doc);
  const at = Math.min(totalMs, Math.max(0, t));

  // The transition from clip i runs from clip i+1's start for `overlaps[i]`.
  const crossing = overlaps.findIndex((o, i) => o > 0 && at >= starts[i + 1] && at < starts[i + 1] + o);
  if (crossing === -1) {
    let index = 0;
    for (let i = 0; i < doc.clips.length; i += 1) if (at >= starts[i]) index = i;
    return { layers: [layerFor(doc, index, at, starts, aspectOf)] };
  }

  const p = (at - starts[crossing + 1]) / overlaps[crossing];
  const out = layerFor(doc, crossing, at, starts, aspectOf);
  const into = layerFor(doc, crossing + 1, at, starts, aspectOf);
  let backdrop: "black" | "white" | undefined;
  switch (doc.clips[crossing].transition.kind) {
    case "slide":
      out.offsetX = -p;
      into.offsetX = 1 - p;
      break;
    case "slideup":
      out.offsetY = -p;
      into.offsetY = 1 - p;
      break;
    case "wipe":
      into.clip = { kind: "rect", x: 1 - p, y: 0, w: p, h: 1 };
      break;
    case "wipeup":
      // The edge travels upward, so the new picture grows from the bottom.
      into.clip = { kind: "rect", x: 0, y: 1 - p, w: 1, h: p };
      break;
    case "circle":
      into.clip = { kind: "circle", r: (p * Math.hypot(1, 1)) / 2 };
      break;
    case "fadeblack":
    case "fadewhite":
      // Out to the colour by the midpoint, then in from it.
      backdrop = doc.clips[crossing].transition.kind === "fadeblack" ? "black" : "white";
      out.alpha = Math.max(0, 1 - 2 * p);
      into.alpha = Math.max(0, 2 * p - 1);
      break;
    case "zoom":
      // FFmpeg's `zoomin`, measured frame by frame: the outgoing picture is zoomed into
      // over the first half of the join and is past the frame by about p = 0.4, and the
      // new picture only then arrives, on a smoothstep over the second half. The scale
      // runs away at p = 0.5, so the denominator is floored to keep it drawable.
      out.scale *= 1 / Math.max(0.04, 1 - ease(Math.min(1, p / 0.5)));
      into.alpha = ease(Math.max(0, (p - 0.5) / 0.5));
      break;
    case "slidedown":
      out.offsetY = p;
      into.offsetY = p - 1;
      break;
    // Cover and reveal each move one picture only, which is what separates them from slide.
    case "cover":
      into.offsetX = 1 - p;
      break;
    case "reveal":
      out.offsetX = -p;
      break;
    case "circleclose":
      out.clip = { kind: "circle", r: ((1 - p) * Math.hypot(1, 1)) / 2 };
      break;
    case "vertopen":
      into.clip = { kind: "rect", x: (1 - p) / 2, y: 0, w: p, h: 1 };
      break;
    case "horzopen":
      into.clip = { kind: "rect", x: 0, y: (1 - p) / 2, w: 1, h: p };
      break;
    case "corner":
      // `wipetl` retreats the outgoing picture into the top-left corner; the new one takes
      // everything it gives up.
      out.clip = { kind: "rect", x: 0, y: 0, w: 1 - p, h: 1 - p };
      break;
    case "squeeze":
      out.clip = { kind: "scale", sx: 1, sy: 1 - p };
      break;
    case "slices":
      into.clip = { kind: "stripes", axis: "x", count: SLICE_COUNT, p };
      break;
    case "vslices":
      into.clip = { kind: "stripes", axis: "y", count: SLICE_COUNT, p };
      break;
    default:
      out.alpha = 1 - p;
      into.alpha = p;
  }
  // Paint order. Most transitions bring the new picture in over the old one, but three
  // work the other way round: the outgoing picture stays in front and moves, shrinks or
  // is clipped away to let the new one show through. Drawing the incoming picture on top
  // of those hides the effect completely and the join reads as a hard cut.
  return { layers: OUTGOING_ON_TOP.has(doc.clips[crossing].transition.kind) ? [into, out] : [out, into], backdrop };
}

/**
 * Where in its video clip `index` is at show time `t` (Round 6), in the video's own
 * milliseconds: the clip's `in_ms` plus how far into the clip the show is. `null` for a
 * photograph. Before the clip starts (its incoming transition) it is held at `in_ms`.
 */
export function videoTimeAt(doc: SlideshowDoc, index: number, t: number): number | null {
  const clip = doc.clips[index];
  if (!clip?.video) return null;
  const start = timeline(doc).starts[index] ?? 0;
  return clip.video.in_ms + Math.min(clip.duration_ms, Math.max(0, t - start));
}
