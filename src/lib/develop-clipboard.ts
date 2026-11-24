import type { Develop } from "@/types/generated/Develop";
import type { LocalAdjust } from "@/types/generated/LocalAdjust";
import type { Mask } from "@/types/generated/Mask";
import type { MaskShape } from "@/types/generated/MaskShape";
import type { Panel } from "@/types/generated/Panel";
import { type CopiedSettings, DEFAULT_DEVELOP, ZERO_ADJUST } from "./develop";

/**
 * Round 24 §5: *Copy settings* as readable JSON on the system clipboard, and the way back in.
 * Pure: nothing here touches the clipboard, the photo or the backend.
 */

export const FORMAT = "develop-settings";
/** The newest document version this build understands. */
export const KNOWN_VERSION = 1;
/** Clipboard text longer than this is never parsed (a pasted file, not settings). */
export const MAX_CLIPBOARD_CHARS = 1_000_000;

const ALL_PANELS: Panel[] = ["basic", "curve", "hsl", "grading", "detail", "effects"];
const PANEL_NAMES = new Set<string>([...ALL_PANELS, "masks"]);

export function serializeSettings(s: CopiedSettings, appVersion: string): string {
  return JSON.stringify({ lumen: FORMAT, version: KNOWN_VERSION, app: appVersion, settings: s }, null, 2);
}

export interface ParsedSettings {
  settings: CopiedSettings;
  /** Brush shapes left out: they belong to the pixels of their own photo. */
  droppedBrush: number;
  /** The document is from a newer Lumen: applied where understood. */
  newer: boolean;
}

type Obj = Record<string, unknown>;
const isObj = (v: unknown): v is Obj => typeof v === "object" && v !== null && !Array.isArray(v);
const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));

/** A finite number within `lo..hi`, or `fallback` when it isn't a number. */
const num = (v: unknown, lo: number, hi: number, fallback: number) => (typeof v === "number" && Number.isFinite(v) ? clamp(v, lo, hi) : fallback);

/** Basic's ranges: exposure ±5 stops, temperature and tint ±100, the rest ±1. */
const BASIC_RANGE: Record<string, number> = { temperature: 100, tint: 100, exposure: 5 };
const range = (key: string) => BASIC_RANGE[key] ?? 1;

/** `into`'s numeric fields read from `from`, clamped by Basic's ranges; other fields untouched. */
function clampNumbers<T extends object>(from: unknown, into: T, skip: readonly string[] = []): T {
  if (!isObj(from)) return into;
  const out = { ...into } as Record<string, unknown>;
  for (const [k, def] of Object.entries(into)) {
    if (skip.includes(k)) continue;
    if (typeof def === "number") out[k] = num(from[k], -range(k), range(k), def);
  }
  return out as T;
}

/** Every number in `v` finite (a non-finite one is not JSON, but a damaged document might hold one as text). */
const finiteDeep = (v: unknown): boolean => (typeof v === "number" ? Number.isFinite(v) : Array.isArray(v) ? v.every(finiteDeep) : isObj(v) ? Object.values(v).every(finiteDeep) : true);

/** A sub-document kept as it came when it is the same kind of thing as the default, else the default. */
const sameKind = <T>(v: unknown, def: T): T => {
  if (Array.isArray(def)) return Array.isArray(v) && finiteDeep(v) ? (v as T) : def;
  if (isObj(def)) return isObj(v) && finiteDeep(v) ? (v as T) : def;
  return def;
};

function cleanAdjust(v: unknown): LocalAdjust {
  const base = clampNumbers(v, ZERO_ADJUST);
  if (!isObj(v)) return base;
  const out: LocalAdjust = { ...base };
  const optional = ["curve", "hsl", "grading", "sharpen", "grain", "glow", "halation", "motion_blur"] as const;
  for (const k of optional) if (v[k] !== undefined && isObj(v[k]) !== Array.isArray(v[k]) && finiteDeep(v[k])) (out as Record<string, unknown>)[k] = v[k];
  if (typeof v.vibrance === "number") out.vibrance = num(v.vibrance, -1, 1, 0);
  return out;
}

const point = (v: unknown): [number, number] | null =>
  Array.isArray(v) && v.length === 2 && typeof v[0] === "number" && typeof v[1] === "number" && Number.isFinite(v[0]) && Number.isFinite(v[1]) ? [v[0], v[1]] : null;

