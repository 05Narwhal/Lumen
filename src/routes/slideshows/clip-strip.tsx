import {
  IconArrowBarDown,
  IconArrowBarLeft,
  IconArrowBarRight,
  IconArrowBarToRight,
  IconArrowBarUp,
  IconArrowsHorizontal,
  IconArrowsVertical,
  IconArrowsMaximize,
  IconArrowsMinimize,
  IconCircle,
  IconCircleDot,
  IconCircleOff,
  IconLayoutColumns,
  IconLayoutRows,
  IconMoon,
  IconSquare,
  IconSun,
  IconZoomIn,
} from "@tabler/icons-react";
import { useEffect, useRef } from "react";
import type { SlideshowDoc } from "@/types/generated/SlideshowDoc";
import type { FrameKind } from "@/types/generated/FrameKind";
import type { TransitionKind } from "@/types/generated/TransitionKind";
import { mediaUrl } from "@/utils/media";

/** The strip's scale: a clip's width is its length, so trimming is a real distance. */
export const PX_PER_SECOND = 48;

/** The badge on a clip that is not a plain fill, so the strip reads at a glance. */
const FRAME_LABEL: Record<FrameKind, string> = {
  fill: "",
  blur: "blur",
  grid: "grid",
  quad: "four up",
  feature: "feature",
  stack: "stacked",
};

const TRANSITION_ICON: Record<TransitionKind, typeof IconSquare> = {
  none: IconSquare,
  crossfade: IconCircleDot,
  fadeblack: IconMoon,
  fadewhite: IconSun,
  slide: IconArrowsHorizontal,
  slideup: IconArrowsVertical,
  wipe: IconArrowBarRight,
  wipeup: IconArrowBarUp,
  circle: IconCircle,
  zoom: IconZoomIn,
  slidedown: IconArrowBarDown,
  cover: IconArrowBarToRight,
  reveal: IconArrowBarLeft,
  circleclose: IconCircleOff,
  vertopen: IconArrowsMaximize,
  horzopen: IconArrowsMinimize,
  corner: IconArrowBarUp,
  squeeze: IconSquare,
  slices: IconLayoutColumns,
  vslices: IconLayoutRows,
};

type Props = {
  doc: SlideshowDoc;
  selected: number | null;
  onSelect: (index: number | null) => void;
  onMove: (from: number, to: number) => void;
  onTrim: (index: number, deltaMs: number) => void;
  /** Where each clip starts, so clicking one moves the playhead there. */
  starts: number[];
  onSeek: (ms: number) => void;
};

/**
 * The clips in order: thumbnails as wide as they are long, the transition between each
 * pair, a drag to reorder and a right edge to drag for trimming.
 */
export function ClipStrip({ doc, selected, onSelect, onMove, onTrim, starts, onSeek }: Props) {
  const dragging = useRef<number | null>(null);
  const trimming = useRef<{ index: number; lastX: number } | null>(null);

  // Trimming follows the pointer on the window, so the drag survives leaving the handle.
  useEffect(() => {
    const move = (e: PointerEvent) => {
      const t = trimming.current;
      if (!t) return;
      onTrim(t.index, ((e.clientX - t.lastX) / PX_PER_SECOND) * 1000);
      t.lastX = e.clientX;
    };
    const up = () => {
      trimming.current = null;
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
  }, [onTrim]);

  return (
    <ol className="clip-strip" aria-label="Clips">
      {doc.clips.map((clip, i) => {
        const Icon = TRANSITION_ICON[clip.transition.kind];
        const last = i === doc.clips.length - 1;
        return (
          <li key={`${clip.asset_id}-${i}`} className="clip-strip__item" style={{ width: (clip.duration_ms / 1000) * PX_PER_SECOND }}>
            <button
              type="button"
              className="clip-strip__clip"
              aria-label={`Clip ${i + 1}`}
              aria-pressed={selected === i}
              draggable
              onDragStart={() => {
                dragging.current = i;
              }}
              onDragOver={(e) => e.preventDefault()}
              onDrop={() => {
                const from = dragging.current;
                dragging.current = null;
                if (from !== null && from !== i) onMove(from, i);
              }}
              onClick={() => {
                onSelect(i);
                onSeek(starts[i]);
              }}
            >
              <span className="clip-strip__photos" data-count={1 + clip.extra.length}>
                {[clip.asset_id, ...clip.extra].map((assetId, n) => (
                  <img key={`${assetId}-${n}`} src={mediaUrl("thumb", assetId, { s: 256 })} alt="" loading="lazy" />
                ))}
              </span>
              <span className="clip-strip__length">{(clip.duration_ms / 1000).toFixed(1)}s</span>
              {clip.frame !== "fill" && <span className="clip-strip__frame">{FRAME_LABEL[clip.frame]}</span>}
            </button>
            <div
              role="separator"
              aria-label={`Trim clip ${i + 1}`}
              className="clip-strip__trim"
              onPointerDown={(e) => {
                e.preventDefault();
                trimming.current = { index: i, lastX: e.clientX };
              }}
            />
            {!last && (
              <span className="clip-strip__transition" title={`${clip.transition.kind} transition`}>
                <Icon size={13} />
              </span>
            )}
          </li>
        );
      })}
    </ol>
  );
}
