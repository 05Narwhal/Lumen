import { useId, type MouseEvent } from "react";
import type { ShownTextLine } from "@/types/generated/ShownTextLine";

/** Units across the photo's height; its width is `aspect` times this. */
const UNITS = 1000;
/** A line's glyphs, as a share of its quad's height, and where their baseline sits. */
const FONT = 0.75;
const BASELINE = 0.78;

const r2 = (v: number) => Math.round(v * 100) / 100;

/** A quad in the SVG's units: its top-left corner, length along the text, height, angle. */
function fit(quad: ShownTextLine["quad"], w: number, h: number) {
  const [x0, y0, x1, y1, , , x3, y3] = [quad[0] * w, quad[1] * h, quad[2] * w, quad[3] * h, 0, 0, quad[6] * w, quad[7] * h];
  return {
    x: x0,
    y: y0,
    length: Math.hypot(x1 - x0, y1 - y0),
    height: Math.hypot(x3 - x0, y3 - y0),
    angle: (Math.atan2(y1 - y0, x1 - x0) * 180) / Math.PI,
  };
}

/** The selection when there is one, else the words of the line right-clicked. */
function menuText(e: MouseEvent): string | null {
  const selected = window.getSelection?.()?.toString().trim();
  if (selected) return selected;
  return (e.target as Element).closest?.("text")?.textContent ?? null;
}

/**
 * The text read in a photo over it (Round 13-3, Live Text–like), in `ZoomPan`'s overlay so it
 * zooms and pans with the photo. One SVG as wide as the photo: the photo is dimmed except
 * under each line, each line is outlined, and each is a real `<text>` stretched onto its quad
 * (`textLength`, rotated about its first corner) with transparent glyphs, so drag-select,
 * Ctrl/Cmd C, double-click and screen readers are the browser's own. Fades as a whole (its
 * parent); nothing here animates per line.
 */
export function TextOverlay({ lines, aspect, onMenu }: { lines: ShownTextLine[]; aspect: number; onMenu: (text: string | null, e: MouseEvent) => void }) {
  const mask = useId();
  const [w, h] = [UNITS * aspect, UNITS];
  const fitted = lines.map((l) => ({ line: l, f: fit(l.quad, w, h) }));
  const points = (q: ShownTextLine["quad"]) => [0, 2, 4, 6].map((i) => `${r2(q[i] * w)},${r2(q[i + 1] * h)}`).join(" ");
  return (
    <svg
      className="text-overlay"
      viewBox={`0 0 ${r2(w)} ${h}`}
      preserveAspectRatio="none"
      role="group"
      aria-label="Text in this photo"
      // A drag over the text selects it; ZoomPan underneath must not pan or swipe.
      onPointerDown={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onContextMenu={(e) => {
        e.preventDefault();
        e.stopPropagation();
        onMenu(menuText(e), e);
      }}
    >
      <defs>
        <mask id={mask}>
          <rect width={r2(w)} height={h} fill="white" />
          {fitted.map(({ line }) => (
            <polygon key={line.ord} points={points(line.quad)} fill="black" />
          ))}
        </mask>
      </defs>
      <rect className="text-overlay__dim" width={r2(w)} height={h} mask={`url(#${mask})`} />
      {fitted.map(({ line, f }) => (
        <g key={line.ord}>
          <polygon className="text-overlay__line" points={points(line.quad)} />
          <text
            className="text-overlay__text"
            x={r2(f.x)}
            y={r2(f.y + f.height * BASELINE)}
            fontSize={r2(f.height * FONT)}
            textLength={r2(f.length)}
            lengthAdjust="spacingAndGlyphs"
            transform={`rotate(${r2(f.angle)} ${r2(f.x)} ${r2(f.y)})`}
          >
            {line.text}
          </text>
        </g>
      ))}
    </svg>
  );
}
