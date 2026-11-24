import { Button } from "@mantine/core";
import { IconChevronLeft, IconChevronRight, IconPuzzle } from "@tabler/icons-react";
import { useMemo, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { SearchInput } from "@/components/page-header";
import type { StoreEntry } from "@/types/generated/StoreEntry";
import type { CardAction } from "./detail";
import { Hero } from "./hero";
import { ActionButton, type Actions, TileGrid } from "./tile";

/** The chips of the Plugins tab, in order; a kind the index has beyond these gets a chip after them. */
const KINDS: { id: string; label: string; none: string }[] = [
  { id: "tools", label: "Tools", none: "tools" },
  { id: "presets", label: "Looks", none: "looks" },
  { id: "importer", label: "Importers", none: "importers" },
  { id: "apps", label: "Apps", none: "apps" },
  { id: "provider", label: "Providers", none: "providers" },
];
const EXTRA_LABEL: Record<string, string> = { prompts: "Prompts", theme: "Themes" };
const kindLabel = (k: string) => KINDS.find((x) => x.id === k)?.label ?? EXTRA_LABEL[k] ?? k;

const NEW_DAYS = 30;
/** Tiles a section shows before *See all*. */
const PREVIEW = 8;
const HERO_MAX = 4;

const isNew = (e: StoreEntry) => {
  const t = Date.parse(e.latest.released);
  return Number.isFinite(t) && Date.now() - t <= NEW_DAYS * 86_400_000 && t <= Date.now() + 86_400_000;
};
const matches = (e: StoreEntry, q: string) => [e.name, e.author, e.summary, e.id].some((s) => s.toLowerCase().includes(q));

type Shelf = { id: string; title: string; empty: string | null; all: StoreEntry[]; shown: StoreEntry[] };

/**
 * The sections, each plugin in the first one that claims it (Updates, New, By JL Studios, then
 * its first kind), so no plugin shows twice on the screen. A section whose plugins were all
 * claimed above is left out; one with none at all says so when it has an empty line.
 */
function shelves(entries: StoreEntry[], isUpdate: (e: StoreEntry) => boolean): Shelf[] {
  const known = KINDS.map((k) => k.id);
  const extra = [...new Set(entries.flatMap((e) => e.kinds))].filter((k) => !known.includes(k)).sort();
  const defs: { id: string; title: string; empty: string | null; has: (e: StoreEntry) => boolean }[] = [
    { id: "updates", title: "Updates", empty: "Everything you have is up to date.", has: isUpdate },
    { id: "new", title: "New", empty: `Nothing new in the last ${NEW_DAYS} days.`, has: isNew },
    { id: "jl", title: "By JL Studios", empty: null, has: (e) => e.jl },
    ...[...known, ...extra].map((k) => ({ id: `kind-${k}`, title: kindLabel(k), empty: null, has: (e: StoreEntry) => e.kinds.includes(k) })),
  ];
  const claimed = new Set<string>();
  const out: Shelf[] = [];
  for (const d of defs) {
    const all = entries.filter(d.has);
    const free = all.filter((e) => !claimed.has(e.id)).slice(0, PREVIEW);
    for (const e of free) claimed.add(e.id);
    if (free.length > 0 || (all.length === 0 && d.empty)) out.push({ id: d.id, title: d.title, empty: d.empty, all, shown: free });
  }
  return out;
}

function Section({ s, actions, open, seeAll }: { s: Shelf; actions: Actions; open: (e: StoreEntry) => void; seeAll: (id: string) => void }) {
  const id = `store-section-${s.id}`;
  return (
    <section className="store-section" aria-labelledby={id}>
      <div className="store-section__head">
        <h2 id={id} className="store-section__title">
          {s.title}
        </h2>
        {s.all.length > 0 && (
          <button type="button" className="store-section__all" onClick={() => seeAll(s.id)} aria-label={`See all ${s.title}`}>
            See all <IconChevronRight size={14} aria-hidden />
          </button>
        )}
      </div>
      {s.shown.length === 0 ? <p className="store-section__empty">{s.empty}</p> : <TileGrid label={s.title} entries={s.shown} actions={actions} open={open} />}
    </section>
  );
}

/** Discover: search, kind chips, the featured carousel and sections; a flat grid while a search, a kind or *See all* is active. */
export function Discover({
  entries,
  featured,
  actionOf,
  busyId,
  onAction,
  menuOf,
}: {
  entries: StoreEntry[];
  /** The index's `featured` plugin id. */
  featured?: string | null;
  actionOf: (e: StoreEntry) => CardAction;
  busyId: string | null;
  onAction: (e: StoreEntry) => void;
  menuOf?: Actions["menuOf"];
}) {
  const [text, setText] = useState("");
  const [kind, setKind] = useState("all");
  const [params, setParams] = useSearchParams();
  const navigate = useNavigate();
  const actions: Actions = { actionOf, busyId, onAction, menuOf };
  const open = (e: StoreEntry) => navigate(`/plugins/${encodeURIComponent(e.id)}`);
  const q = text.trim().toLowerCase();
  const isUpdate = (e: StoreEntry) => actionOf(e).label === "Update";
  const updateKey = entries.filter(isUpdate).map((e) => e.id).join();

  /** The featured plugin first, then others that have a screenshot (JL Studios' first). */
  const heroItems = useMemo(() => {
    const lead = entries.find((e) => e.id === featured);
    if (!lead) return [];
    const rest = entries.filter((e) => e.id !== lead.id && e.screenshots.length > 0).sort((a, b) => Number(b.jl) - Number(a.jl));
    return [lead, ...rest].slice(0, HERO_MAX);
  }, [entries, featured]);
  const sections = useMemo(() => shelves(entries, isUpdate), [entries, updateKey]);
  const chips = useMemo(() => {
    const known = KINDS.map((k) => k.id);
    const extra = [...new Set(entries.flatMap((e) => e.kinds))].filter((k) => !known.includes(k)).sort();
    return [{ id: "all", label: "All" }, ...KINDS.map((k) => ({ id: k.id, label: k.label })), ...extra.map((k) => ({ id: k, label: kindLabel(k) }))];
  }, [entries]);

  const see = params.get("see");
  const seeing = see ? sections.find((s) => s.id === see) : undefined;
  const seeAll = (id: string) => setParams({ see: id });
  const clear = () => {
    setText("");
    setKind("all");
  };

  const flat = q !== "" || kind !== "all";
  const shown = entries.filter((e) => (kind === "all" || e.kinds.includes(kind)) && (!q || matches(e, q)));

  let body;
  if (flat) {
    const none = KINDS.find((k) => k.id === kind)?.none ?? kindLabel(kind).toLowerCase();
    body =
      shown.length === 0 ? (
        q ? (
          <EmptyState icon={<IconPuzzle />} title="No plugins match." description="Try a different word, or another kind." action={<Button variant="default" onClick={clear}>Clear search</Button>} />
        ) : (
          <EmptyState icon={<IconPuzzle />} title={`No ${none} in the store yet.`} description="Check back later, or look at all plugins." action={<Button variant="default" onClick={clear}>Show all</Button>} />
        )
      ) : (
        <TileGrid key={`${kind}:${q}`} label="Plugins in the store" entries={shown} actions={actions} open={open} />
      );
  } else if (seeing) {
    body = (
      <section className="store-section" aria-labelledby="store-see-all">
        <div className="store-section__head">
          <button type="button" className="store-section__back" onClick={() => setParams({})} aria-label="Back to Discover">
            <IconChevronLeft size={16} aria-hidden />
          </button>
          <h2 id="store-see-all" className="store-section__title store-section__title--big">
            {seeing.title}
          </h2>
          <span className="store-section__count">{seeing.all.length}</span>
        </div>
        <TileGrid label={seeing.title} entries={seeing.all} actions={actions} open={open} />
      </section>
    );
  } else if (entries.length === 0) {
    body = <EmptyState icon={<IconPuzzle />} title="The store has no plugins yet." description="Check back later." />;
  } else {
    body = (
      <>
        <Hero items={heroItems} action={(e) => <ActionButton e={e} actions={actions} size="sm" />} open={open} />
        {sections.map((s) => (
          <Section key={s.id} s={s} actions={actions} open={open} seeAll={seeAll} />
        ))}
      </>
    );
  }

  return (
    <>
      {!seeing && (
        <div className="store-filters">
          <SearchInput text={text} setText={setText} clear={() => setText("")} placeholder="Search plugins" />
          <div className="store-chips" role="group" aria-label="Kind">
            {chips.map((c) => (
              <button key={c.id} type="button" className="store-chip" aria-pressed={kind === c.id} onClick={() => setKind(c.id)}>
                {c.label}
              </button>
            ))}
          </div>
        </div>
      )}
      {body}
    </>
  );
}
