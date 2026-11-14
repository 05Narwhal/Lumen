import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
  type PointerEvent,
  type ReactNode,
  type WheelEvent,
} from "react";
import { clampPan, clampScale, MIN_SCALE, oneToOneScale, zoomAround, type Size, type Transform } from "./zoom-math";

export interface ZoomPanHandle {
  zoomTo: (target: "fit" | "1:1") => void;
  isZoomed: () => boolean;
}

interface Props {
  src: string;
  alt: string;
  /** Natural pixel size, for the true 1:1 zoom. */
  natural?: Size | null;
  onScaleChange?: (scale: number) => void;
  onLoad?: () => void;
  /** Drawn over the picture, in its box and under its zoom (the face boxes, Round 4I). */
  overlay?: ReactNode;
}

/**
 * Wheel zoom around the cursor, drag to pan when zoomed, double-click fit ↔ 1:1.
 * One GPU-composited transform, bounded so the image never leaves the box.
 */
export const ZoomPan = forwardRef<ZoomPanHandle, Props>(function ZoomPan({ src, alt, natural, onScaleChange, onLoad, overlay }, ref) {
  const box = useRef<HTMLDivElement>(null);
  const img = useRef<HTMLImageElement>(null);
  const [t, setT] = useState<Transform>({ scale: 1, x: 0, y: 0 });
  const drag = useRef<{ x: number; y: number; ox: number; oy: number } | null>(null);
  const [dragging, setDragging] = useState(false);
  // Where the fitted picture sits in the box, so an overlay can lie exactly on it.
  const [placed, setPlaced] = useState<{ left: number; top: number; width: number; height: number } | null>(null);
  const measure = useCallback(() => {
    const i = img.current;
    if (!i || !i.clientWidth) return;
    setPlaced({ left: i.offsetLeft, top: i.offsetTop, width: i.clientWidth, height: i.clientHeight });
  }, []);
  useEffect(() => {
    if (!overlay || !box.current || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(box.current);
    return () => ro.disconnect();
  }, [overlay, measure]);

  // No reset when `src` changes. The viewer remounts this per asset (it is keyed by id), and
  // within one asset a new `src` is the sharper version replacing the one on screen — which
  // happens *because* the user zoomed. Resetting here threw that zoom away.
  useEffect(() => onScaleChange?.(t.scale), [t.scale, onScaleChange]);

  const sizes = useCallback(() => {
    const b = box.current;
    const i = img.current;
    const boxSize = { width: b?.clientWidth ?? 1, height: b?.clientHeight ?? 1 };
    const fitted = { width: i?.clientWidth || boxSize.width, height: i?.clientHeight || boxSize.height };
    return { boxSize, fitted, one: oneToOneScale(natural ?? null, fitted) };
  }, [natural]);

  const onWheel = (e: WheelEvent) => {
    e.preventDefault();
    const r = box.current!.getBoundingClientRect();
    const { boxSize, fitted, one } = sizes();
    const cx = e.clientX - r.left - r.width / 2;
    const cy = e.clientY - r.top - r.height / 2;
    setT((p) => zoomAround(p, Math.exp(-e.deltaY * 0.0022), cx, cy, boxSize, fitted, one));
  };

  const zoomTo = useCallback(
    (target: "fit" | "1:1") => {
      const { one } = sizes();
      setT(target === "fit" ? { scale: MIN_SCALE, x: 0, y: 0 } : { scale: clampScale(one, one), x: 0, y: 0 });
    },
    [sizes],
  );

  useImperativeHandle(ref, () => ({ zoomTo, isZoomed: () => t.scale > 1.01 }), [zoomTo, t.scale]);

  const onDouble = () => (t.scale > 1.01 ? zoomTo("fit") : zoomTo("1:1"));

  const onDown = (e: PointerEvent) => {
    if (t.scale <= 1) return;
    (e.target as Element).setPointerCapture(e.pointerId);
    drag.current = { x: e.clientX, y: e.clientY, ox: t.x, oy: t.y };
    setDragging(true);
  };
  const onMove = (e: PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const { boxSize, fitted } = sizes();
    setT((p) => clampPan({ ...p, x: d.ox + (e.clientX - d.x), y: d.oy + (e.clientY - d.y) }, boxSize, fitted));
  };
  const onUp = () => {
    drag.current = null;
    setDragging(false);
  };

  return (
    <div
      ref={box}
      className="zoom-pan"
      data-zoomed={t.scale > 1.01}
      data-dragging={dragging}
      onWheel={onWheel}
      onDoubleClick={onDouble}
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerCancel={onUp}
    >
      <img
        ref={img}
        className="zoom-pan__img"
        src={src}
        alt={alt}
        draggable={false}
        onLoad={() => {
          measure();
          onLoad?.();
        }}
        style={{ transform: `translate(${t.x}px, ${t.y}px) scale(${t.scale})` }}
      />
      {overlay && placed && (
        <div className="zoom-pan__overlay" style={{ ...placed, transform: `translate(${t.x}px, ${t.y}px) scale(${t.scale})`, ["--zoom" as string]: t.scale }}>
          {overlay}
        </div>
      )}
    </div>
  );
});
