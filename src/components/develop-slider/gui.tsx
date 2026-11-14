import { Slider } from "@mantine/core";
import { useState, type ReactNode } from "react";
import { lerpNumber, useTween } from "@/hooks/use-tween";
import { holdMaskActive } from "@/lib/develop-masks";
import "./styles.scss";

/** How long a slider takes to reach another photo's value. */
const GLIDE_MS = 260;

export interface DevelopSliderProps {
  label: string;
  /** How a change is named in History, when the label alone does not say ("Red hue"). */
  historyLabel?: string;
  /** In the slider's own units (what the value column shows). */
  value: number;
  min: number;
  max: number;
  step?: number;
  /** Where double-click returns to. */
  neutral?: number;
  format: (v: number) => string;
  /** While dragging: the preview follows, nothing is recorded. */
  onChange: (v: number) => void;
  /** On release (and on reset, keys, clicks): record the change as `label` + the value. */
  onCommit: (v: number, label: string) => void;
  /** A CSS background for the track, from a token (`var(--track-temperature)`). */
  track?: string;
  /** Beside the label: a colour swatch, say. */
  adornment?: ReactNode;
}

/**
 * One adjustment: label, track, value, in a 28 px row. Double-click anywhere on the row puts
 * it back to neutral. The track runs both ways from neutral when neutral is inside the range.
 */
export function DevelopSlider({ label, historyLabel, value, min, max, step = 1, neutral = 0, format, onChange, onCommit, track, adornment }: DevelopSliderProps) {
  // Opening another photo glides the slider to its value; a drag or a key follows at once.
  const [handling, setHandling] = useState(false);
  const shown = useTween(value, lerpNumber, GLIDE_MS, handling);
  const bipolar = neutral > min && neutral < max;
  const changed = Math.abs(value - neutral) > step / 2;
  const named = historyLabel ?? label;
  return (
    <div
      className="develop-slider"
      data-changed={changed}
      onDoubleClick={() => {
        onChange(neutral);
        onCommit(neutral, `${named} reset`);
      }}
      title="Double-click to reset"
      onPointerDownCapture={() => {
        setHandling(true);
        // The mask tint steps aside while a slider is dragged (Round 5).
        holdMaskActive();
      }}
      onKeyDownCapture={() => setHandling(true)}
    >
      <span className="develop-slider__label">
        {adornment}
        {label}
      </span>
      <Slider
        className="develop-slider__track"
        classNames={{ track: "develop-slider__rail", bar: bipolar || track ? "develop-slider__bar--hidden" : undefined }}
        style={track ? ({ "--develop-track": track } as React.CSSProperties) : undefined}
        data-painted={!!track}
        value={shown}
        min={min}
        max={max}
        step={step}
        size="xs"
        label={null}
        thumbLabel={named}
        marks={bipolar ? [{ value: neutral }] : undefined}
        onChange={onChange}
        onChangeEnd={(v) => {
          setHandling(false);
          onCommit(v, `${named} ${format(v)}`);
        }}
      />
      <span className="develop-slider__value">{format(shown)}</span>
    </div>
  );
}
