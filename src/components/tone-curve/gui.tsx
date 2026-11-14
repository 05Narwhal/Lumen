import { useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { monotone, type CurvePoint } from "@/lib/tone-curve";
import { addPoint, isDraggedOff, isEnd, movePoint, removePoint, withEnds } from "./points";
import "./styles.scss";

const SIZE = 256;
const SAMPLES = 128;

export type CurveChannel = "master" | "r" | "g" | "b";

/**
 * A point curve over one channel, drawn from the same monotone cubic Rust renders with.
 * Click to add a point, drag to move it (never past its neighbours; the ends only up and
 * down), drag it out or double-click to remove it. Points take focus: arrows nudge them by one
 * level (Shift: ten), Delete removes one.
 */
export function ToneCurve({
  points,
  channel,
  label,
  luma,
  onChange,
  onCommit,
}: {
  points: CurvePoint[];
  channel: CurveChannel;
  /** Named in History and to screen readers ("Red curve"). */
  label: string;
  /** Luminance counts to draw faintly behind. */
  luma?: number[];
  onChange: (points: CurvePoint[]) => void;
  onCommit: (points: CurvePoint[], label: string) => void;
}) {
  const ref = useRef<SVGSVGElement>(null);
  const edited = withEnds(points);
  const [drag, setDrag] = useState<number | null>(null);
  const live = useRef<CurvePoint[]>(edited);
  live.current = edited;

  const curvePath = useMemo(() => {
    const f = monotone(points);
    return Array.from({ length: SAMPLES + 1 }, (_, i) => {
      const x = i / SAMPLES;
      const y = Math.min(1, Math.max(0, f(x)));
      return `${i === 0 ? "M" : "L"}${(x * SIZE).toFixed(1)} ${((1 - y) * SIZE).toFixed(1)}`;
    }).join(" ");
  }, [points]);

  const lumaPath = useMemo(() => {
    if (!luma?.length) return "";
    const top = Math.log1p(Math.max(...luma, 1));
    return `M0 ${SIZE} ${luma.map((c, i) => `L${i} ${(SIZE - (Math.log1p(c) / top) * SIZE * 0.6).toFixed(1)}`).join(" ")} L255 ${SIZE} Z`;
  }, [luma]);

  const at = (e: PointerEvent) => {
    const r = ref.current!.getBoundingClientRect();
    return { x: (e.clientX - r.left) / r.width, y: 1 - (e.clientY - r.top) / r.height };
  };

  const down = (e: PointerEvent<SVGSVGElement>) => {
    const { x, y } = at(e);
    const hit = (e.target as Element).getAttribute("data-index");
    let index = hit === null ? null : Number(hit);
    if (index === null) {
      const added = addPoint(points, x, y);
      if (!added) return;
      index = added.index;
      onChange(added.points);
    }
    e.currentTarget.setPointerCapture?.(e.pointerId);
    setDrag(index);
  };
  const move = (e: PointerEvent<SVGSVGElement>) => {
    if (drag === null) return;
    const { x, y } = at(e);
    onChange(movePoint(live.current, drag, x, y));
  };
  const up = (e: PointerEvent<SVGSVGElement>) => {
    if (drag === null) return;
    const { x, y } = at(e);
    const i = drag;
    setDrag(null);
    e.currentTarget.releasePointerCapture?.(e.pointerId);
    const next = isDraggedOff(x, y) && !isEnd(live.current, i) ? removePoint(live.current, i) : live.current;
    onChange(next);
    onCommit(next, label);
  };

  const key = (i: number) => (e: KeyboardEvent) => {
    const step = (e.shiftKey ? 10 : 1) / 255;
    const [px, py] = edited[i];
    let next: CurvePoint[] | null = null;
    if (e.key === "ArrowUp") next = movePoint(edited, i, px, py + step);
    else if (e.key === "ArrowDown") next = movePoint(edited, i, px, py - step);
    else if (e.key === "ArrowLeft") next = movePoint(edited, i, px - step, py);
    else if (e.key === "ArrowRight") next = movePoint(edited, i, px + step, py);
    else if (e.key === "Delete" || e.key === "Backspace") next = removePoint(edited, i);
    if (!next) return;
    e.preventDefault();
    onChange(next);
    onCommit(next, label);
  };

  return (
    <div className="tone-curve" data-channel={channel}>
      <svg
        ref={ref}
        className="tone-curve__plot"
        viewBox={`0 0 ${SIZE} ${SIZE}`}
        role="group"
        aria-label={label}
        onPointerDown={down}
        onPointerMove={move}
        onPointerUp={up}
      >
        {[0.25, 0.5, 0.75].map((g) => (
          <g key={g} className="tone-curve__grid">
            <line x1={g * SIZE} y1={0} x2={g * SIZE} y2={SIZE} />
            <line x1={0} y1={g * SIZE} x2={SIZE} y2={g * SIZE} />
          </g>
        ))}
        <line className="tone-curve__diagonal" x1={0} y1={SIZE} x2={SIZE} y2={0} />
        {lumaPath && <path className="tone-curve__luma" d={lumaPath} />}
        <path className="tone-curve__line" d={curvePath} />
        {edited.map(([x, y], i) => (
          <circle
            key={i}
            className="tone-curve__point"
            data-index={i}
            data-active={drag === i}
            cx={x * SIZE}
            cy={(1 - y) * SIZE}
            r={5}
            tabIndex={0}
            role="slider"
            aria-label={`${label} point ${i + 1}`}
            aria-valuenow={Math.round(y * 255)}
            aria-valuemin={0}
            aria-valuemax={255}
            aria-valuetext={`input ${Math.round(x * 255)}, output ${Math.round(y * 255)}`}
            onKeyDown={key(i)}
            onDoubleClick={() => {
              const next = removePoint(edited, i);
              onChange(next);
              onCommit(next, label);
            }}
          />
        ))}
      </svg>
    </div>
  );
}
