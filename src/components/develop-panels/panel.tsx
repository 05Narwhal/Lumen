import { IconChevronRight, IconRestore } from "@tabler/icons-react";
import { useState, type ReactNode } from "react";
import type { DevelopSession } from "@/hooks/use-develop-session";
import type { Develop } from "@/types/generated/Develop";

/** `mask`: the panel edits the selected mask's adjustments and hides what only a photo has. */
export type PanelScope = "photo" | "mask";

const OPEN_KEY = "lumen.develop.closed-panels";

function readClosed(): string[] {
  try {
    return JSON.parse(localStorage.getItem(OPEN_KEY) ?? "[]") as string[];
  } catch {
    return [];
  }
}

function writeClosed(ids: string[]) {
  try {
    localStorage.setItem(OPEN_KEY, JSON.stringify(ids));
  } catch {
    // A private window: remembered for this session only.
  }
}

/**
 * One panel of the right column: a quiet header that folds it away (remembered per viewer)
 * and resets it, and its rows.
 */
export function Panel({ id, title, changed, onReset, children }: { id: string; title: string; changed: boolean; onReset: () => void; children: ReactNode }) {
  const [open, setOpen] = useState(() => !readClosed().includes(id));
  const toggle = () => {
    const next = !open;
    setOpen(next);
    const closed = new Set(readClosed());
    if (next) closed.delete(id);
    else closed.add(id);
    writeClosed([...closed]);
  };
  return (
    <section className="develop-panel" data-open={open} aria-label={title}>
      <div className="develop-panel__head">
        <button type="button" className="develop-panel__title" aria-expanded={open} onClick={toggle}>
          <IconChevronRight size={14} className="develop-panel__chevron" aria-hidden />
          {title}
          {changed && <span className="develop-panel__dot" aria-label="changed" />}
        </button>
        <button type="button" className="develop-panel__reset" aria-label={`Reset ${title}`} title={`Reset ${title}`} disabled={!changed} onClick={onReset}>
          <IconRestore size={14} />
        </button>
      </div>
      {/* Kept mounted so it can fold smoothly; out of reach while folded. */}
      <div className="develop-panel__body" aria-hidden={!open} inert={!open}>
        <div className="develop-panel__inner">{children}</div>
      </div>
    </section>
  );
}

/** A quiet heading inside a panel, for a group of rows. */
export function Group({ title, children, action }: { title: string; children: ReactNode; action?: ReactNode }) {
  return (
    <div className="develop-panel__group">
      <div className="develop-panel__group-head">
        <span>{title}</span>
        {action}
      </div>
      {children}
    </div>
  );
}

/** Wiring for a slider: its value in display units, a live change and a recorded one. */
export function bind(session: DevelopSession, read: (d: Develop) => number, write: (d: Develop, v: number) => Develop, scale = 1) {
  return {
    value: read(session.develop) * scale,
    onChange: (v: number) => session.update((d) => write(d, v / scale)),
    onCommit: (v: number, label: string) => {
      session.update((d) => write(d, v / scale));
      session.commit(label);
    },
  };
}
