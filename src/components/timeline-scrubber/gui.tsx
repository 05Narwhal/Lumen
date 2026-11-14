import { useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent } from "react";
import type { GroupBy } from "@/types/generated/GroupBy";
import type { Section } from "@/types/generated/Section";
import { buildMarks, dateAt, offsetForPointer, thumbFor, visibleLabels } from "./marks";
import "./styles.scss";

export interface TimelineScrubberProps {
  /** The element that scrolls: the grid. */
  scroller: HTMLElement | null;
  sections: Section[];
  /** The content's height in px. */
  total: number;
  /** How the grid is grouped; null for a grid with no dates (an album in manual order). */
  grouping: GroupBy | null;
  onScrollTo: (offset: number) => void;
  /** A drag started or ended: the grid holds new tiles' images until it ends. */
  onDraggingChange?: (dragging: boolean) => void;
}

/**
 * The Library's scrollbar (Round 4H, Google Photos style): a track along the right edge with
 * the years beside it, a thumb that follows the grid, and a bubble with the date under the
 * pointer. Dragging anywhere on the track scrolls the grid there; a year jumps to its start.
 */
export function TimelineScrubber({ scroller, sections, total, grouping, onScrollTo, onDraggingChange }: TimelineScrubberProps) {
  const track = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(scroller?.scrollTop ?? 0);
  const [viewport, setViewport] = useState(scroller?.clientHeight ?? 0);
  // What the grid really scrolls: its rows plus the padding under the last one.
  const [scrollHeight, setScrollHeight] = useState(scroller?.scrollHeight ?? 0);
  const [trackPx, setTrackPx] = useState(0);
  const [hoverY, setHoverY] = useState<number | null>(null);
  const [dragging, setDragging] = useState(false);

  // Follow the grid's own scrolling, one state write per event (the browser sends one a frame).
  useEffect(() => {
    if (!scroller) return;
    const on = () => setScrollTop(scroller.scrollTop);
    on();
    scroller.addEventListener("scroll", on, { passive: true });
    return () => scroller.removeEventListener("scroll", on);
  }, [scroller]);

  // The view's height and the track's, as the window changes size.
  useLayoutEffect(() => {
    const measure = () => {
      if (scroller) {
        setViewport(scroller.clientHeight);
        setScrollHeight(scroller.scrollHeight);
      }
      if (track.current) setTrackPx(track.current.getBoundingClientRect().height);
    };
    measure();
    if (typeof ResizeObserver === "undefined" || !scroller) return;
    const ro = new ResizeObserver(measure);
    ro.observe(scroller);
    return () => ro.disconnect();
  }, [scroller, total]);

  const content = Math.max(total, scrollHeight);
  const range = Math.max(1, content - viewport);
  // Marks are placed on the scroll range, so the thumb sits on a year's label when that year
  // starts at the top of the view.
  const marks = useMemo(() => buildMarks(sections, range, grouping).map((m) => ({ ...m, at: Math.min(1, m.at) })), [sections, range, grouping]);
  const labels = useMemo(() => visibleLabels(marks, trackPx), [marks, trackPx]);
  const months = useMemo(() => marks.filter((m) => m.kind === "month"), [marks]);

  if (content <= viewport) return null;

  const pointerY = (e: PointerEvent) => {
    const r = track.current?.getBoundingClientRect();
    return r ? { y: e.clientY - r.top, h: r.height } : { y: 0, h: 0 };
  };
  const scrollToPointer = (e: PointerEvent) => {
    const { y, h } = pointerY(e);
    onScrollTo(offsetForPointer(y, h, content, viewport));
  };
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture?.(e.pointerId);
    setDragging(true);
    onDraggingChange?.(true);
    scrollToPointer(e);
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    setHoverY(pointerY(e).y);
    if (dragging) scrollToPointer(e);
  };
  const endDrag = (e: PointerEvent<HTMLDivElement>) => {
    e.currentTarget.releasePointerCapture?.(e.pointerId);
    if (dragging) onDraggingChange?.(false);
    setDragging(false);
  };

  const thumbY = thumbFor(scrollTop, viewport, content, trackPx);
  const now = dateAt(sections, scrollTop, grouping);
  // While dragging the bubble rides the thumb; while hovering it shows where a click would go.
  const bubbleY = dragging ? thumbY : hoverY;
  const bubbleText = bubbleY === null ? "" : dateAt(sections, dragging ? scrollTop : offsetForPointer(bubbleY, trackPx, content, viewport), grouping);
  const jumpTo = (label: string) => {
    const s = sections.find((x) => String(x.year) === label);
    if (s) onScrollTo(s.y);
  };

  return (
    <div
      ref={track}
      className="timeline-scrubber"
      role="scrollbar"
      aria-orientation="vertical"
      aria-label="Scroll through the timeline"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round((Math.min(scrollTop, range) / range) * 100)}
      aria-valuetext={now || undefined}
      data-dragging={dragging}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onPointerLeave={() => setHoverY(null)}
    >
      <div className="timeline-scrubber__line" aria-hidden />
      {months.map((m) => (
        <span key={m.label} className="timeline-scrubber__tick" style={{ top: `${m.at * 100}%` }} aria-hidden />
      ))}
      {labels.map((m) => (
        <button
          key={m.label}
          type="button"
          className="timeline-scrubber__year"
          // Seen only on hover; the grid's own keys (Page Up/Down, Home, End) scroll by keyboard.
          tabIndex={-1}
          style={{ top: `${m.at * 100}%` }}
          onPointerDown={(e) => e.stopPropagation()}
          onClick={() => jumpTo(m.label)}
        >
          {m.label}
        </button>
      ))}
      <div className="timeline-scrubber__thumb" style={{ transform: `translateY(${thumbY}px)` }} aria-hidden />
      {bubbleText && (
        <div className="timeline-scrubber__bubble" style={{ top: bubbleY ?? 0 }}>
          {bubbleText}
        </div>
      )}
    </div>
  );
}
