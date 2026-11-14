import { useEffect, useRef } from "react";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { createStore } from "@/lib/store";
import type { Develop } from "@/types/generated/Develop";
import type { Effects } from "@/types/generated/Effects";
import type { LocalAdjust } from "@/types/generated/LocalAdjust";
import type { Mask } from "@/types/generated/Mask";
import type { MaskShape } from "@/types/generated/MaskShape";
import { DEFAULT_DEVELOP, ZERO_ADJUST } from "./develop";
import { type FrameMap, fromFrame, quantise } from "./mask-raster";

/** Phase 8e's masks, as plain functions over a photo's mask list (Rust renders them). */

/** The shapes drawn by hand; an AI one comes from a detection. */
export type ShapeKind = Exclude<MaskShape["kind"], "ai">;
export type RadialShape = Extract<MaskShape, { kind: "radial" }>;
export type LinearShape = Extract<MaskShape, { kind: "linear" }>;

export const SHAPE_NAMES: Record<ShapeKind, string> = { brush: "Brush", linear: "Linear gradient", radial: "Radial gradient" };

/** A brush's settings: radius as a share of the photo's long edge, feather and flow 0–1. */
export interface BrushSettings {
  radius: number;
  feather: number;
  flow: number;
}

/** The biggest brush: a radius of 0.3 of the long edge, a diameter of 60 %. */
export const BRUSH_MAX_RADIUS = 0.3;

/** The smallest brush's radius, half a pixel of the photo's long edge (a diameter of 1 px). */
export const brushMinRadius = (longEdgePx: number) => 0.5 / Math.max(1, longEdgePx);

/** A brush's diameter in pixels of the full photo. */
export const brushPixels = (radius: number, longEdgePx: number) => 2 * radius * longEdgePx;

const clampRadius = (radius: number, longEdgePx: number) => Math.min(BRUSH_MAX_RADIUS, Math.max(brushMinRadius(longEdgePx), radius));

/** The radius (share of the long edge) of a brush `px` wide, held between 1 px and 60 %. */
export const brushRadiusOf = (px: number, longEdgePx: number) => clampRadius(px / (2 * Math.max(1, longEdgePx)), longEdgePx);

/** The Size slider's 0–1 position for a diameter: logarithmic, `px = (60 % of the edge)^t`. */
export function brushPosition(px: number, longEdgePx: number) {
  const top = 2 * BRUSH_MAX_RADIUS * Math.max(2, longEdgePx);
  return Math.min(1, Math.max(0, Math.log(Math.max(1, px)) / Math.log(top)));
}

/** The diameter in pixels at a slider position; the inverse of `brushPosition`. */
export const brushSizeAt = (t: number, longEdgePx: number) => (2 * BRUSH_MAX_RADIUS * Math.max(2, longEdgePx)) ** Math.min(1, Math.max(0, t));

/** `[` (−1) and `]` (+1): a tenth bigger or smaller, never under 1 px or over 60 %. */
export const stepBrushRadius = (radius: number, dir: 1 | -1, longEdgePx: number) => clampRadius(dir > 0 ? radius * 1.1 : radius / 1.1, longEdgePx);

/**
 * Alt + right-drag (Photoshop's): `dx` and `dy` are the pointer's travel in screen pixels since
 * the press. Sideways moves the ring's edge with the pointer one to one, up softens it.
 * `longEdgeOnScreen` is the photo's long edge as drawn; `min` the smallest radius allowed.
 */
export function brushDrag(from: BrushSettings, dx: number, dy: number, longEdgeOnScreen: number, min: number): BrushSettings {
  const radius = longEdgeOnScreen > 0 ? Math.min(BRUSH_MAX_RADIUS, Math.max(min, from.radius + dx / longEdgeOnScreen)) : from.radius;
  return { ...from, radius, feather: Math.min(1, Math.max(0, from.feather - dy / 200)) };
}

/**
 * What the Masks panel and the canvas share: the selected mask, the overlay, the brush, and
 * whether a stroke paints (`add`) or erases.
 */
export interface MaskUi {
  selected: number | null;
  overlay: boolean;
  /** Index into `OVERLAY_COLOURS`. */
  colour: number;
  brush: BrushSettings;
  /** Painting with the brush: adding to the mask or subtracting from it. Null: handles only. */
  painting: { add: boolean } | null;
  /** A slider, a handle or a stroke is being dragged: the tint steps aside until release. */
  active: boolean;
}

export const maskUi = createStore<MaskUi>({ selected: null, overlay: true, active: false, colour: 0, brush: { radius: 0.05, feather: 0.5, flow: 1 }, painting: null });

