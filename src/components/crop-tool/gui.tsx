import { useEffect, useState } from "react";
import { useElementSize } from "@/hooks/use-element-size";
import type { Recipe } from "@/types/generated/Recipe";
import type { Rect } from "./crop-math";
import { CropOverlay } from "./crop-overlay";
import { editLayout } from "./edit-geometry";
import "./styles.scss";

const FULL: Rect = { x: 0, y: 0, w: 1, h: 1 };

export type GridMode = "auto" | "thirds" | "fine" | "off";

/** Which grid is drawn now. *Auto* shows a fine grid while straightening and none otherwise
 * (the crop rectangle carries its own thirds). */
export function shownGrid(mode: GridMode, cropping: boolean, straightening: boolean): "thirds" | "fine" | null {
  if (mode === "off") return null;
  if (mode === "auto") return straightening ? "fine" : null;
  if (mode === "thirds" && cropping) return null;
  return mode;
}

/**
 * The quick editor's picture. Rust renders it once without geometry; the turns, flips,
 * straighten and crop are CSS transforms laid out by `editLayout`, so each change animates
 * (and costs no render). Outside crop mode only the crop is visible: the frame slides and
 * scales behind a viewport of the crop's shape, which is what makes *Done cropping* a zoom.
 */
export function EditCanvas({
  src,
  alt,
  initialAspect,
  recipe,
  spin,
  cropping,
  aspect,
  grid,
  live,
  onCrop,
  onFrameAspect,
}: {
  src: string | null;
  alt: string;
  initialAspect: number;
  recipe: Recipe;
  spin: number;
  cropping: boolean;
  /** The locked crop shape, or null for free. */
  aspect: number | null;
  grid: "thirds" | "fine" | null;
  /** A control is being dragged: follow it without easing. */
  live: boolean;
  onCrop: (r: Rect) => void;
  onFrameAspect?: (a: number) => void;
}) {
  const [ref, stage] = useElementSize<HTMLDivElement>(0);
  const [imageAspect, setImageAspect] = useState(initialAspect);
  const [ready, setReady] = useState(false);
  const layout = editLayout({ imageAspect, recipe, spin, cropping, stage });

  useEffect(() => onFrameAspect?.(layout.frameAspect), [layout.frameAspect, onFrameAspect]);
  // Ease only once the first layout has been painted, or the picture would grow in from 0.
  useEffect(() => {
    if (ready || stage.width === 0 || !src) return;
    const id = requestAnimationFrame(() => requestAnimationFrame(() => setReady(true)));
    return () => cancelAnimationFrame(id);
  }, [ready, stage.width, src]);

  return (
    <div className="edit-canvas" ref={ref}>
      <div className="edit-canvas__viewport" data-animate={ready && !live} data-cropping={cropping} style={layout.viewport}>
        <div className="edit-canvas__clip">
          <div className="edit-canvas__frame" style={layout.frame}>
            {src && (
              <img
                className="edit-canvas__img"
                src={src}
                alt={alt}
                draggable={false}
                style={{ ...layout.image, transform: layout.transform }}
                onLoad={(e) => {
                  const img = e.currentTarget;
                  if (img.naturalWidth && img.naturalHeight) setImageAspect(img.naturalWidth / img.naturalHeight);
                }}
              />
            )}
          </div>
        </div>
        {/* Outside the clip, so the handles on the edge are not cut in half. While cropping
            the frame is the viewport, so the overlay's percentages are the frame's. */}
        {cropping && <CropOverlay rect={recipe.crop ?? FULL} aspect={aspect} frameAspect={layout.frameAspect} onChange={onCrop} />}
        {grid && <span className="edit-canvas__grid" data-grid={grid} aria-hidden />}
      </div>
    </div>
  );
}
