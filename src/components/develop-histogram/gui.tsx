import { useMemo } from "react";
import { lerpArray, useTween } from "@/hooks/use-tween";
import type { DevelopHistogram as Histogram } from "@/types/generated/DevelopHistogram";
import "./styles.scss";

const H = 100;
/** How long the hills take to become the next photo's. */
const MORPH_MS = 320;
const NONE: number[] = [];

/** The four channels one after another, so one tween moves them together. */
const pack = (h: Histogram | null) => (h ? [...h.r, ...h.g, ...h.b, ...h.luma] : NONE);

/**
 * Height for a count: square-root scaled against `peak`, so one spike does not flatten the
 * rest (a log scale would make every shape look full).
 */
const height = (c: number, peak: number) => (H - Math.sqrt(c / peak) * H).toFixed(2);

/** An area under 256 counts. */
export function areaPath(counts: number[], peak: number): string {
  if (counts.length === 0 || peak <= 0) return "";
  const y = (c: number) => height(c, peak);
  let d = `M0 ${H}`;
  counts.forEach((c, i) => {
    d += ` L${i} ${y(c)}`;
  });
  return `${d} L${counts.length - 1} ${H} Z`;
}

/** The same counts as a line, for luminance. */
export function linePath(counts: number[], peak: number): string {
  if (counts.length === 0 || peak <= 0) return "";
  return counts.map((c, i) => `${i === 0 ? "M" : "L"}${i} ${height(c, peak)}`).join(" ");
}

/**
 * The photo's histogram (spec §3), measured by Rust on the rendered preview: red, green and
 * blue as translucent hills whose overlaps mix, luminance as a line. The triangles light when
 * anything clips and turn the clipping overlay on and off (`J`).
 */
export function DevelopHistogram({ histogram, clipping, onClipping }: { histogram: Histogram | null; clipping: boolean; onClipping: (on: boolean) => void }) {
  const h = histogram;
  // Another photo (or a slider) morphs the hills from what they were (Round 4A).
  const target = useMemo(() => pack(h), [h]);
  const all = useTween(target, lerpArray, MORPH_MS);
  const n = all.length / 4;
  const [r, g, b, luma] = [0, 1, 2, 3].map((k) => all.slice(k * n, (k + 1) * n));
  const peak = n ? Math.max(...r, ...g, ...b, 1) : 1;
  const clip = (side: "shadows" | "highlights", lit: boolean) => (
    <button
      type="button"
      className={`develop-histogram__clip develop-histogram__clip--${side}`}
      data-lit={lit}
      aria-pressed={clipping}
      aria-label={clipping ? "Hide clipping" : "Show clipping"}
      title={`${side === "shadows" ? "Shadows" : "Highlights"} ${lit ? "clip" : "do not clip"} · J shows clipping`}
      onClick={() => onClipping(!clipping)}
    />
  );
  return (
    <figure className="develop-histogram" aria-label="Histogram">
      <svg viewBox={`0 0 255 ${H}`} preserveAspectRatio="none" role="img" aria-label={h ? "Histogram of the developed photo" : "No histogram yet"}>
        {h && (
          <>
            <path className="develop-histogram__r" d={areaPath(r, peak)} />
            <path className="develop-histogram__g" d={areaPath(g, peak)} />
            <path className="develop-histogram__b" d={areaPath(b, peak)} />
            <path className="develop-histogram__luma" d={linePath(luma, Math.max(...luma, 1))} />
          </>
        )}
      </svg>
      {clip("shadows", (h?.clippedShadows ?? 0) > 0)}
      {clip("highlights", (h?.clippedHighlights ?? 0) > 0)}
    </figure>
  );
}
