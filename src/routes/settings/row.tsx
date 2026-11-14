import type { ReactNode } from "react";
import { highlight, matches, pointAt, useSearchQuery } from "./search";

/**
 * One setting (Round 5; one line since the 2026-11 rebuild): what it is and, beside it, one
 * plain sentence on what it does, the control on the right. The inspector shows the whole
 * sentence of the row the pointer or focus is on. `wide` gives the control the row's full
 * width below the text (a list, a picker). While Settings is searching, a row that does not
 * match is left out and its matches are marked.
 */
export function SettingRow({ label, hint, children, wide }: { label?: ReactNode; hint?: ReactNode; children?: ReactNode; wide?: boolean }) {
  const query = useSearchQuery();
  if (query && (!(label || hint) || !matches(query, label, hint))) return null;
  const point = label || hint ? () => pointAt({ label, hint }) : undefined;
  return (
    <div className={wide ? "settings-row settings-row--wide" : "settings-row"} onMouseEnter={point} onFocus={point}>
      {(label || hint) && (
        <div className="settings-row__text">
          {label && <span className="settings-row__label">{highlight(label, query)}</span>}
          {hint && <span className="settings-row__hint">{highlight(hint, query)}</span>}
        </div>
      )}
      {children && <div className="settings-row__control">{children}</div>}
    </div>
  );
}

/** Rows that belong together, on one surface, with an optional title above it. */
export function SettingGroup({ title, children }: { title?: string; children: ReactNode }) {
  return (
    <div className="settings-group">
      {title && <h3 className="settings-group__title">{title}</h3>}
      <div className="settings-group__rows">{children}</div>
    </div>
  );
}