/**
 * Marks an edit in progress (Round 5): the overlay hides until the pointer is released
 * anywhere, so a release outside the control that began the drag still brings it back.
 */
export function holdMaskActive() {
  if (typeof window === "undefined") return;
  const release = () => {
    window.removeEventListener("pointerup", release);
    window.removeEventListener("pointercancel", release);
    maskUi.set((u) => (u.active ? { ...u, active: false } : u));
  };
  window.addEventListener("pointerup", release);
  window.addEventListener("pointercancel", release);
  maskUi.set((u) => (u.active ? u : { ...u, active: true }));
}

/**
 * Whether the overlay shows after the selected mask went from `prev` to `next` (Round 8B):
 * moving, resizing or painting a shape shows it, even if it was off, so the mask can be seen
 * while it is placed; changing the mask's photo adjustments hides it, so the result can be
 * judged. Anything else (another mask selected, nothing changed) leaves `overlay` as it is.
 */
export function overlayAfter(prev: Mask | undefined, next: Mask | undefined, overlay: boolean): boolean {
  if (!prev || !next || prev.id !== next.id) return overlay;
  if (JSON.stringify(prev.shapes) !== JSON.stringify(next.shapes)) return true;
  if (JSON.stringify(prev.adjust) !== JSON.stringify(next.adjust)) return false;
  return overlay;
}

/** A shape is grabbed or a stroke begun: the overlay shows from the press, not the first move. */
export function showMaskOverlay() {
  maskUi.set((u) => (u.overlay ? u : { ...u, overlay: true }));
}

/** Applies `overlayAfter` to `maskUi` as the selected mask changes. `O` still sets it by hand. */
export function useMaskOverlayRule(mask: Mask | undefined) {
  const last = useRef(mask);
  useEffect(() => {
    const prev = last.current;
    last.current = mask;
    if (prev === mask) return;
    maskUi.set((u) => {
      const overlay = overlayAfter(prev, mask, u.overlay);
      return overlay === u.overlay ? u : { ...u, overlay };
    });
  }, [mask]);
}

/** The overlay's colours, as tokens (`Shift+O` cycles them). */
export const OVERLAY_COLOURS = ["--mask-overlay-red", "--mask-overlay-green", "--mask-overlay-blue", "--mask-overlay-white"];

export function defaultShape(kind: ShapeKind, add: boolean): MaskShape {
  if (kind === "radial") return { kind, add, center: [0.5, 0.5], radius: [0.25, 0.25], angle: 0, feather: 0.5, inside: true };
  if (kind === "linear") return { kind, add, from: [0.5, 0.2], to: [0.5, 0.6] };
  return { kind, add, radius: 0.05, feather: 0.5, flow: 1, points: [] };
}

/** A new mask after the highest id. Gradients start with a shape; a brush starts empty. */
export function newMask(masks: Mask[], kind: ShapeKind): Mask {
  const id = masks.reduce((m, x) => Math.max(m, x.id), 0) + 1;
  return { id, name: `Mask ${id}`, shapes: kind === "brush" ? [] : [defaultShape(kind, true)], invert: false, opacity: 1, adjust: ZERO_ADJUST, visible: true };
}

export const updateMask = (masks: Mask[], id: number, change: Partial<Mask>): Mask[] => masks.map((m) => (m.id === id ? { ...m, ...change } : m));

export const setAdjust = (masks: Mask[], id: number, change: Partial<LocalAdjust>): Mask[] =>
  masks.map((m) => (m.id === id ? { ...m, adjust: { ...m.adjust, ...change } } : m));

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const SHARPEN = DEFAULT_DEVELOP.detail.sharpen;
/** What the optional effects are when a recipe leaves them out (as `Glow::default()` and the like). */
const EFFECT_OFF = { motion_blur: { amount: 0, angle: 0 }, glow: { amount: 0, size: 0.4 }, halation: { amount: 0, size: 0.3 } };

/**
 * A mask's adjustments as a whole `Develop`, so the Develop panels can show and edit them
 * unchanged (Round 19 §1). What a mask cannot do (crop, vignette, Denoise, Auto) stays default.
 */
