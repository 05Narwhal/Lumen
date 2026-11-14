// Round 22 §2: images too large for the decoder, as the viewer and Develop tell them.

import type { Size } from "@/components/viewer/zoom-math";

/** Round 22 §2: a photo with more pixels than this is loaded in full in the background as soon as it opens. */
export const OVERSIZED_PIXELS = 100_000_000;

export function isOversized(width: number | null | undefined, height: number | null | undefined): boolean {
  return !!width && !!height && width * height > OVERSIZED_PIXELS;
}

/**
 * How much of the photo's catalogued size a loaded image is, in percent, when it is smaller
 * (the full resolution did not fit this PC's memory); `null` when it is all there. Long edges,
 * so an orientation that turns the photo does not matter.
 */
export function shownPercent(loaded: Size, real: Size): number | null {
  const l = Math.max(loaded.width, loaded.height);
  const r = Math.max(real.width, real.height);
  if (!l || !r || l >= r) return null;
  return Math.round((l / r) * 100);
}
