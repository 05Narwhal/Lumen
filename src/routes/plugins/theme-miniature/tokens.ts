import type { ThemeSwatch } from "@/types/generated/ThemeSwatch";

const PX = /^\s*(-?\d+(?:\.\d+)?)px\s*$/;

/**
 * A plain `px` length scaled to the miniature's size. Anything else (the swatches are checked
 * in Rust, but a store index is data) gets `fallback` rather than reaching the stylesheet.
 */
export function scaleLength(value: string, factor = 0.4, floor = 0, fallback = "4px"): string {
  const m = PX.exec(value);
  if (!m) return fallback;
  const n = Number(m[1]);
  return `${n <= 0 ? 0 : Math.max(floor, Math.round(n * factor))}px`;
}

/** The custom properties the miniature's stylesheet paints from; the swatch's own colours, untouched. */
export function miniatureVars(s: ThemeSwatch): Record<string, string> {
  return {
    "--mini-bg": s.bg,
    "--mini-surface": s.surface,
    "--mini-sidebar": s.sidebar,
    "--mini-fg": s.fg,
    "--mini-fg2": s.fg2,
    "--mini-accent": s.accent,
    "--mini-placeholder": s.placeholder,
    "--mini-radius": scaleLength(s.radius),
    "--mini-gap": scaleLength(s.gap, 0.5, 1, "3px"),
  };
}
