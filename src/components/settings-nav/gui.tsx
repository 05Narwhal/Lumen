import { IconSearch } from "@tabler/icons-react";
import { motion } from "motion/react";
import { Fragment, useRef, type KeyboardEvent, type ReactNode, type RefObject } from "react";
import { isMac } from "@/components/window-controls";
import "./styles.scss";

export interface SettingsSection {
  id: string;
  label: string;
  icon?: ReactNode;
  /** A short tag on the entry's right: the section's state ("On", "Alpha"). */
  status?: string | null;
  /** The heading this entry sits under; a heading is drawn where it changes. */
  group?: string;
}

const SPRING = { type: "spring", stiffness: 520, damping: 42, mass: 0.8 } as const;

/**
 * The Settings sidebar (2026-11 rebuild): a search box, then one line per section under small
 * group headings, its state as a tag on the right. ↑/↓ (and Home/End) move between entries and
 * the highlight slides between them as one shared element. While searching, each entry shows
 * how many of its settings match and an entry with none is dimmed.
 */
export function SettingsNav({
  sections,
  active,
  onChange,
  query = "",
  onQuery,
  hits,
  searchRef,
}: {
  sections: SettingsSection[];
  active: string;
  onChange: (id: string) => void;
  query?: string;
  onQuery?: (q: string) => void;
  hits?: Record<string, number>;
  searchRef?: RefObject<HTMLInputElement | null>;
}) {
  const refs = useRef(new Map<string, HTMLButtonElement>());
  const searching = query.trim() !== "";
  const go = (id: string) => {
    onChange(id);
    refs.current.get(id)?.focus();
  };
  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    const i = sections.findIndex((s) => s.id === active);
    const to = { ArrowDown: i + 1, ArrowUp: i - 1, Home: 0, End: sections.length - 1 }[e.key];
    if (to === undefined) return;
    e.preventDefault();
    go(sections[(to + sections.length) % sections.length].id);
  };
  return (
    <div className="settings-nav-wrap">
      {onQuery && (
        <label className="settings-nav__search">
          <IconSearch size={14} stroke={1.8} aria-hidden />
          <input
            ref={searchRef}
            type="search"
            placeholder="Search settings"
            aria-label="Search settings"
            autoComplete="off"
            spellCheck={false}
            value={query}
            onChange={(e) => onQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key !== "Escape") return;
              e.preventDefault();
              e.stopPropagation();
              if (query) onQuery("");
              else e.currentTarget.blur();
            }}
          />
          {!query && <kbd aria-hidden>{isMac() ? "⌘F" : "Ctrl F"}</kbd>}
        </label>
      )}
      <nav className="settings-nav" aria-label="Settings" onKeyDown={onKeyDown} data-searching={searching || undefined}>
        {sections.map((s, i) => {
          const on = !searching && s.id === active;
          const heading = s.group && s.group !== sections[i - 1]?.group ? s.group : null;
          const n = hits?.[s.id] ?? 0;
          return (
            <Fragment key={s.id}>
              {heading && <div className="settings-nav__heading" aria-hidden>{heading}</div>}
              <button
                ref={(el) => {
                  if (el) refs.current.set(s.id, el);
                  else refs.current.delete(s.id);
                }}
                type="button"
                id={`settings-section-${s.id}`}
                aria-current={on ? "page" : undefined}
                className="settings-nav__item"
                data-dim={(searching && n === 0) || undefined}
                onClick={() => onChange(s.id)}
              >
                {on && <motion.span layoutId="settings-nav-indicator" className="settings-nav__indicator" aria-hidden transition={SPRING} />}
                {s.icon && <span className="settings-nav__icon" aria-hidden>{s.icon}</span>}
                <span className="settings-nav__label">{s.label}</span>
                {searching ? (
                  n > 0 && <span className="settings-nav__hits" aria-label={`${n} ${n === 1 ? "match" : "matches"}`}>{n}</span>
                ) : (
                  s.status && <span className="settings-nav__status" title={s.status}>{s.status}</span>
                )}
              </button>
            </Fragment>
          );
        })}
      </nav>
    </div>
  );
}
