import { IconChevronRight } from "@tabler/icons-react";
import { useEffect, useRef, useState, type ReactNode } from "react";

/** A section of the left column: a quiet header that folds it, an action on the right. */
export function Section({ title, action, children }: { title: string; action?: ReactNode; children: ReactNode }) {
  const [open, setOpen] = useState(true);
  return (
    <section className="develop-sidebar__section" data-open={open} aria-label={title}>
      <div className="develop-sidebar__head">
        <button type="button" className="develop-sidebar__title" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
          <IconChevronRight size={14} className="develop-sidebar__chevron" aria-hidden />
          {title}
        </button>
        {action}
      </div>
      <div className="develop-sidebar__body" aria-hidden={!open} inert={!open}>
        <div className="develop-sidebar__inner">{children}</div>
      </div>
    </section>
  );
}

/** A name edited in place: Enter keeps it, Escape or an empty name gives up. */
export function InlineName({ value, label, onDone }: { value: string; label: string; onDone: (name: string | null) => void }) {
  const [text, setText] = useState(value);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);
  const done = (keep: boolean) => onDone(keep && text.trim() ? text.trim() : null);
  return (
    <input
      ref={ref}
      className="develop-sidebar__name-input"
      aria-label={label}
      value={text}
      onChange={(e) => setText(e.currentTarget.value)}
      onBlur={() => done(true)}
      onKeyDown={(e) => {
        if (e.key === "Enter") done(true);
        else if (e.key === "Escape") done(false);
        else return;
        e.preventDefault();
        e.stopPropagation();
      }}
    />
  );
}
