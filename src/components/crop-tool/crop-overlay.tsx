import { useRef, type KeyboardEvent, type PointerEvent } from "react";
import { dragCrop, type Handle, type Rect } from "./crop-math";

const HANDLES: { handle: Exclude<Handle, "move">; label: string }[] = [
  { handle: "nw", label: "Crop top-left corner" },
  { handle: "n", label: "Crop top edge" },
  { handle: "ne", label: "Crop top-right corner" },
  { handle: "e", label: "Crop right edge" },
  { handle: "se", label: "Crop bottom-right corner" },
  { handle: "s", label: "Crop bottom edge" },
  { handle: "sw", label: "Crop bottom-left corner" },
  { handle: "w", label: "Crop left edge" },
];

/** One percent of the frame per arrow key press. */
const NUDGE = 0.01;

/**
 * The crop rectangle over the uncropped preview: everything outside it dimmed, thirds
 * inside, eight handles and the body to move it. Works in frame units; the overlay's own
 * size turns pointer pixels into them.
 */
export function CropOverlay({ rect, aspect, frameAspect, onChange }: { rect: Rect; aspect: number | null; frameAspect: number; onChange: (r: Rect) => void }) {
  const layer = useRef<HTMLDivElement>(null);
  const drag = useRef<{ handle: Handle; start: Rect; x: number; y: number } | null>(null);

  const down = (handle: Handle) => (e: PointerEvent<HTMLElement>) => {
    e.preventDefault();
    e.stopPropagation();
    e.currentTarget.setPointerCapture?.(e.pointerId);
    drag.current = { handle, start: rect, x: e.clientX, y: e.clientY };
  };
  const move = (e: PointerEvent<HTMLElement>) => {
    const d = drag.current;
    const box = layer.current?.getBoundingClientRect();
    if (!d || !box || box.width === 0 || box.height === 0) return;
    onChange(dragCrop(d.start, d.handle, (e.clientX - d.x) / box.width, (e.clientY - d.y) / box.height, aspect, frameAspect));
  };
  const up = () => {
    drag.current = null;
  };
  const nudge = (handle: Handle) => (e: KeyboardEvent) => {
    const step = { ArrowLeft: [-NUDGE, 0], ArrowRight: [NUDGE, 0], ArrowUp: [0, -NUDGE], ArrowDown: [0, NUDGE] }[e.key];
    if (!step) return;
    e.preventDefault();
    e.stopPropagation();
    onChange(dragCrop(rect, handle, step[0], step[1], aspect, frameAspect));
  };

  const pct = (v: number) => `${v * 100}%`;
  return (
    <div className="crop-overlay" ref={layer} onPointerMove={move} onPointerUp={up} onPointerCancel={up}>
      <div
        className="crop-overlay__rect"
        style={{ left: pct(rect.x), top: pct(rect.y), width: pct(rect.w), height: pct(rect.h) }}
        onPointerDown={down("move")}
        onKeyDown={nudge("move")}
        tabIndex={0}
        role="group"
        aria-label="Crop area; drag to move"
      >
        <span className="crop-overlay__thirds" aria-hidden />
        {HANDLES.map(({ handle, label }) => (
          <button key={handle} type="button" className="crop-overlay__handle" data-handle={handle} aria-label={label} onPointerDown={down(handle)} onKeyDown={nudge(handle)} />
        ))}
      </div>
    </div>
  );
}
