// Before and after (Round 4J): pure layout for the canvas's compare modes.
import type { CompareMode } from "@/hooks/use-develop-session";
import type { Size } from "./zoom";

export type { CompareMode } from "@/hooks/use-develop-session";

export type Axis = "row" | "column";
type Box = { x: number; y: number; w: number; h: number };

/** The gap between side-by-side panes, in CSS pixels. */
export const PANE_GAP = 8;

/** The stage's rectangles left and right of a divider at `d` (0–1 of its width). */
export function splitClip(d: number, view: Size): { before: Box; after: Box } {
  const x = Math.min(1, Math.max(0, d)) * view.w;
  return { before: { x: 0, y: 0, w: x, h: view.h }, after: { x, y: 0, w: view.w - x, h: view.h } };
}

/** One pane's size when the stage is halved along `axis`. */
export function paneSize(view: Size, axis: Axis, gap = PANE_GAP): Size {
  return axis === "row" ? { w: Math.max(0, (view.w - gap) / 2), h: view.h } : { w: view.w, h: Math.max(0, (view.h - gap) / 2) };
}

/** Left and right, or top and bottom: whichever shows the photo larger. */
export function sideBySideAxis(photo: Size, view: Size, gap = PANE_GAP): Axis {
  const fit = (p: Size) => Math.min(p.w / photo.w, p.h / photo.h);
  return fit(paneSize(view, "row", gap)) >= fit(paneSize(view, "column", gap)) ? "row" : "column";
}

/** `Y`: split, then side by side, then off. */
export const nextCompare = (c: CompareMode): CompareMode => (c === "split" ? "side" : c === "side" ? "off" : "split");
