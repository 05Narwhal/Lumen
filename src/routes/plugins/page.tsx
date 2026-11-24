import { ActionIcon, Button } from "@mantine/core";
import { IconArrowLeft, IconDots, IconInfoCircle, IconPlugConnectedX, IconPuzzle, IconShieldCheck } from "@tabler/icons-react";
import { motion } from "motion/react";
import { type CSSProperties, useEffect, useState } from "react";
import Markdown from "react-markdown";
import { useNavigate, useParams } from "react-router";
import { useContextMenu } from "@/components/context-menu";
import { EmptyState } from "@/components/empty-state";
import { useStoreIndex } from "@/hooks/use-store-index";
import { NO_PERMISSIONS, packageSize, permissionWords } from "@/lib/plugin-permission-words";
import { storeReadme } from "@/lib/plugin-store";
import { pluginReadme } from "@/lib/plugins";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import type { StoreEntry } from "@/types/generated/StoreEntry";
import { usePluginActions } from "./actions";
import { openLink } from "./detail";
import { Lightbox, Shots } from "./shots";
import { kindWord, StoreIcon, swatchOf } from "./tile";
import "./styles.scss";

/** What the page shows, from the store's entry and/or the installed copy. */
type View = {
  id: string;
  name: string;
  author: string;
  jl: boolean;
  summary: string;
  icon: string | null;
  screenshots: string[];
  authorUrl: string | null;
  repository: string | null;
  homepage: string | null;
  permissions: string[];
  kinds: string[];
  version: string;
  size: number | null;
  released: string | null;
};

function viewOf(entry: StoreEntry | undefined, info: PluginInfo | undefined): View | null {
  if (entry) {
    return {
      id: entry.id,
      name: entry.name,
      author: entry.author,
      jl: entry.jl,
      summary: entry.summary,
      icon: entry.icon ?? info?.iconUrl ?? null,
      screenshots: entry.screenshots.length > 0 ? entry.screenshots : (info?.screenshots ?? []),
      authorUrl: entry.authorUrl,
      repository: entry.repository,
      homepage: entry.homepage,
      permissions: entry.permissions,
      kinds: entry.kinds,
      version: entry.latest.version,
      size: entry.latest.size,
      released: entry.latest.released,
    };
  }
  if (!info) return null;
  return {
    id: info.id,
    name: info.name,
    author: info.author,
    jl: info.trust.type === "jlStudios",
    summary: info.description,
    icon: info.iconUrl,
    screenshots: info.screenshots,
    authorUrl: info.authorUrl,
    repository: info.repository,
    homepage: info.homepage,
    permissions: info.permissions as string[],
    kinds: info.kinds as string[],
    version: info.version,
    size: null,
    released: null,
  };
}

/** The README as text: the installed copy's own, else the store's cached one. */
function useReadme(id: string, installedHasReadme: boolean, storeHasReadme: boolean): string | null {
  const [text, setText] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    const read = installedHasReadme ? pluginReadme(id) : storeHasReadme ? storeReadme(id) : Promise.resolve(null);
    read.then(
      (t) => live && setText(t),
      () => live && setText(null),
    );
    return () => {
      live = false;
    };
  }, [id, installedHasReadme, storeHasReadme]);
  return text;
}

