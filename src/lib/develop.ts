import { createStore } from "@/lib/store";
import type { Develop } from "@/types/generated/Develop";
import type { LocalAdjust } from "@/types/generated/LocalAdjust";
import type { Panel } from "@/types/generated/Panel";
import type { Recipe } from "@/types/generated/Recipe";
import defaults from "./develop-defaults.json";

/** `Develop::default()`, written by a Rust test so the two never drift. */
// JSON cannot say "exactly eight" of the HSL bands; the Rust test that writes it does.
export const DEFAULT_DEVELOP: Develop = defaults as unknown as Develop;

/** A recipe's develop settings, or the defaults when it has none. */
export const developOf = (r: Recipe): Develop => r.develop ?? DEFAULT_DEVELOP;

/** `r` with its develop replaced; geometry and trim stay. */
export const withDevelop = (r: Recipe, d: Develop): Recipe => ({ ...r, develop: d });

/** `r` with no crop, turn, flip or straighten: what the crop tool draws its geometry over. */
export const withoutGeometry = (r: Recipe): Recipe => ({ ...r, crop: null, rotate: 0, straighten: 0, flip_h: false, flip_v: false });

export const sameRecipe = (a: Recipe, b: Recipe) => JSON.stringify(a) === JSON.stringify(b);

/** Whether `r` crops, turns, flips or straightens: a preview of one that does not is the whole photo. */
export const hasGeometry = (r: Recipe) => !sameRecipe(r, withoutGeometry(r));

/** A signed number the way Develop's sliders show it, with a real minus sign. */
export function signed(v: number, digits = 0): string {
  const s = Math.abs(v).toFixed(digits);
  if (Number(s) === 0) return (0).toFixed(digits);
  return `${v > 0 ? "+" : "−"}${s}`;
}

/** The panels of the Develop screen, as `photos-core::develop::Panel` names them. */
export const PANELS: { value: Panel; label: string }[] = [
  { value: "basic", label: "Basic" },
  { value: "curve", label: "Tone curve" },
  { value: "hsl", label: "Colour" },
  { value: "grading", label: "Colour grading" },
  { value: "detail", label: "Detail" },
  { value: "effects", label: "Effects" },
];

/**
 * `into` with the chosen panels taken from `from` — the rule `photos-core::develop::merge_panels`
 * applies, used here for what the open photo shows (a hovered preset, a paste onto it).
 */
export function mergePanels(into: Develop, from: Develop, panels: Panel[]): Develop {
  const d = { ...into };
  for (const p of panels) {
    if (p === "basic") {
      d.basic = from.basic;
      d.auto = from.auto;
    } else if (p === "curve") d.curve = from.curve;
    else if (p === "hsl") d.hsl = from.hsl;
    else if (p === "grading") d.grading = from.grading;
    else if (p === "detail") d.detail = from.detail;
    else if (p === "effects") d.effects = from.effects;
    else if (from.masks?.length) d.masks = from.masks;
    else delete d.masks;
  }
  return d;
}

/** A mask's adjustments when it does nothing (`LocalAdjust::default()`). */
export const ZERO_ADJUST: LocalAdjust = {
  temperature: 0,
  tint: 0,
  exposure: 0,
  contrast: 0,
  highlights: 0,
  shadows: 0,
  whites: 0,
  blacks: 0,
  texture: 0,
  clarity: 0,
  dehaze: 0,
  saturation: 0,
  sharpness: 0,
};

/** What *Copy settings* holds until *Paste*. */
export const developClipboard = createStore<{ develop: Develop; panels: Panel[] } | null>(null);

/** Develop's own selection in its filmstrip, apart from the library's. */
export const developSelection = createStore<{ ids: Set<number>; anchor: number | null }>({ ids: new Set(), anchor: null });

/**
 * The frame a recipe produces from a display-oriented `w × h`, as `Recipe::output_size` does:
 * quarter turns swap the sides, a crop takes its share.
 */
export function outputSize(r: Recipe, w: number, h: number): [number, number] {
  const [tw, th] = r.rotate % 180 === 90 ? [h, w] : [w, h];
  if (!r.crop) return [tw, th];
  return [Math.max(1, Math.round(tw * r.crop.w)), Math.max(1, Math.round(th * r.crop.h))];
}
