import { AnimatePresence, motion } from "framer-motion";
import { useEffect, useState, type CSSProperties, type ReactNode } from "react";
import "./styles.scss";

/** `total` 0 means unknown: a shimmer instead of a ring. `visible` flips after the delay. */
export interface AiButtonState {
  visible: boolean;
  stage: string | null;
  done: number;
  total: number;
}

export interface AiButtonProps {
  label: string;
  /** null = idle; anything else = a task is running (visible or still inside the delay). */
  state: AiButtonState | null;
  onRun: () => void;
  onCancel: () => void;
  /** false = this action cannot be stopped: no Cancel on hover, and a click while it runs does nothing. */
  cancellable?: boolean;
  disabled?: boolean;
  icon?: ReactNode;
  title?: string;
}

/**
 * The button that starts an AI action is also its progress (Round 12 spec §5): after the
 * delay its label crossfades to the stage, a ring traces its border (a shimmer when the total
 * is unknown), hover or focus offers Cancel, and a click while a task runs is always Cancel.
 */
export function AiButton({ label, state, onRun, onCancel, cancellable = true, disabled, icon, title }: AiButtonProps) {
  const [engaged, setEngaged] = useState(false);
  const running = state !== null;
  const busy = state?.visible === true;
  const known = busy && state.total > 0;
  // The pointer that pressed the button is still over it when the stage appears; Cancel is
  // offered on the next hover or focus, so the stage gets read first.
  useEffect(() => {
    if (busy) setEngaged(false);
  }, [busy]);
  const offerCancel = busy && engaged && cancellable;
  const text = !busy ? label : offerCancel ? "Cancel" : (state.stage ?? label);
  const mode = !busy ? "idle" : offerCancel ? "cancel" : "stage";
  const style = known ? ({ "--ai-progress": String(Math.round((state.done / state.total) * 1000) / 1000) } as CSSProperties) : undefined;
  return (
    <button
      type="button"
      className="ai-button"
      title={title}
      disabled={disabled}
      style={style}
      aria-busy={busy || undefined}
      data-busy={busy ? "" : undefined}
      data-indeterminate={busy && !known ? "" : undefined}
      onClick={running ? (cancellable ? onCancel : undefined) : onRun}
      onMouseEnter={() => setEngaged(true)}
      onMouseLeave={() => setEngaged(false)}
      onFocus={() => setEngaged(true)}
      onBlur={() => setEngaged(false)}
    >
      {icon ? <span className="ai-button__icon">{icon}</span> : null}
      <span className="ai-button__box">
        {/* The idle label, invisible, holds the box's width; the live label floats over it. */}
        <span className="ai-button__twin" aria-hidden>
          {label}
        </span>
        <span className="ai-button__live" aria-live="polite">
          <AnimatePresence mode="popLayout" initial={false}>
            <motion.span
              key={mode}
              className="ai-button__text"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.12 }}
            >
              {text}
            </motion.span>
          </AnimatePresence>
        </span>
      </span>
    </button>
  );
}
