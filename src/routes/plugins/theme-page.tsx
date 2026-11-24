import { ActionIcon, Button } from "@mantine/core";
import { IconArrowLeft, IconDots, IconInfoCircle, IconPalette, IconPlugConnectedX } from "@tabler/icons-react";
import { motion } from "motion/react";
import { type CSSProperties, useEffect, useState } from "react";
import Markdown from "react-markdown";
import { useNavigate, useParams } from "react-router";
import { useContextMenu } from "@/components/context-menu";
import { EmptyState } from "@/components/empty-state";
import { useStoreIndex } from "@/hooks/use-store-index";
import { packageSize } from "@/lib/plugin-permission-words";
import { storeReadme } from "@/lib/plugin-store";
import { openLink } from "./detail";
import { Lightbox, Shots } from "./shots";
import { ThemeMiniature } from "./theme-miniature";
import { ActionButton, Dots, hasLayout, type ThemeActions, ThemeIcon, TryButton } from "./theme-tile";
import { swatchOf } from "./tile";
import { TryBar } from "./try-bar";
import { useThemeActions } from "./use-themes";
import "./styles.scss";
import "./themes.scss";

/** The README text from the store's cached copy; none when the entry lists none. */
function useReadme(id: string, has: boolean): string | null {
  const [text, setText] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    (has ? storeReadme(id) : Promise.resolve(null)).then(
      (t) => live && setText(t),
      () => live && setText(null),
    );
    return () => {
      live = false;
    };
  }, [id, has]);
  return text;
}

/** `/themes/:id`: one theme's page, in the plugin page's language, without permissions (a theme has none). */
export function ThemePageRoute() {
  const { id = "" } = useParams();
  const navigate = useNavigate();
  const { state } = useStoreIndex();
  const t = useThemeActions();
  const menu = useContextMenu();
  const [shot, setShot] = useState<number | null>(null);
  const [dark, setDark] = useState(false);
  const entry = state.phase === "ready" ? (state.index.themes ?? []).find((x) => x.id === id) : undefined;
  const readme = useReadme(id, entry?.readme != null);
  const back = () => navigate("/plugins?tab=themes");

  if (!entry) {
    if (state.phase === "loading") return <div className="plugin-page" />;
    return (
      <div className="plugin-page">
        <EmptyState
          icon={state.phase === "ready" ? <IconPalette /> : <IconPlugConnectedX />}
          title={state.phase === "ready" ? "That theme is not in the store." : "The store could not be loaded."}
          description="Go back to the list to pick another."
          action={
            <Button variant="default" onClick={back}>
              Back to themes
            </Button>
          }
        />
      </div>
    );
  }

  const actions: ThemeActions = {
    actionOf: t.actionOf,
    busyId: t.busyId,
    onAction: (x) => void t.act(x.id, x.name),
    onTry: (x) => void t.tryTheme(x.id),
  };
  const items = () => t.menuOf(entry);
  return (
    <div className="plugin-page theme-page">
      <header className="plugin-page__head" onContextMenu={(ev) => menu.open(ev, items())}>
        <span className="plugin-page__wash" aria-hidden style={{ "--tile-swatch": `var(--cover-swatch-${swatchOf(entry.id)})` } as CSSProperties}>
          {entry.icon && <img src={entry.icon} alt="" draggable={false} />}
        </span>
        <Button className="plugin-page__back" variant="subtle" size="xs" radius="xl" leftSection={<IconArrowLeft size={14} />} onClick={back} aria-label="Back to themes">
          Themes
        </Button>
        <motion.div className="plugin-page__hero theme-page__hero" initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.35, ease: [0.22, 1, 0.36, 1] }}>
          <ThemeIcon url={entry.icon} size={104} className="plugin-page__icon" />
          <div className="plugin-page__title">
            <h1>{entry.name}</h1>
            <div className="plugin-page__author">
              {entry.authorUrl ? (
                <button type="button" className="plugin-page__link" onClick={() => openLink(entry.authorUrl as string)}>
                  {entry.author}
                </button>
              ) : (
                entry.author
              )}
              {entry.jl && <span className="store-badge">By JL Studios</span>}
              {hasLayout(entry) && <span className="store-badge">Layout</span>}
            </div>
            <p className="plugin-page__summary">{entry.summary}</p>
            <div className="plugin-page__cta">
              <ActionButton t={entry} actions={actions} size="sm" />
              <TryButton t={entry} actions={actions} size="sm" />
              <ActionIcon variant="subtle" color="gray" radius="xl" size="lg" aria-label={`More for ${entry.name}`} onClick={(ev) => menu.open(ev, items())}>
                <IconDots size={18} />
              </ActionIcon>
              <span className="plugin-page__meta">Theme · {entry.latest.version}</span>
            </div>
          </div>
          <div className="theme-page__stage">
            <button type="button" className="theme-page__mini" onClick={() => setDark((d) => !d)} aria-label={dark ? "Show the light version" : "Show the dark version"} aria-pressed={dark}>
              <ThemeMiniature tokens={entry.tokens} scheme={dark ? "dark" : "light"} />
            </button>
            <Dots t={entry} />
          </div>
        </motion.div>
      </header>

      {entry.screenshots.length > 0 && <Shots name={entry.name} shots={entry.screenshots} onOpen={setShot} />}

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
            <p className="plugin-page__card-text">{entry.summary}</p>
          )}
        </div>
        <aside className="plugin-page__side">
          <section className="plugin-page__card" aria-labelledby="theme-info">
            <h2 id="theme-info" className="plugin-page__card-title">
              <IconInfoCircle size={16} aria-hidden /> Information
            </h2>
            <dl className="plugin-page__facts">
              <dt>Version</dt>
              <dd>{entry.latest.version}</dd>
              <dt>Kind</dt>
              <dd>{hasLayout(entry) ? "Colours and layout" : "Colours"}</dd>
              <dt>Size</dt>
              <dd>{packageSize(entry.latest.size)}</dd>
              <dt>Released</dt>
              <dd>{entry.latest.released}</dd>
            </dl>
            <p className="plugin-page__card-text theme-page__note">A theme changes how Lumen looks. It has no code and no access to your photos.</p>
            <div className="plugin-page__links">
              {entry.repository && (
                <Button variant="light" size="xs" radius="xl" onClick={() => openLink(entry.repository as string)}>
                  Repository
                </Button>
              )}
              {entry.homepage && (
                <Button variant="light" size="xs" radius="xl" onClick={() => openLink(entry.homepage as string)}>
                  Homepage
                </Button>
              )}
            </div>
          </section>
        </aside>
      </div>

      <Lightbox shots={entry.screenshots} at={shot} onChange={setShot} />
      <TryBar state={t.trying} onKeep={t.keep} onRevert={t.revert} />
    </div>
  );
}