export function adjustView(a: LocalAdjust): Develop {
  const effects: Effects = { ...DEFAULT_DEVELOP.effects, grain: a.grain ?? DEFAULT_DEVELOP.effects.grain };
  if (a.motion_blur) effects.motion_blur = a.motion_blur;
  if (a.glow) effects.glow = a.glow;
  if (a.halation) effects.halation = a.halation;
  const own = a.sharpen;
  return {
    ...DEFAULT_DEVELOP,
    basic: {
      temperature: a.temperature,
      tint: a.tint,
      exposure: a.exposure,
      contrast: a.contrast,
      highlights: a.highlights,
      shadows: a.shadows,
      whites: a.whites,
      blacks: a.blacks,
      texture: a.texture,
      clarity: a.clarity,
      dehaze: a.dehaze,
      vibrance: a.vibrance ?? 0,
      saturation: a.saturation,
    },
    curve: a.curve ?? DEFAULT_DEVELOP.curve,
    hsl: a.hsl ?? DEFAULT_DEVELOP.hsl,
    grading: a.grading ?? DEFAULT_DEVELOP.grading,
    detail: { sharpen: { amount: a.sharpness, radius: own?.radius ?? SHARPEN.radius, detail: own?.detail ?? SHARPEN.detail, masking: own?.masking ?? SHARPEN.masking }, denoise: null },
    effects,
    auto: false,
  };
}

/** The inverse of `adjustView`: values still at default are left out, as Rust writes them. */
export function adjustFrom(view: Develop): LocalAdjust {
  const { basic: b, detail, effects: e } = view;
  const s = detail.sharpen;
  const out: LocalAdjust = {
    temperature: b.temperature,
    tint: b.tint,
    exposure: b.exposure,
    contrast: b.contrast,
    highlights: b.highlights,
    shadows: b.shadows,
    whites: b.whites,
    blacks: b.blacks,
    texture: b.texture,
    clarity: b.clarity,
    dehaze: b.dehaze,
    saturation: b.saturation,
    sharpness: s.amount,
  };
  if (b.vibrance !== 0) out.vibrance = b.vibrance;
  if (!same(view.curve, DEFAULT_DEVELOP.curve)) out.curve = view.curve;
  if (!same(view.hsl, DEFAULT_DEVELOP.hsl)) out.hsl = view.hsl;
  if (!same(view.grading, DEFAULT_DEVELOP.grading)) out.grading = view.grading;
  if (s.radius !== SHARPEN.radius || s.detail !== SHARPEN.detail || s.masking !== SHARPEN.masking) out.sharpen = { radius: s.radius, detail: s.detail, masking: s.masking };
  if (!same(e.grain, DEFAULT_DEVELOP.effects.grain)) out.grain = e.grain;
  if (e.motion_blur && !same(e.motion_blur, EFFECT_OFF.motion_blur)) out.motion_blur = e.motion_blur;
  if (e.glow && !same(e.glow, EFFECT_OFF.glow)) out.glow = e.glow;
  if (e.halation && !same(e.halation, EFFECT_OFF.halation)) out.halation = e.halation;
  return out;
}

/** Whether a mask does anything besides its shapes: any adjustment off its default. */
export const adjustChanged = (a: LocalAdjust): boolean => !same(adjustFrom(adjustView(a)), ZERO_ADJUST);

/**
 * The session the Develop panels see in mask mode: `develop` is the mask's adjustments, and
 * `update` / `apply` write into that mask (one photo change, so one undo step). Everything
 * else is the photo's session.
 */
export function maskSession(session: DevelopSession, maskId: number): DevelopSession {
  const lift =
    (change: Partial<Develop> | ((d: Develop) => Develop)) =>
    (photo: Develop): Develop => {
      const mask = photo.masks?.find((m) => m.id === maskId);
      if (!mask || !photo.masks) return photo;
      const view = adjustView(mask.adjust);
      const next = typeof change === "function" ? change(view) : { ...view, ...change };
      return { ...photo, masks: photo.masks.map((m) => (m.id === maskId ? { ...m, adjust: adjustFrom(next) } : m)) };
    };
  const mask = session.develop.masks?.find((m) => m.id === maskId);
  return {
    ...session,
    develop: adjustView(mask?.adjust ?? ZERO_ADJUST),
    update: (change) => session.update(lift(change)),
    apply: (change, label) => session.apply(lift(change), label),
  };
}

export const removeMask = (masks: Mask[], id: number): Mask[] => masks.filter((m) => m.id !== id);

/** `id` moved to array index `toIndex` (clamped); the same list back when nothing moves. Later = applied later = drawn higher. */
export function moveMask(masks: Mask[], id: number, toIndex: number): Mask[] {
  const from = masks.findIndex((m) => m.id === id);
  if (from < 0) return masks;
  const to = Math.min(masks.length - 1, Math.max(0, toIndex));
  if (to === from) return masks;
  const next = masks.slice();
  const [m] = next.splice(from, 1);
  next.splice(to, 0, m);
  return next;
}

export function duplicateMask(masks: Mask[], id: number): Mask[] {
  const src = masks.find((m) => m.id === id);
  if (!src) return masks;
  const next = masks.reduce((m, x) => Math.max(m, x.id), 0) + 1;
  const i = masks.indexOf(src);
  return [...masks.slice(0, i + 1), { ...src, id: next, name: `${src.name} copy` }, ...masks.slice(i + 1)];
}

