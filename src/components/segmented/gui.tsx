import { motion } from "motion/react";
import "./styles.scss";

export interface SegmentedOption<T extends string> {
  label: string;
  value: T;
  /** Shown but not choosable; `title` says why. */
  disabled?: boolean;
  title?: string;
}

export interface SegmentedProps<T extends string> {
  value: T;
  onChange: (value: T) => void;
  data: SegmentedOption<T>[];
  /** Announced as the group's name; each option announces its own label. */
  label: string;
  size?: "xs" | "sm";
}

const SPRING = { type: "spring", stiffness: 420, damping: 38, mass: 0.9 } as const;

/**
 * A segmented control whose selection travels, the same way the sidebar's indicator and
 * the library's filter chips do.
 *
 * Mantine's SegmentedControl positions its indicator from measured rects written into an
 * inline style, and re-measures a frame after the value changes. When the surrounding
 * screen re-renders in those same frames — which it does here, because the value is
 * persisted through an async settings round trip — that second write lands on top of the
 * transitions it had just started and cancels them, so the pill jumps. Measured in the
 * real webview: `transitionrun:transform` on frame 0, `transitioncancel:transform` on
 * frame 1. A shared-layout animation re-projects across re-renders instead of cancelling,
 * so it survives whatever else is rendering.
 */
export function Segmented<T extends string>({ value, onChange, data, label, size = "sm" }: SegmentedProps<T>) {
  return (
    <div className="segmented" role="radiogroup" aria-label={label} data-size={size}>
      {data.map((o) => {
        const selected = o.value === value;
        return (
          <button
            key={o.value}
            type="button"
            role="radio"
            aria-checked={selected}
            aria-label={o.label}
            className="segmented__option"
            disabled={o.disabled}
            title={o.title}
            onClick={() => onChange(o.value)}
          >
            {selected && (
              <motion.span layoutId={`segmented-${label}`} className="segmented__pill" aria-hidden transition={SPRING} />
            )}
            <span className="segmented__label">{o.label}</span>
          </button>
        );
      })}
    </div>
  );
}
