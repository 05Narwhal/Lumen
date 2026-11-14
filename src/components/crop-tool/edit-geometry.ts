import type { Crop } from "@/types/generated/Crop";
import type { Recipe } from "@/types/generated/Recipe";

/**
 * The quick editor draws geometry itself, so a turn or a crop can animate: Rust renders the
 * photo once with no geometry, and these functions place it. They follow the renderer's
 * order exactly (`photos-media::edit::apply`): flips, quarter turns, straighten (rotate
 * and zoom so no corner is empty), then the crop, which is relative to the frame after all
 * of those.
 */

const mod360 = (v: number) => ((v % 360) + 360) % 360;

/** Turn a quarter clockwise (`1`) or anticlockwise (`-1`); the crop turns with the picture. */
export function rotateBy(r: Recipe, dir: 1 | -1): Recipe {
  const k = r.crop;
  const crop: Crop | null = k ? (dir === 1 ? { x: 1 - k.y - k.h, y: k.x, w: k.h, h: k.w } : { x: k.y, y: 1 - k.x - k.w, w: k.h, h: k.w }) : null;
  return { ...r, rotate: mod360(r.rotate + dir * 90), crop };
}

/**
 * Flip what is on screen. The recipe's flips come before the turn, so on a photo turned a
 * quarter a left-right flip on screen is an up-down flip of the source. A mirror also
 * mirrors the straighten angle and the crop.
 */
export function flipShown(r: Recipe, axis: "h" | "v"): Recipe {
  const quarter = r.rotate % 180 === 90;
  const sourceAxis = quarter ? (axis === "h" ? "v" : "h") : axis;
  const k = r.crop;
  const crop = k ? (axis === "h" ? { ...k, x: 1 - k.x - k.w } : { ...k, y: 1 - k.y - k.h }) : null;
  return {
    ...r,
    flip_h: sourceAxis === "h" ? !r.flip_h : r.flip_h,
    flip_v: sourceAxis === "v" ? !r.flip_v : r.flip_v,
    straighten: r.straighten === 0 ? 0 : -r.straighten,
    crop,
  };
}

/**
 * The on-screen angle for a stored `rotate`, as close as possible to the one shown now, so
 * the animation always takes the short way round (and 270 → 0 turns a quarter, not three).
 */
export function nearestSpin(current: number, rotate: number): number {
  const base = current - mod360(current);
  const candidates = [base - 360 + rotate, base + rotate, base + 360 + rotate];
  return candidates.reduce((best, c) => (Math.abs(c - current) < Math.abs(best - current) ? c : best));
}

/** The renderer's zoom: z = cos|θ| + sin|θ| · max(w/h, h/w). */
export function straightenZoom(deg: number, frameAspect: number): number {
  const t = (Math.abs(deg) * Math.PI) / 180;
  return Math.cos(t) + Math.sin(t) * Math.max(frameAspect, 1 / frameAspect);
}

export interface Box {
  width: number;
  height: number;
  left: number;
  top: number;
}

export interface EditLayout {
  /** What is visible: the whole frame while cropping, else the crop. */
  viewport: { width: number; height: number };
  /** The frame after turns and straighten, positioned inside the viewport. */
  frame: Box;
  /** The unrotated picture, centred in the frame; `transform` turns and flips it. */
  image: Box;
  transform: string;
  frameAspect: number;
}

// `+ 0` turns -0 into 0, which would otherwise leak into styles and comparisons.
const round = (v: number) => Math.round(v * 100) / 100 + 0;

export function editLayout({
  imageAspect,
  recipe,
  spin,
  cropping,
  stage,
}: {
  imageAspect: number;
  recipe: Recipe;
  /** The on-screen quarter-turn angle (see `nearestSpin`). */
  spin: number;
  cropping: boolean;
  stage: { width: number; height: number };
}): EditLayout {
  const quarter = recipe.rotate % 180 === 90;
  const frameAspect = quarter ? 1 / imageAspect : imageAspect;
  const crop = !cropping && recipe.crop ? recipe.crop : { x: 0, y: 0, w: 1, h: 1 };
  const viewAspect = (frameAspect * crop.w) / crop.h;
  const width = Math.min(stage.width, stage.height * viewAspect);
  const height = width / viewAspect;
  const fw = width / crop.w;
  const fh = height / crop.h;
  const [iw, ih] = quarter ? [fh, fw] : [fw, fh];
  const zoom = straightenZoom(recipe.straighten, frameAspect);
  const flips = recipe.flip_h || recipe.flip_v ? ` scale(${recipe.flip_h ? -1 : 1}, ${recipe.flip_v ? -1 : 1})` : "";
  return {
    viewport: { width: round(width), height: round(height) },
    frame: {
      width: round(fw),
      height: round(fh),
      left: round(-crop.x * fw),
      top: round(-crop.y * fh),
    },
    image: {
      width: round(iw),
      height: round(ih),
      left: round((fw - iw) / 2),
      top: round((fh - ih) / 2),
    },
    transform: `rotate(${round(spin + recipe.straighten)}deg) scale(${round(zoom * 10000) / 10000})${flips}`,
    frameAspect,
  };
}
