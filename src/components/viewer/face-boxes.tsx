import { useRef, useState, type PointerEvent } from "react";
import { rectToOriginal, rectToShown, visible, type Rect } from "@/lib/face-geometry";
import type { Recipe } from "@/types/generated/Recipe";

/** A drawn box smaller than this share of the photo is a slip, not a face. */
const MIN_BOX = 0.01;

const pct = ([x, y, w, h]: Rect) => ({ left: `${x * 100}%`, top: `${y * 100}%`, width: `${w * 100}%`, height: `${h * 100}%` });

/** Anything boxed on a photo: a face, or a pet (Round 8B). */
export interface Boxed {
  id: number;
  name: string | null;
  x: number;
  y: number;
  w: number;
  h: number;
}

const faceLabel = (b: Boxed) => b.name ?? "Unknown";
const faceAria = (b: Boxed) => (b.name ? `${b.name}: change who this is` : "Unknown: tag this face");

/**
 * The faces in a photo drawn over it (Round 4I), in `ZoomPan`'s overlay. Boxes are stored on
 * the unedited photo and mapped through the photo's edit (`face-geometry`). With `show`
 * every box is drawn with its name under it, else only the `hovered` one. Clicking a box
 * picks that face. With `drawing` the cursor is a crosshair and a drag draws a new box,
 * handed back in the unedited photo's 0..1. Pets use the same layer (Round 8B), with their
 * own `label` and `ariaLabel` and the `kind` that styles them.
 */
export function FaceBoxes<T extends Boxed>({
  faces,
  recipe,
  aspect,
  show,
  hovered,
  drawing,
  onPick,
  onDrawn,
  onMenu,
  kind = "face",
  label = faceLabel,
  ariaLabel = faceAria,
}: {
  /** Right-click on a box (Round 4G): its menu. */
  onMenu?: (face: T, e: React.MouseEvent) => void;
  faces: T[];
  kind?: "face" | "pet";
  label?: (box: T) => string;
  ariaLabel?: (box: T) => string;
  recipe: Recipe | null | undefined;
  aspect: number;
  show: boolean;
  hovered: number | null;
  drawing: boolean;
  onPick: (face: T) => void;
  onDrawn: (rect: Rect) => void;
}) {
  const layer = useRef<HTMLDivElement>(null);
  const [start, setStart] = useState<[number, number] | null>(null);
  const [box, setBox] = useState<Rect | null>(null);

  const at = (e: PointerEvent): [number, number] => {
    const r = layer.current!.getBoundingClientRect();
    return [Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)), Math.min(1, Math.max(0, (e.clientY - r.top) / r.height))];
  };
  const span = (a: [number, number], b: [number, number]): Rect => [Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.abs(a[0] - b[0]), Math.abs(a[1] - b[1])];

  const onDown = (e: PointerEvent) => {
    // The zoom and pan underneath must not take this drag.
    e.stopPropagation();
    layer.current?.setPointerCapture?.(e.pointerId);
    const p = at(e);
    setStart(p);
    setBox([p[0], p[1], 0, 0]);
  };
  const onMove = (e: PointerEvent) => {
    if (start) setBox(span(start, at(e)));
  };
  const onUp = (e: PointerEvent) => {
    if (!start) return;
    const drawn = span(start, at(e));
    setStart(null);
    setBox(null);
    if (drawn[2] >= MIN_BOX && drawn[3] >= MIN_BOX) onDrawn(rectToOriginal(drawn, recipe, aspect));
  };

  return (
    <div className="face-boxes" data-kind={kind} data-show={show || undefined}>
      {faces.map((f) => {
        const shown = rectToShown([f.x, f.y, f.w, f.h], recipe, aspect);
        if (!visible(shown) || (!show && hovered !== f.id)) return null;
        return (
          <button
            key={f.id}
            type="button"
            className="face-boxes__box"
            data-hovered={hovered === f.id || undefined}
            data-unknown={f.name == null || undefined}
            style={pct(shown)}
            aria-label={ariaLabel(f)}
            onPointerDown={(e) => e.stopPropagation()}
            onDoubleClick={(e) => e.stopPropagation()}
            onClick={() => onPick(f)}
            onContextMenu={onMenu ? (e) => onMenu(f, e) : undefined}
          >
            <span className="face-boxes__name">{label(f)}</span>
          </button>
        );
      })}
      {drawing && (
        <div ref={layer} className="face-boxes__draw" data-testid="face-draw" onPointerDown={onDown} onPointerMove={onMove} onPointerUp={onUp} onDoubleClick={(e) => e.stopPropagation()}>
          {box && <span className="face-boxes__box face-boxes__box--drawing" style={pct(box)} />}
        </div>
      )}
    </div>
  );
}
