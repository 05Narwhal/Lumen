import { Button } from "@mantine/core";
import { IconSearch } from "@tabler/icons-react";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useLocation, useNavigate, useSearchParams } from "react-router";
import { PageHeader } from "@/components/page-header";
import { SettingsNav } from "@/components/settings-nav";
import { useSettings } from "@/hooks/use-settings";
import { returnPath } from "@/lib/return-to";
import { discordInfoQuery } from "./discord";
import { Inspector } from "./inspector";
import { pointAt, SearchContext } from "./search";
import { GROUPS, SECTIONS, groupOf, useSectionStatus } from "./sections";
import "./styles.scss";

const GROUP_ORDER = GROUPS.map((g) => g.id);

/**
 * Settings (UX spec §6.9, Rounds 5 and 18, rebuilt 2026-11 as Julian's sample B): every section
 * in one slim list on the left under five group headings, a search box above it (Ctrl/⌘ F), a
 * page per section. A page is one column of one-line rows beside a sticky inspector that shows
 * the section's state and the whole explanation of the row the pointer or focus is on; a
 * `full` section (Storage, Keyboard, Licences) takes the whole width; People's faces keep the
 * inspector. A search shows the matching rows of every row section at once. The section is named in the address
 * (`?section=music`; an older `#music` still works). Back, or Esc, returns to the page Settings
 * was opened from. The list and the page body scroll on their own under a header that stays.
 */
export function SettingsRoute() {
  const { settings, update } = useSettings();
  const status = useSectionStatus(settings);
  const [params, setParams] = useSearchParams();
  const { hash } = useLocation();
  const asked = params.get("section") ?? hash.replace(/^#/, "");
  // A build with no Discord Application ID has no Discord section (Round 17).
  const discord = useQuery(discordInfoQuery).data;
  const sections = useMemo(() => SECTIONS.filter((s) => s.id !== "discord" || discord?.available), [discord?.available]);
  // In the sidebar's order: by group, each group's sections in its listed order.
  const ordered = useMemo(() => [...sections].sort((a, b) => rank(a.id) - rank(b.id)), [sections]);
  const section = ordered.find((s) => s.id === asked) ?? ordered[0];
  const navigate = useNavigate();
  const back = returnPath();
  const [query, setQuery] = useState("");
  const searchRef = useRef<HTMLInputElement>(null);
  const searching = query.trim() !== "";
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
        return;
      }
      // A dialog or menu open on top answers Esc itself.
      if (e.key !== "Escape" || e.defaultPrevented || document.querySelector("[role=dialog], [role=menu]")) return;
      const t = e.target as HTMLElement | null;
      if (t?.closest("input, textarea, select, [contenteditable=true]")) return;
      navigate(back);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [navigate, back]);
  // A new section starts with nothing pointed at.
  useEffect(() => pointAt(null), [section.id]);
  const hits = useHits(searching);
  if (!settings) return null;

  const open = (id: string) => {
    setQuery("");
    setParams({ section: id }, { replace: true });
  };
  const searchable = ordered.filter((s) => s.layout !== "full" && s.layout !== "tiles");
  const total = Object.values(hits.counts).reduce((a, b) => a + b, 0);

  return (
    <div className="settings">
      <PageHeader title="Settings" search={false} back={{ to: back, label: "Back" }} />
      <div className="settings__layout">
        <aside className="settings__nav">
          <SettingsNav
            sections={ordered.map((s) => ({ id: s.id, label: s.label, icon: s.icon, status: status[s.id], group: groupOf(s.id)?.label }))}
            active={section.id}
            onChange={open}
            query={query}
            onQuery={setQuery}
            hits={hits.counts}
            searchRef={searchRef}
          />
        </aside>
        <div className="settings__pane">
          {searching ? (
            <SearchContext.Provider value={query.trim()}>
              <section key="search" ref={hits.ref} className="settings__body settings__body--search" aria-label="Search results">
                <header className="settings__pane-head">
                  <h2>Results</h2>
                  <p className="settings__intro" role="status">
                    {total === 0 ? "" : `${total} ${total === 1 ? "setting matches" : "settings match"} “${query.trim()}”`}
                  </p>
                </header>
                {searchable.map((s) => (
                  <div key={s.id} className="settings-result" data-result={s.id}>
                    <h3 className="settings-result__head">
                      <span aria-hidden>{s.icon}</span>
                      {s.label}
                      <button type="button" onClick={() => open(s.id)} aria-label={`Open ${s.label}`}>
                        Open
                      </button>
                    </h3>
                    <div className="settings__list">
                      <s.Body settings={settings} update={update} />
                    </div>
                  </div>
                ))}
                {total === 0 && (
                  <div className="settings__empty">
                    <IconSearch size={22} stroke={1.6} aria-hidden />
                    <span>No settings match “{query.trim()}”.</span>
                    <Button variant="default" size="xs" onClick={() => setQuery("")}>
                      Clear search
                    </Button>
                  </div>
                )}
              </section>
            </SearchContext.Provider>
          ) : (
            <section key={section.id} className="settings__body" aria-labelledby={`settings-title-${section.id}`}>
              <header className="settings__pane-head">
                <h2 id={`settings-title-${section.id}`}>{section.label}</h2>
                <p className="settings__intro">{section.intro}</p>
              </header>
              {section.layout === "full" ? (
                <div className="settings__list settings__list--full">
                  <section.Body settings={settings} update={update} />
                </div>
              ) : (
                <div className="settings__split">
                  <div className="settings__list">
                    <section.Body settings={settings} update={update} />
                  </div>
                  <Inspector section={section} status={status[section.id]} settings={settings} />
                </div>
              )}
            </section>
          )}
        </div>
      </div>
    </div>
  );
}

/** How many rows each section shows while searching, counted from the page as it fills in. */
function useHits(searching: boolean) {
  const [el, setEl] = useState<HTMLElement | null>(null);
  const [counts, setCounts] = useState<Record<string, number>>({});
  useLayoutEffect(() => {
    if (!searching || !el) {
      setCounts({});
      return;
    }
    const count = () => {
      const next: Record<string, number> = {};
      el.querySelectorAll<HTMLElement>("[data-result]").forEach((r) => {
        next[r.dataset.result!] = r.querySelectorAll(".settings-row").length;
      });
      setCounts((prev) => (JSON.stringify(prev) === JSON.stringify(next) ? prev : next));
    };
    count();
    const mo = new MutationObserver(count);
    mo.observe(el, { childList: true, subtree: true, characterData: true });
    return () => mo.disconnect();
  }, [searching, el]);
  return { ref: setEl, counts };
}

/** A section's place in the sidebar: its group's order, then its order inside the group. */
function rank(id: string): number {
  const g = groupOf(id);
  return g ? GROUP_ORDER.indexOf(g.id) * 100 + g.tabs.indexOf(id) : 9999;
}