/** A shape as Lumen writes it, or null (a brush, or damaged). AI shapes keep their kind, not the raster. */
function cleanShape(v: unknown): MaskShape | null {
  if (!isObj(v) || typeof v.add !== "boolean") return null;
  if (v.kind === "linear") {
    const from = point(v.from);
    const to = point(v.to);
    return from && to ? { kind: "linear", add: v.add, from, to } : null;
  }
  if (v.kind === "radial") {
    const center = point(v.center);
    const radius = point(v.radius);
    if (!center || !radius) return null;
    return { kind: "radial", add: v.add, center, radius, angle: num(v.angle, -360, 360, 0), feather: num(v.feather, 0, 1, 0.5), inside: v.inside !== false };
  }
  if (v.kind === "ai" && typeof v.target === "string" && ["sky", "subject", "background", "person"].includes(v.target)) {
    return { kind: "ai", add: v.add, target: v.target as "sky", instance: typeof v.instance === "number" && v.instance >= 0 ? Math.floor(v.instance) : 0, raster: "" };
  }
  return null;
}

function cleanMasks(v: unknown): { masks: Mask[]; droppedBrush: number } {
  const masks: Mask[] = [];
  let droppedBrush = 0;
  if (!Array.isArray(v)) return { masks, droppedBrush };
  for (const m of v) {
    if (!isObj(m) || !Array.isArray(m.shapes)) continue;
    const shapes: MaskShape[] = [];
    for (const s of m.shapes) {
      if (isObj(s) && s.kind === "brush") droppedBrush++;
      else {
        const c = cleanShape(s);
        if (c) shapes.push(c);
      }
    }
    if (shapes.length === 0) continue;
    masks.push({
      id: typeof m.id === "number" && Number.isFinite(m.id) ? Math.floor(m.id) : masks.length + 1,
      name: typeof m.name === "string" ? m.name.slice(0, 120) : `Mask ${masks.length + 1}`,
      shapes,
      invert: m.invert === true,
      opacity: num(m.opacity, 0, 1, 1),
      adjust: cleanAdjust(m.adjust),
      visible: m.visible !== false,
    });
  }
  return { masks, droppedBrush };
}

function cleanDevelop(v: Obj): { develop: Develop; droppedBrush: number } {
  const d = DEFAULT_DEVELOP;
  const hsl = Array.isArray(v.hsl) && v.hsl.length === d.hsl.length ? (v.hsl.map((b, i) => clampNumbers(b, d.hsl[i])) as Develop["hsl"]) : d.hsl;
  const develop: Develop = {
    basic: clampNumbers(v.basic, d.basic),
    curve: sameKind(v.curve, d.curve),
    hsl,
    grading: sameKind(v.grading, d.grading),
    detail: sameKind(v.detail, d.detail),
    effects: sameKind(v.effects, d.effects),
    auto: v.auto === true,
  };
  const { masks, droppedBrush } = cleanMasks(v.masks);
  if (masks.length > 0) develop.masks = masks;
  return { develop, droppedBrush };
}

/**
 * Lumen's settings document in `text`, made safe to apply: fields Lumen doesn't know are ignored,
 * numbers clamped, brush masks left out, AI masks kept by kind. `null` for anything else (other
 * JSON, other text, over 1 MB), so a paste then falls back to Lumen's own clipboard.
 */
export function parseSettings(text: string): ParsedSettings | null {
  if (text.length === 0 || text.length > MAX_CLIPBOARD_CHARS) return null;
  let doc: unknown;
  try {
    doc = JSON.parse(text);
  } catch {
    return null;
  }
  if (!isObj(doc) || doc.lumen !== FORMAT || typeof doc.version !== "number" || !(doc.version >= 1)) return null;
  const s = doc.settings;
  if (!isObj(s) || !isObj(s.develop)) return null;
  const { develop, droppedBrush } = cleanDevelop(s.develop);
  const listed = Array.isArray(s.panels) ? (s.panels.filter((p) => typeof p === "string" && PANEL_NAMES.has(p)) as Panel[]) : [];
  return { settings: { develop, panels: listed.length > 0 ? listed : ALL_PANELS }, droppedBrush, newer: doc.version > KNOWN_VERSION };
}