export const addShape = (masks: Mask[], id: number, kind: ShapeKind, add: boolean): Mask[] =>
  masks.map((m) => (m.id === id ? { ...m, shapes: [...m.shapes, defaultShape(kind, add)] } : m));

export const replaceShape = (masks: Mask[], id: number, index: number, shape: MaskShape): Mask[] =>
  masks.map((m) => (m.id === id ? { ...m, shapes: m.shapes.map((s, i) => (i === index ? shape : s)) } : m));

/** A stroke (points 0–1 of the unedited photo) added to a mask as one brush shape. */
export function appendStroke(masks: Mask[], id: number, brush: BrushSettings & { add: boolean }, points: [number, number][]): Mask[] {
  const shape: MaskShape = { kind: "brush", add: brush.add, radius: brush.radius, feather: brush.feather, flow: brush.flow, points: points.map(quantise) };
  return masks.map((m) => (m.id === id ? { ...m, shapes: [...m.shapes, shape] } : m));
}

/** Unedited-photo pixels between two points (0–1): radii and angles live there. */
const photoPx = (f: FrameMap, a: [number, number], b: [number, number]) => [(b[0] - a[0]) * f.base[0], (b[1] - a[1]) * f.base[1]];

/**
 * A radial's size handle dragged to frame point `p`: its radius along that axis becomes the
 * pointer's distance along the axis, measured in the unedited photo (so turns, flips and
 * crops map back).
 */
export function radialResize(shape: RadialShape, axis: "x" | "y", f: FrameMap, p: [number, number]): RadialShape {
  const [dx, dy] = photoPx(f, shape.center, fromFrame(f, p));
  const t = (shape.angle * Math.PI) / 180;
  const along = axis === "x" ? Math.abs(dx * Math.cos(t) + dy * Math.sin(t)) / f.base[0] : Math.abs(-dx * Math.sin(t) + dy * Math.cos(t)) / f.base[1];
  const r = Math.max(0.005, Math.min(2, along));
  return { ...shape, radius: axis === "x" ? [r, shape.radius[1]] : [shape.radius[0], r] };
}

/** A radial's rotate handle dragged to `p`: the angle of the pointer about the centre. */
export function radialRotate(shape: RadialShape, f: FrameMap, p: [number, number]): RadialShape {
  const [dx, dy] = photoPx(f, shape.center, fromFrame(f, p));
  // The handle sits on the y axis (−90° from x), so the pointer's angle less −90.
  const deg = (Math.atan2(dy, dx) * 180) / Math.PI + 90;
  return { ...shape, angle: ((deg % 360) + 360) % 360 };
}

/** A radial's feather ring dragged to `p`: the inner ellipse's share of the outer. */
export function radialFeather(shape: RadialShape, f: FrameMap, p: [number, number]): RadialShape {
  const [dx, dy] = photoPx(f, shape.center, fromFrame(f, p));
  const t = (shape.angle * Math.PI) / 180;
  const q1 = (dx * Math.cos(t) + dy * Math.sin(t)) / (shape.radius[0] * f.base[0]);
  const q2 = (-dx * Math.sin(t) + dy * Math.cos(t)) / (shape.radius[1] * f.base[1]);
  return { ...shape, feather: Math.max(0, Math.min(1, 1 - Math.hypot(q1, q2))) };
}

/** Moves a point (0–1) by a frame displacement. */
export function moveBy(f: FrameMap, point: [number, number], from: [number, number], to: [number, number]): [number, number] {
  const a = fromFrame(f, from);
  const b = fromFrame(f, to);
  return [point[0] + b[0] - a[0], point[1] + b[1] - a[1]];
}

/** A linear gradient turned about its centre so it points along frame direction `p − centre`. */
export function linearRotate(shape: LinearShape, f: FrameMap, p: [number, number], centreFrame: [number, number]): LinearShape {
  const c: [number, number] = [(shape.from[0] + shape.to[0]) / 2, (shape.from[1] + shape.to[1]) / 2];
  const [hx, hy] = photoPx(f, shape.from, shape.to).map((v) => v / 2);
  const half = Math.hypot(hx, hy);
  const [dx, dy] = photoPx(f, fromFrame(f, centreFrame), fromFrame(f, p));
  const len = Math.hypot(dx, dy) || 1;
  // The rotate handle lies beyond `to`: `to` follows the pointer's direction.
  const [ux, uy] = [((dx / len) * half) / f.base[0], ((dy / len) * half) / f.base[1]];
  return { ...shape, from: [c[0] - ux, c[1] - uy], to: [c[0] + ux, c[1] + uy] };
}
