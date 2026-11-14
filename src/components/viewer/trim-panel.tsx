import { Button } from "@mantine/core";
import { IconArrowBarToLeft, IconArrowBarToRight, IconCut, IconRestore, IconScissors } from "@tabler/icons-react";
import { useRef, type KeyboardEvent, type PointerEvent } from "react";
import type { Segment } from "@/types/generated/Segment";
import { keptMs } from "@/lib/trim";
import { formatDuration } from "@/utils/date";

export interface Selection {
  a: number;
  b: number;
}

const DRAG_PX = 4;
const pct = (ms: number, total: number) => `${total > 0 ? (ms / total) * 100 : 0}%`;

/** The stretches that are cut: the complement of the kept segments within the clip. */
export function cutsOf(segs: Segment[], total: number): Segment[] {
  const cuts: Segment[] = [];
  let at = 0;
  for (const s of segs) {
    if (s.start_ms > at) cuts.push({ start_ms: at, end_ms: s.start_ms });
    at = Math.max(at, s.end_ms);
  }
  if (at < total) cuts.push({ start_ms: at, end_ms: total });
  return cuts;
}

/**
 * The clip laid out left to right over its poster: cut stretches are hatched, the selection
 * is a framed range with a handle at each end, and the playhead is a line. A click seeks;
 * a drag across it selects.
 */
export function TrimTimeline({
  durationMs,
  segments,
  selection,
  timeMs,
  posterUrl,
  onSeek,
  onSelect,
}: {
  durationMs: number;
  segments: Segment[];
  selection: Selection | null;
  timeMs: number;
  posterUrl: string;
  onSeek: (ms: number) => void;
  onSelect: (s: Selection | null) => void;
}) {
  const track = useRef<HTMLDivElement>(null);
  const drag = useRef<{
    x0: number;
    t0: number;
    mode: "new" | "a" | "b";
    moved: boolean;
  } | null>(null);

  const timeAt = (clientX: number) => {
    const r = track.current?.getBoundingClientRect();
    if (!r || r.width === 0) return 0;
    return Math.round(Math.min(1, Math.max(0, (clientX - r.left) / r.width)) * durationMs);
  };

  const down = (mode: "new" | "a" | "b") => (e: PointerEvent) => {
    e.stopPropagation();
    (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
    drag.current = { x0: e.clientX, t0: timeAt(e.clientX), mode, moved: false };
  };
  const move = (e: PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    if (!d.moved && Math.abs(e.clientX - d.x0) < DRAG_PX) return;
    d.moved = true;
    const t = timeAt(e.clientX);
    if (d.mode === "new") onSelect({ a: Math.min(d.t0, t), b: Math.max(d.t0, t) });
    else if (selection) {
      const other = d.mode === "a" ? selection.b : selection.a;
      onSelect({ a: Math.min(other, t), b: Math.max(other, t) });
    }
    onSeek(t);
  };
  const up = (e: PointerEvent) => {
    const d = drag.current;
    drag.current = null;
    if (d && !d.moved && d.mode === "new") onSeek(timeAt(e.clientX));
  };

  const onKey = (e: KeyboardEvent) => {
    const step = e.shiftKey ? 5000 : 1000;
    if (e.key === "ArrowLeft") onSeek(Math.max(0, timeMs - step));
    else if (e.key === "ArrowRight") onSeek(Math.min(durationMs, timeMs + step));
    else if (e.key === "Home") onSeek(0);
    else if (e.key === "End") onSeek(durationMs);
    else return;
    e.preventDefault();
  };

  return (
    <div
      ref={track}
      className="trim-timeline"
      style={{ backgroundImage: `url(${posterUrl})` }}
      role="slider"
      tabIndex={0}
      aria-label="Timeline"
      aria-valuemin={0}
      aria-valuemax={Math.round(durationMs)}
      aria-valuenow={Math.round(timeMs)}
      aria-valuetext={formatDuration(timeMs)}
      onPointerDown={down("new")}
      onPointerMove={move}
      onPointerUp={up}
      onKeyDown={onKey}
    >
      {cutsOf(segments, durationMs).map((c) => (
        <span
          key={c.start_ms}
          className="trim-timeline__cut"
          style={{
            left: pct(c.start_ms, durationMs),
            width: pct(c.end_ms - c.start_ms, durationMs),
          }}
        />
      ))}
      {selection && (
        <span
          className="trim-timeline__sel"
          style={{
            left: pct(selection.a, durationMs),
            width: pct(selection.b - selection.a, durationMs),
          }}
        >
          <span className="trim-timeline__handle" data-edge="a" aria-label="Selection start" onPointerDown={down("a")} onPointerMove={move} onPointerUp={up} />
          <span className="trim-timeline__handle" data-edge="b" aria-label="Selection end" onPointerDown={down("b")} onPointerMove={move} onPointerUp={up} />
        </span>
      )}
      <span className="trim-timeline__head" style={{ left: pct(timeMs, durationMs) }} />
    </div>
  );
}

/**
 * The trimmer's controls, in a bar under the timeline. Nothing is cut from the file: the
 * kept segments are stored in the recipe, the viewer plays only those, and an export
 * writes them out as a new file.
 */
export function TrimPanel({
  durationMs,
  segments,
  selection,
  timeMs,
  onSelect,
  onCut,
  onKeep,
  onReset,
}: {
  durationMs: number;
  segments: Segment[];
  selection: Selection | null;
  timeMs: number;
  onSelect: (s: Selection | null) => void;
  onCut: () => void;
  onKeep: () => void;
  onReset: () => void;
}) {
  const kept = keptMs(segments);
  const trimmed = kept < durationMs - 50;
  const setEdge = (edge: "a" | "b") => {
    const s = selection ?? {
      a: edge === "a" ? timeMs : 0,
      b: edge === "b" ? timeMs : durationMs,
    };
    const next = edge === "a" ? { a: timeMs, b: Math.max(timeMs, s.b) } : { a: Math.min(s.a, timeMs), b: timeMs };
    onSelect(next.b > next.a ? next : null);
  };
  return (
    <div className="trim-bar" role="toolbar" aria-label="Trim">
      <p className="trim-bar__summary">
        Plays <strong>{formatDuration(kept)}</strong> of {formatDuration(durationMs)}
        {segments.length > 1 && ` in ${segments.length} parts`}
      </p>
      <span className="trim-bar__rule" aria-hidden />
      <button type="button" className="trim-bar__tool" title="Start selection here (I)" onClick={() => setEdge("a")}>
        <IconArrowBarToRight size={16} aria-hidden />
        <span>Start</span>
      </button>
      <button type="button" className="trim-bar__tool" title="End selection here (O)" onClick={() => setEdge("b")}>
        <IconArrowBarToLeft size={16} aria-hidden />
        <span>End</span>
      </button>
      <span className="trim-bar__selection">
        {selection
          ? `${formatDuration(selection.a)} – ${formatDuration(selection.b)} (${formatDuration(selection.b - selection.a)})`
          : "Drag across the timeline to select"}
      </span>
      <div className="trim-bar__actions">
        <Button size="xs" leftSection={<IconCut size={14} />} disabled={!selection} onClick={onCut} title="Delete">
          Cut selection
        </Button>
        <Button size="xs" variant="default" leftSection={<IconScissors size={14} />} disabled={!selection} onClick={onKeep}>
          Keep only selection
        </Button>
        <Button size="xs" variant="subtle" leftSection={<IconRestore size={14} />} disabled={!trimmed} onClick={onReset}>
          Restore full clip
        </Button>
      </div>
    </div>
  );
}
