import { Slider, Tooltip } from "@mantine/core";
import { IconCrop, IconFlipHorizontal, IconFlipVertical, IconGrid3x3, IconRotate2, IconRotateClockwise2 } from "@tabler/icons-react";
import type { ReactNode } from "react";
import type { EditSession } from "@/hooks/use-edit-session";
import { ASPECTS, flipShown, rotateBy, type GridMode } from "@/components/crop-tool";

export type AspectChoice = (typeof ASPECTS)[number]["value"];

const GRID_LABELS: Record<GridMode, string> = {
  auto: "Auto",
  thirds: "Thirds",
  fine: "Grid",
  off: "Off",
};
export const GRID_ORDER: GridMode[] = ["auto", "thirds", "fine", "off"];

function Tool({
  label,
  keys,
  pressed,
  onClick,
  children,
  wide,
}: {
  label: string;
  keys?: string;
  pressed?: boolean;
  onClick: () => void;
  children: ReactNode;
  wide?: boolean;
}) {
  return (
    <Tooltip label={keys ? `${label} (${keys})` : label} openDelay={500} withArrow>
      <button type="button" className="edit-toolbar__tool" data-wide={wide || undefined} aria-label={label} aria-pressed={pressed} onClick={onClick}>
        {children}
      </button>
    </Tooltip>
  );
}

/**
 * The quick editor's controls, floating over the bottom of the picture: crop and its shapes,
 * quarter turns, flips of what is on screen, straighten (drag, or type an exact angle) and
 * the grid. Every change goes into the session's draft; nothing is stored until *Done*.
 */
export function EditToolbar({
  session,
  cropping,
  onCropping,
  aspect,
  onAspect,
  grid,
  onGrid,
  onStraightening,
  cropSize,
}: {
  session: EditSession;
  cropping: boolean;
  onCropping: (on: boolean) => void;
  aspect: AspectChoice;
  onAspect: (a: AspectChoice) => void;
  grid: GridMode;
  onGrid: (g: GridMode) => void;
  /** True while the straighten slider is held, so the canvas shows the fine grid. */
  onStraightening: (on: boolean) => void;
  /** The crop in the original's pixels, shown while cropping. */
  cropSize: { width: number; height: number } | null;
}) {
  const { draft, set } = session;
  const angle = draft.straighten;
  const setAngle = (v: number) => set({ straighten: Math.max(-45, Math.min(45, Math.round(v * 10) / 10)) }, "straighten");
  return (
    <div className="edit-toolbar" role="toolbar" aria-label="Edit tools">
      {cropping && (
        <div className="edit-toolbar__row edit-toolbar__row--crop">
          <div className="edit-toolbar__chips" role="radiogroup" aria-label="Crop shape">
            {ASPECTS.map((a) => (
              <button
                key={a.label}
                type="button"
                role="radio"
                className="edit-toolbar__chip"
                aria-checked={a.value === aspect}
                onClick={() => onAspect(a.value)}
              >
                {a.label}
              </button>
            ))}
          </div>
          {cropSize && (
            <span className="edit-toolbar__size" aria-label="Crop size">
              {cropSize.width} × {cropSize.height}
            </span>
          )}
          <button type="button" className="edit-toolbar__text" onClick={() => set({ crop: null })} disabled={!draft.crop}>
            Reset crop
          </button>
        </div>
      )}
      <div className="edit-toolbar__row">
        <Tool label={cropping ? "Done cropping" : "Crop"} keys="C" pressed={cropping} onClick={() => onCropping(!cropping)} wide>
          <IconCrop size={18} aria-hidden />
          <span>{cropping ? "Done" : "Crop"}</span>
        </Tool>
        <span className="edit-toolbar__rule" aria-hidden />
        <Tool label="Rotate left" keys="[" onClick={() => set((d) => rotateBy(d, -1))}>
          <IconRotate2 size={18} aria-hidden />
        </Tool>
        <Tool label="Rotate right" keys="]" onClick={() => set((d) => rotateBy(d, 1))}>
          <IconRotateClockwise2 size={18} aria-hidden />
        </Tool>
        {/* Tabler names its icons by the mirror line: a vertical line flips left to right. */}
        <Tool label="Flip horizontal" keys="H" onClick={() => set((d) => flipShown(d, "h"))}>
          <IconFlipVertical size={18} aria-hidden />
        </Tool>
        <Tool label="Flip vertical" keys="V" onClick={() => set((d) => flipShown(d, "v"))}>
          <IconFlipHorizontal size={18} aria-hidden />
        </Tool>
        <span className="edit-toolbar__rule" aria-hidden />
        <div className="edit-toolbar__straighten" onDoubleClick={() => session.reset("straighten")} title="Double-click to level">
          <span className="edit-toolbar__label">Straighten</span>
          <Slider
            className="edit-toolbar__slider"
            value={angle}
            min={-45}
            max={45}
            step={0.1}
            label={null}
            size="sm"
            classNames={{ bar: "edit-toolbar__bar" }}
            marks={[{ value: 0 }]}
            aria-label="Straighten"
            onChange={(v) => {
              onStraightening(true);
              setAngle(v);
            }}
            onChangeEnd={() => onStraightening(false)}
          />
          <label className="edit-toolbar__angle" data-changed={angle !== 0}>
            <input
              type="number"
              aria-label="Straighten angle in degrees"
              min={-45}
              max={45}
              step={0.1}
              value={angle}
              onChange={(e) => {
                const v = Number(e.currentTarget.value);
                if (Number.isFinite(v)) setAngle(v);
              }}
              onFocus={() => onStraightening(true)}
              onBlur={() => onStraightening(false)}
            />
            <span aria-hidden>°</span>
          </label>
        </div>
        <span className="edit-toolbar__rule" aria-hidden />
        <Tool
          label={`Grid: ${GRID_LABELS[grid]}`}
          keys="G"
          pressed={grid !== "auto" && grid !== "off"}
          onClick={() => onGrid(GRID_ORDER[(GRID_ORDER.indexOf(grid) + 1) % GRID_ORDER.length])}
          wide
        >
          <IconGrid3x3 size={18} aria-hidden />
          <span>{GRID_LABELS[grid]}</span>
        </Tool>
      </div>
    </div>
  );
}
