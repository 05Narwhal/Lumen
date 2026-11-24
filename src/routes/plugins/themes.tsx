import { Button } from "@mantine/core";
import { IconChevronLeft, IconChevronRight, IconPalette } from "@tabler/icons-react";
import { useMemo, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { SearchInput } from "@/components/page-header";
import type { StoreTheme } from "@/types/generated/StoreTheme";
import { Hero } from "./hero";
import { ThemeMiniature } from "./theme-miniature";
import { ActionButton, type ThemeActions, ThemeGrid, TryButton } from "./theme-tile";
import { TryBar } from "./try-bar";
import { useThemeActions } from "./use-themes";
import "./themes.scss";

/** The chips of the Themes tab, in order. */
const CHIPS: { id: string; label: string; none: string }[] = [
  { id: "dark", label: "Dark", none: "dark themes" },
  { id: "light", label: "Light", none: "light themes" },
  { id: "colourful", label: "Colourful", none: "colourful themes" },
  { id: "layout", label: "Layout", none: "themes that change the layout" },
];

const NEW_DAYS = 30;
const PREVIEW = 8;
const HERO_MAX = 4;

const isNew = (t: StoreTheme) => {
  const at = Date.parse(t.latest.released);
  return Number.isFinite(at) && Date.now() - at <= NEW_DAYS * 86_400_000 && at <= Date.now() + 86_400_000;
};
const matches = (t: StoreTheme, q: string) => [t.name, t.author, t.summary, t.id].some((s) => s.toLowerCase().includes(q));
const tagged = (tag: string) => (t: StoreTheme) => t.tags.includes(tag);

type Shelf = { id: string; title: string; all: StoreTheme[]; shown: StoreTheme[] };

/** The sections, each theme in the first that claims it, so none shows twice; leftovers go to *More themes*. */
export function themeShelves(themes: StoreTheme[]): Shelf[] {
  const defs: { id: string; title: string; has: (t: StoreTheme) => boolean }[] = [
    { id: "new", title: "New", has: isNew },
    { id: "jl", title: "By JL Studios", has: (t) => t.jl },
    ...CHIPS.map((c) => ({ id: c.id, title: `${c.label} themes`, has: tagged(c.id) })),
    { id: "more", title: "More themes", has: () => true },
  ];
  const claimed = new Set<string>();
  const out: Shelf[] = [];
  for (const d of defs) {
    const free = themes.filter((t) => d.has(t) && !claimed.has(t.id));
    for (const t of free) claimed.add(t.id);
    if (free.length > 0) out.push({ id: d.id, title: d.title, all: free, shown: free.slice(0, PREVIEW) });
  }
  return out;
}

/** A theme's miniature that goes dark while the pointer is on it (the hero's stage). */
function HoverMini({ t }: { t: StoreTheme }) {
  const [hover, setHover] = useState(false);
  return (
    <span className="theme-hero-mini" onPointerEnter={() => setHover(true)} onPointerLeave={() => setHover(false)}>
      <ThemeMiniature tokens={t.tokens} scheme={hover ? "dark" : "light"} />
    </span>
  );
}

/** The Themes tab: search, chips, the featured carousel and sections; a flat grid while a search, a chip or *See all* is on. */
export function Themes({ themes, featured }: { themes: StoreTheme[]; featured?: string | null }) {
  const [text, setText] = useState("");
  const [chip, setChip] = useState("all");
  const [params, setParams] = useSearchParams();
  const navigate = useNavigate();
  const t = useThemeActions();
  const actions: ThemeActions = {
    actionOf: t.actionOf,
    busyId: t.busyId,
    onAction: (x) => void t.act(x.id, x.name),
    onTry: (x) => void t.tryTheme(x.id),
    menuOf: t.menuOf,
  };
  const open = (x: StoreTheme) => navigate(`/themes/${encodeURIComponent(x.id)}`);
  const q = text.trim().toLowerCase();

  const heroItems = useMemo(() => {
    const lead = themes.find((x) => x.id === featured);
    if (!lead) return [];
    return [lead, ...themes.filter((x) => x.id !== lead.id).sort((a, b) => Number(b.jl) - Number(a.jl))].slice(0, HERO_MAX);
  }, [themes, featured]);
  const sections = useMemo(() => themeShelves(themes), [themes]);

  const see = params.get("see");
  const seeing = see ? sections.find((s) => s.id === see) : undefined;
  const seeAll = (id: string) => setParams({ tab: "themes", see: id });
  const clear = () => {
    setText("");
    setChip("all");
  };
  const flat = q !== "" || chip !== "all";
  const shown = themes.filter((x) => (chip === "all" || x.tags.includes(chip)) && (!q || matches(x, q)));

  let body;
  if (flat) {
    const none = CHIPS.find((c) => c.id === chip)?.none ?? "themes";
    body =
      shown.length === 0 ? (
        q ? (
          <EmptyState icon={<IconPalette />} title="No themes match." description="Try a different word, or another kind." action={<Button variant="default" onClick={clear}>Clear search</Button>} />
        ) : (
          <EmptyState icon={<IconPalette />} title={`No ${none} in the store yet.`} description="Check back later, or look at all themes." action={<Button variant="default" onClick={clear}>Show all</Button>} />
        )
      ) : (
        <ThemeGrid key={`${chip}:${q}`} label="Themes in the store" themes={shown} actions={actions} open={open} />
      );
  } else if (seeing) {
    body = (
      <section className="store-section" aria-labelledby="store-see-all">
        <div className="store-section__head">
          <button type="button" className="store-section__back" onClick={() => setParams({ tab: "themes" })} aria-label="Back to Discover">
            <IconChevronLeft size={16} aria-hidden />
          </button>
          <h2 id="store-see-all" className="store-section__title store-section__title--big">
            {seeing.title}
          </h2>
          <span className="store-section__count">{seeing.all.length}</span>
        </div>
        <ThemeGrid label={seeing.title} themes={seeing.all} actions={actions} open={open} />
      </section>
    );
  } else if (themes.length === 0) {
    body = <EmptyState icon={<IconPalette />} title="The store has no themes yet." description="Check back later." />;
  } else {
    body = (
      <>
        <Hero
          items={heroItems}
          label="Featured themes"
          action={(x) => (
            <>
              <ActionButton t={x} actions={actions} size="sm" />
              <TryButton t={x} actions={actions} size="sm" />
            </>
          )}
          open={open}
          stage={(x) => <HoverMini t={x} />}
        />
        {sections.map((s) => (
          <section key={s.id} className="store-section" aria-labelledby={`store-section-${s.id}`}>
            <div className="store-section__head">
              <h2 id={`store-section-${s.id}`} className="store-section__title">
                {s.title}
              </h2>
              {s.all.length > s.shown.length && (
                <button type="button" className="store-section__all" onClick={() => seeAll(s.id)} aria-label={`See all ${s.title}`}>
                  See all <IconChevronRight size={14} aria-hidden />
                </button>
              )}
            </div>
            <ThemeGrid label={s.title} themes={s.shown} actions={actions} open={open} />
          </section>
        ))}
      </>
    );
  }

  return (
    <>
      {!seeing && (
        <div className="store-filters">
          <SearchInput text={text} setText={setText} clear={() => setText("")} placeholder="Search themes" />
          <div className="store-chips" role="group" aria-label="Kind">
            {[{ id: "all", label: "All" }, ...CHIPS].map((c) => (
              <button key={c.id} type="button" className="store-chip" aria-pressed={chip === c.id} onClick={() => setChip(c.id)}>
                {c.label}
              </button>
            ))}
          </div>
        </div>
      )}
      {body}
      <TryBar state={t.trying} onKeep={t.keep} onRevert={t.revert} />
    </>
  );
}
