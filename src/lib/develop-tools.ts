import type { GridMode } from "@/components/crop-tool";
import { createStore } from "@/lib/store";
import type { Crop } from "@/types/generated/Crop";

/**
 * The crop tool's own settings in Develop (Phase 8e), shared by its panel and the canvas.
 * `aspect` is a width ÷ height in pixels, `"original"` (the frame's own) or null (free).
 */
export interface CropUi {
  aspect: number | "original" | null;
  /** Custom W:H, kept while another preset is chosen. */
  custom: [number, number];
  grid: GridMode;
  /** Drawing a level line on the canvas. */
  leveling: boolean;
}

export const cropUi = createStore<CropUi>({ aspect: null, custom: [5, 4], grid: "thirds", leveling: false });

/** Develop's aspect presets, in Lightroom's order. */
export const CROP_ASPECTS: { label: string; value: number | "original" | null | "custom" }[] = [
  { label: "Original", value: "original" },
  { label: "Free", value: null },
  { label: "1:1", value: 1 },
  { label: "4:5", value: 4 / 5 },
  { label: "3:2", value: 3 / 2 },
  { label: "16:9", value: 16 / 9 },
  { label: "Custom", value: "custom" },
];

/** The aspect a drag is held to, or null when free. */
export function cropAspect(ui: CropUi, frameAspect: number): number | null {
  if (ui.aspect === "original") return frameAspect;
  return ui.aspect;
}

/**
 * The angle to straighten by so a line drawn on screen from `a` to `b` becomes level (or
 * plumb, when it is nearer vertical), added to the photo's current `straighten`.
 */
export function levelFromLine(current: number, a: { x: number; y: number }, b: { x: number; y: number }): number {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  if (Math.hypot(dx, dy) < 4) return current;
  let deg = (Math.atan2(dy, dx) * 180) / Math.PI;
  // The nearest of level and plumb.
  while (deg > 45) deg -= 90;
  while (deg < -45) deg += 90;
  return Math.max(-45, Math.min(45, Math.round((current - deg) * 100) / 100));
}

export const FULL_CROP: Crop = { x: 0, y: 0, w: 1, h: 1 };
