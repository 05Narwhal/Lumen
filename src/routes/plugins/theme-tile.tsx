import { Button } from "@mantine/core";
import { IconPalette } from "@tabler/icons-react";
import { motion } from "motion/react";
import { useState } from "react";
import { useContextMenu, type MenuItem } from "@/components/context-menu";
import type { StoreTheme } from "@/types/generated/StoreTheme";
import { ThemeMiniature } from "./theme-miniature";
import type { ThemeAction } from "./use-themes";

export interface ThemeActions {
  actionOf: (id: string) => ThemeAction;
  busyId: string | null;
  onAction: (t: StoreTheme) => void;
  onTry: (t: StoreTheme) => void;
  /** Right-click menu items for a tile. */
  menuOf?: (t: StoreTheme) => MenuItem[];
}

export const hasLayout = (t: Pick<StoreTheme, "tags">) => t.tags.includes("layout");

/** The theme's icon, else a palette; themes have no puzzle piece. */
export function ThemeIcon({ url, size, className = "" }: { url: string | null; size: number; className?: string }) {
  return url ? (
    <img className={`store-icon ${className}`} src={url} alt="" width={size} height={size} loading="lazy" draggable={false} />
  ) : (
    <span className={`store-icon store-icon--empty ${className}`} aria-hidden style={{ width: size, height: size }}>
      <IconPalette size={Math.round(size / 2)} stroke={1.6} />
    </span>
  );
}

/** Light and dark background, then the accent: the three dots under a theme. */
export function Dots({ t }: { t: StoreTheme }) {
  const colours = [t.tokens.light.bg, t.tokens.dark.bg, t.tokens.light.accent];
  return (
    <span className="theme-dots" aria-hidden>
      {colours.map((c, i) => (
        <i key={i} style={{ background: c }} />
      ))}
    </span>
  );
}

export function ActionButton({ t, actions, size = "xs" }: { t: StoreTheme; actions: ThemeActions; size?: "xs" | "sm" }) {
  const a = actions.actionOf(t.id);
  return (
    <Button
      size={size}
      radius="xl"
      variant={a.label === "Keep" ? "filled" : a.label === "Get" ? "light" : "default"}
      disabled={a.disabled || (actions.busyId !== null && actions.busyId !== t.id)}
      loading={actions.busyId === t.id}
      onClick={() => actions.onAction(t)}
      aria-label={`${a.label} ${t.name}`}
    >
      {a.label}
    </Button>
  );
}

/** *Try* shows only for a theme that isn't installed yet (an installed one is *Use*). */
export function TryButton({ t, actions, size = "xs" }: { t: StoreTheme; actions: ThemeActions; size?: "xs" | "sm" }) {
  if (actions.actionOf(t.id).label !== "Get") return null;
  return (
    <Button size={size} radius="xl" variant="subtle" disabled={actions.busyId !== null} onClick={() => actions.onTry(t)} aria-label={`Try ${t.name}`}>
      Try
    </Button>
  );
}

/** Every tile the same size: a live miniature (light, dark on hover), icon over its edge, name, author, summary, dots and buttons. */
export function ThemeTile({ t, index, actions, open }: { t: StoreTheme; index: number; actions: ThemeActions; open: (t: StoreTheme) => void }) {
  const [hover, setHover] = useState(false);
  const menu = useContextMenu();
  return (
    <motion.li
      className="store-tile theme-tile"
      onContextMenu={(ev) => actions.menuOf && menu.open(ev, actions.menuOf(t))}
      initial={{ opacity: 0, y: 14 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.32, ease: [0.22, 1, 0.36, 1], delay: Math.min(index, 11) * 0.035 }}
      onPointerEnter={() => setHover(true)}
      onPointerLeave={() => setHover(false)}
      onFocus={() => setHover(true)}
      onBlur={() => setHover(false)}
    >
      <button type="button" className="store-tile__open" onClick={() => open(t)} aria-label={`Details of ${t.name}`}>
        <span className="store-tile__thumb theme-tile__thumb">
          <ThemeMiniature tokens={t.tokens} scheme={hover ? "dark" : "light"} />
          {hasLayout(t) && <span className="theme-tile__layout">Layout</span>}
        </span>
        <ThemeIcon url={t.icon} size={48} className="store-tile__icon" />
        <span className="store-tile__text">
          <span className="store-tile__name">
            <span className="store-tile__name-text">{t.name}</span>
            {t.jl && (
              <span className="store-badge" title="By JL Studios">
                JL
              </span>
            )}
          </span>
          <span className="store-tile__author">{t.author}</span>
          <span className="store-tile__summary">{t.summary}</span>
        </span>
      </button>
      <span className="store-tile__foot">
        <Dots t={t} />
        <span className="theme-tile__buttons">
          <TryButton t={t} actions={actions} />
          <ActionButton t={t} actions={actions} />
        </span>
      </span>
    </motion.li>
  );
}

export function ThemeGrid({ label, themes, actions, open }: { label: string; themes: StoreTheme[]; actions: ThemeActions; open: (t: StoreTheme) => void }) {
  return (
    <ul className="store-grid" aria-label={label}>
      {themes.map((t, i) => (
        <ThemeTile key={t.id} t={t} index={i} actions={actions} open={open} />
      ))}
    </ul>
  );
}
