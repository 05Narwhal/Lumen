import { type KeyboardEvent, useEffect, useRef } from "react";
import "./styles.scss";

export interface InlineNameProps {
  value: string;
  /** Shows the field in place of the name. */
  editing: boolean;
  /** Enter or blur with a non-empty, trimmed name. */
  onCommit: (name: string) => void;
  /** Esc, or an empty name. */
  onCancel: () => void;
  className?: string;
}

/**
 * A name that turns into a text field in place (Round 24 §2): it takes the name's font and
 * width, selects its text on open, saves on Enter or blur, cancels on Esc. Its keys never reach
 * Develop's shortcuts, and its pointer events never reach a row's drag or click.
 */
export function InlineName({ value, editing, onCommit, onCancel, className }: InlineNameProps) {
  const ref = useRef<HTMLInputElement>(null);
  // Enter or Esc ends the edit; the blur that follows the unmount must not end it twice.
  const done = useRef(false);

  useEffect(() => {
    if (!editing) return;
    done.current = false;
    ref.current?.focus();
    ref.current?.select();
  }, [editing]);

  if (!editing) return <span className={className}>{value}</span>;

  const finish = (text: string) => {
    if (done.current) return;
    done.current = true;
    const name = text.trim();
    if (name) onCommit(name);
    else onCancel();
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    e.stopPropagation();
    if (e.key === "Enter") {
      e.preventDefault();
      finish(e.currentTarget.value);
    } else if (e.key === "Escape") {
      e.preventDefault();
      if (done.current) return;
      done.current = true;
      onCancel();
    }
  };

  return (
    <input
      ref={ref}
      className={`inline-name ${className ?? ""}`}
      defaultValue={value}
      aria-label="Name"
      spellCheck={false}
      maxLength={80}
      onKeyDown={onKeyDown}
      onKeyUp={(e) => e.stopPropagation()}
      onBlur={(e) => finish(e.currentTarget.value)}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onPointerDown={(e) => e.stopPropagation()}
    />
  );
}