/** `/plugins/:id`: one plugin's page. Nothing on it loads a remote URL; images are cached copies over `photos://`. */
export function PluginPageRoute() {
  const { id = "" } = useParams();
  const navigate = useNavigate();
  const { state } = useStoreIndex();
  const { plugins, busyId, actionOf, act, menuOf } = usePluginActions();
  const menu = useContextMenu();
  const [shot, setShot] = useState<number | null>(null);
  const entry = state.phase === "ready" ? state.index.plugins.find((e) => e.id === id) : undefined;
  const info = plugins.find((p) => p.id === id);
  const readme = useReadme(id, info?.hasReadme ?? false, entry?.readme != null);
  const view = viewOf(entry, info);
  const back = () => navigate("/plugins");

  if (!view) {
    if (state.phase === "loading") return <div className="plugin-page" />;
    return (
      <div className="plugin-page">
        <EmptyState
          icon={state.phase === "ready" ? <IconPuzzle /> : <IconPlugConnectedX />}
          title={state.phase === "ready" ? "That plugin is not in the store." : "The store could not be loaded."}
          description="Go back to the list to pick another."
          action={
            <Button variant="default" onClick={back}>
              Back to plugins
            </Button>
          }
        />
      </div>
    );
  }

  const action = actionOf(view);
  // Right-click the header (the README stays selectable), or the ⋯ button: Get / Update, Settings and Remove for an installed plugin.
  const items = () => menuOf(view);
  return (
    <div className="plugin-page">
      <header className="plugin-page__head" onContextMenu={(ev) => menu.open(ev, items())}>
        <span className="plugin-page__wash" aria-hidden style={{ "--tile-swatch": `var(--cover-swatch-${swatchOf(view.id)})` } as CSSProperties}>
          {view.icon && <img src={view.icon} alt="" draggable={false} />}
        </span>
        <Button className="plugin-page__back" variant="subtle" size="xs" radius="xl" leftSection={<IconArrowLeft size={14} />} onClick={back} aria-label="Back to plugins">
          Plugins
        </Button>
        <motion.div className="plugin-page__hero" initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.35, ease: [0.22, 1, 0.36, 1] }}>
          <StoreIcon url={view.icon} size={104} className="plugin-page__icon" />
          <div className="plugin-page__title">
            <h1>{view.name}</h1>
            <div className="plugin-page__author">
              {view.authorUrl ? (
                <button type="button" className="plugin-page__link" onClick={() => openLink(view.authorUrl as string)}>
                  {view.author}
                </button>
              ) : (
                view.author
              )}
              {view.jl && <span className="store-badge">By JL Studios</span>}
            </div>
            <p className="plugin-page__summary">{view.summary}</p>
            <div className="plugin-page__cta">
              <Button radius="xl" variant={action.label === "Open" ? "default" : "filled"} onClick={() => act(view)} disabled={action.disabled || (busyId !== null && busyId !== view.id)} loading={busyId === view.id}>
                {action.label}
              </Button>
              <ActionIcon variant="subtle" color="gray" radius="xl" size="lg" aria-label={`More for ${view.name}`} onClick={(ev) => menu.open(ev, items())}>
                <IconDots size={18} />
              </ActionIcon>
              <span className="plugin-page__meta">
                {kindWord(view.kinds)} · {view.version}
              </span>
            </div>
          </div>
        </motion.div>
      </header>

      {view.screenshots.length > 0 && <Shots name={view.name} shots={view.screenshots} onOpen={setShot} />}

      <div className="plugin-page__cols">
        <div className="plugin-page__readme">
          {readme ? (
            <Markdown
              skipHtml
              components={{
                a: ({ href, children }) => (
                  <a
                    href={href}
                    onClick={(e) => {
                      e.preventDefault();
                      if (href) openLink(href);
                    }}
                  >
                    {children}
                  </a>
                ),
                img: () => null,
              }}
            >
              {readme}
            </Markdown>
          ) : (
            <p className="plugin-page__card-text">{view.summary}</p>
          )}
        </div>
        <aside className="plugin-page__side">
          <section className="plugin-page__card" aria-labelledby="plugin-can">
            <h2 id="plugin-can" className="plugin-page__card-title">
              <IconShieldCheck size={16} aria-hidden /> It can
            </h2>
            {view.permissions.length === 0 ? (
              <p className="plugin-page__card-text">{NO_PERMISSIONS}</p>
            ) : (
              <ul className="plugin-page__perms">
                {view.permissions.map((p) => (
                  <li key={p}>{permissionWords(p)}</li>
                ))}
              </ul>
            )}
          </section>
          <section className="plugin-page__card" aria-labelledby="plugin-facts">
            <h2 id="plugin-facts" className="plugin-page__card-title">
              <IconInfoCircle size={16} aria-hidden /> Information
            </h2>
          <dl className="plugin-page__facts">
            <dt>Version</dt>
            <dd>{view.version}</dd>
            <dt>Kind</dt>
            <dd>{kindWord(view.kinds)}</dd>
            {view.size !== null && (
              <>
                <dt>Size</dt>
                <dd>{packageSize(view.size)}</dd>
              </>
            )}
            {view.released && (
              <>
                <dt>Released</dt>
                <dd>{view.released}</dd>
              </>
            )}
          </dl>
          <div className="plugin-page__links">
            {view.repository && (
              <Button variant="light" size="xs" radius="xl" onClick={() => openLink(view.repository as string)}>
                Repository
              </Button>
            )}
            {view.homepage && (
              <Button variant="light" size="xs" radius="xl" onClick={() => openLink(view.homepage as string)}>
                Homepage
              </Button>
            )}
          </div>
          </section>
        </aside>
      </div>

      <Lightbox shots={view.screenshots} at={shot} onChange={setShot} />
    </div>
  );
}
