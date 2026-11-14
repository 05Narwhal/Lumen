import { TextInput } from "@mantine/core";
import { useEffect, useMemo, useRef, useState } from "react";
import { COVER_ICONS, COVER_ICON_CATEGORIES } from "@/components/icon-cover";

const RECENT_KEY = "lumen.cover.recent-icons";
const RECENT_MAX = 8;

function readRecent(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((n): n is string => typeof n === "string").slice(0, RECENT_MAX) : [];
  } catch {
    return [];
  }
}

function writeRecent(name: string): string[] {
  const next = [name, ...readRecent().filter((n) => n !== name)].slice(0, RECENT_MAX);
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    /* private window: the section just stays as it was */
  }
  return next;
}

type Entry = (typeof COVER_ICONS)[number];

/**
 * The cover editor's icon choice as an emoji picker (Round 20 §7): one search box, one scrolling
 * area of sections (Recent, then each category with a sticky heading), a flat grid while searching.
 */
export function IconPicker({ value, onChange }: { value: string; onChange: (name: string) => void }) {
  const [find, setFind] = useState("");
  const [recent, setRecent] = useState<string[]>(readRecent);
  const scroller = useRef<HTMLDivElement>(null);
  const q = find.trim().toLowerCase();

  const sections = useMemo(() => {
    const byName = new Map(COVER_ICONS.map((i) => [i.name, i]));
    const rec = recent.map((n) => byName.get(n)).filter((i): i is Entry => !!i);
    return [
      ...(rec.length > 0 ? [{ title: "Recent", items: rec }] : []),
      ...COVER_ICON_CATEGORIES.map((c) => ({ title: c as string, items: COVER_ICONS.filter((i) => i.category === c) })),
    ];
  }, [recent]);
  const hits = q ? COVER_ICONS.filter((i) => `${i.label} ${i.name} ${i.category}`.toLowerCase().includes(q)) : [];

  // On open, the chosen icon is scrolled into view (its first appearance past Recent).
  useEffect(() => {
    const el = scroller.current?.querySelector<HTMLElement>('[data-section="cat"] [aria-pressed="true"]');
    el?.scrollIntoView?.({ block: "nearest" });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const choose = (name: string) => {
    setRecent(writeRecent(name));
    onChange(name);
  };
  const cell = ({ name, label, icon: Icon }: Entry, prefix: string) => (
    <button key={`${prefix}${name}`} type="button" className="cover-editor__icon" aria-label={label} aria-pressed={value === name} title={label} onClick={() => choose(name)}>
      <Icon size={20} />
    </button>
  );

  return (
    <>
      <TextInput size="xs" placeholder="Search icons" aria-label="Search icons" value={find} onChange={(e) => setFind(e.currentTarget.value)} />
      <div className="cover-editor__icons" ref={scroller}>
        {q ? (
          hits.length === 0 ? (
            <span className="cover-editor__none-line">No icon by that name.</span>
          ) : (
            <>
              <h4 className="cover-editor__section">{hits.length} icons</h4>
              <div className="cover-editor__cells">{hits.map((i) => cell(i, "s"))}</div>
            </>
          )
        ) : (
          sections.map((s) => (
            <section key={s.title} data-section={s.title === "Recent" ? "recent" : "cat"}>
              <h4 className="cover-editor__section">{s.title}</h4>
              <div className="cover-editor__cells">{s.items.map((i) => cell(i, s.title))}</div>
            </section>
          ))
        )}
      </div>
    </>
  );
}
