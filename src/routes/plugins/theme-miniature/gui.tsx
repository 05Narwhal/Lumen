import { motion, useReducedMotionConfig } from "motion/react";
import type { CSSProperties } from "react";
import type { ThemeSwatch } from "@/types/generated/ThemeSwatch";
import type { ThemeSwatches } from "@/types/generated/ThemeSwatches";
import { miniatureVars } from "./tokens";
import "./styles.scss";

const TILES = 6;

/** One scheme of the window: title bar, sidebar, a title line and a grid of photo tiles. */
function Window({ swatch }: { swatch: ThemeSwatch }) {
  return (
    <div className="theme-miniature__window" style={miniatureVars(swatch) as CSSProperties}>
      <div className="theme-miniature__title">
        <i />
        <i />
        <i />
      </div>
      <div className="theme-miniature__body">
        <div className="theme-miniature__sidebar">
          <b data-on />
          <b />
          <b />
          <b />
        </div>
        <div className="theme-miniature__main">
          <span className="theme-miniature__heading" />
          <div className="theme-miniature__grid">
            {Array.from({ length: TILES }, (_, i) => (
              <i key={i} data-accent={i === 1 ? "" : undefined} />
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

/**
 * A small Lumen window painted from a theme's own swatches, no images: both schemes are drawn
 * and `scheme` cross-fades between them. Decorative; the tile names the theme.
 */
export function ThemeMiniature({ tokens, scheme }: { tokens: ThemeSwatches; scheme: "light" | "dark" }) {
  const reduced = useReducedMotionConfig() ?? false;
  const fade = { duration: reduced ? 0 : 0.35, ease: [0.22, 1, 0.36, 1] as const };
  return (
    <div className="theme-miniature" aria-hidden data-scheme={scheme}>
      <Window swatch={tokens.light} />
      <motion.div className="theme-miniature__layer" initial={false} animate={{ opacity: scheme === "dark" ? 1 : 0 }} transition={fade}>
        <Window swatch={tokens.dark} />
      </motion.div>
    </div>
  );
}
