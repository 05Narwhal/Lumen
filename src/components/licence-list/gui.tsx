import { TextInput } from "@mantine/core";
import { IconChevronRight, IconExternalLink, IconSearch } from "@tabler/icons-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useMemo, useState } from "react";
import { usePlugins } from "@/hooks/use-plugins";
import "./styles.scss";

/** `src/assets/licenses/notices.json`, written by `scripts/licenses/generate.mjs`. */
export interface NoticeItem {
  name: string;
  version?: string;
  license: string;
  url?: string;
  /** Ids into `texts`. */
  texts: string[];
  /** A credit or statement shown as written. */
  note?: string;
}
export interface Notices {
  generated: string;
  texts: Record<string, string>;
  groups: { id: string; title: string; intro?: string; items: NoticeItem[] }[];
}

/** Lumen's own source, and where each release attaches FFmpeg's exact source. */
const SOURCE_URL = "https://github.com/05Narwhal/lumen";

const open = (url: string) => void openUrl(url).catch(() => window.open(url, "_blank", "noopener"));

function Entry({ item, texts }: { item: NoticeItem; texts: Notices["texts"] }) {
  const [shown, setShown] = useState(false);
  const hasMore = item.texts.length > 0 || !!item.url;
  return (
    <li className="licence-list__item" data-open={shown}>
      <button type="button" className="licence-list__head" aria-expanded={shown} disabled={!hasMore} onClick={() => setShown((v) => !v)}>
        {hasMore && <IconChevronRight size={14} className="licence-list__chevron" aria-hidden />}
        <span className="licence-list__name">{item.name}</span>
        {item.version && <span className="licence-list__version">{item.version}</span>}
        <span className="licence-list__licence">{item.license}</span>
      </button>
      {item.note && <p className="licence-list__note">{item.note}</p>}
      {shown && (
        <div className="licence-list__body">
          {item.url && (
            <button type="button" className="licence-list__link" onClick={() => open(item.url as string)}>
              {item.url} <IconExternalLink size={12} aria-hidden />
            </button>
          )}
          {item.texts.map((id) => (
            <pre key={id} className="licence-list__text">
              {texts[id]}
            </pre>
          ))}
        </div>
      )}
    </li>
  );
}

/**
 * Settings → Licences: every third-party work Lumen ships, runs or shows, grouped, with a
 * filter by name or licence, and each one's licence text on demand. The notices are a large
 * file, so they load only when this list is shown.
 */
export function LicenceList() {
  const [notices, setNotices] = useState<Notices | null>(null);
  const [failed, setFailed] = useState(false);
  const [filter, setFilter] = useState("");
  useEffect(() => {
    let live = true;
    import("@/assets/licenses/notices.json")
      .then((m) => live && setNotices((m.default ?? m) as unknown as Notices))
      .catch(() => live && setFailed(true));
    return () => {
      live = false;
    };
  }, []);
  const { plugins } = usePlugins();
  const groups = useMemo(() => {
    if (!notices) return [];
    const q = filter.trim().toLowerCase();
    return notices.groups
      .map((g) => ({ ...g, items: q ? g.items.filter((i) => i.name.toLowerCase().includes(q) || i.license.toLowerCase().includes(q)) : g.items }))
      .filter((g) => g.items.length > 0);
  }, [notices, filter]);

  if (failed) return <p className="licence-list__empty">The licence list could not be loaded.</p>;
  if (!notices) return <p className="licence-list__empty">Loading licences…</p>;
  return (
    <div className="licence-list">
      <p className="licence-list__source">
        Lumen is free software under the GNU GPL, and its source code is public.{" "}
        <button type="button" className="licence-list__link" onClick={() => open(SOURCE_URL)}>
          Source code <IconExternalLink size={12} aria-hidden />
        </button>
      </p>
      <TextInput
        className="licence-list__filter"
        size="xs"
        placeholder="Find a library or licence"
        aria-label="Find a licence"
        leftSection={<IconSearch size={14} />}
        value={filter}
        onChange={(e) => setFilter(e.currentTarget.value)}
      />
      {groups.length === 0 && <p className="licence-list__empty">Nothing matches “{filter}”.</p>}
      {plugins.length > 0 && (
        <section className="licence-list__group" aria-label="Plugins">
          <h3>
            Plugins <span className="licence-list__count">{plugins.length}</span>
          </h3>
          <ul>
            {plugins.map((p) => (
              <li key={p.id} className="licence-list__item">
                <div className="licence-list__head">
                  <span className="licence-list__name">{p.name}</span>
                  <span className="licence-list__version">{p.version}</span>
                  <span className="licence-list__licence">{p.license}</span>
                </div>
                {p.assets.map((a) => (
                  <p key={a.path} className="licence-list__note">
                    {a.path} · {a.license}
                  </p>
                ))}
              </li>
            ))}
          </ul>
        </section>
      )}
      {groups.map((g) => (
        <section key={g.id} className="licence-list__group" aria-label={g.title}>
          <h3>
            {g.title} <span className="licence-list__count">{g.items.length.toLocaleString()}</span>
          </h3>
          {g.intro && <p className="licence-list__intro">{g.intro}</p>}
          <ul>
            {g.items.map((i) => (
              <Entry key={`${i.name}@${i.version ?? ""}`} item={i} texts={notices.texts} />
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}
