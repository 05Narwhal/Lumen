import { Button } from "@mantine/core";
import { IconPuzzle } from "@tabler/icons-react";
import { motion } from "motion/react";
import type { CSSProperties } from "react";
import type { StoreEntry } from "@/types/generated/StoreEntry";
import { useContextMenu, type MenuItem } from "@/components/context-menu";
import type { CardAction } from "./detail";

/** A kind as one word on a tile's footer. */
const KIND_WORD: Record<string, string> = { tools: "Tool", presets: "Looks", importer: "Importer", apps: "App", provider: "Provider", prompts: "Prompts", theme: "Theme" };
export const kindWord = (kinds: string[]) => (kinds[0] ? (KIND_WORD[kinds[0]] ?? kinds[0]) : "Plugin");

/** A stable cover swatch (1–10) per plugin, for a tile with neither screenshot nor icon. */
export function swatchOf(id: string): number {
  let h = 0;
  for (const c of id) h = (h * 31 + c.charCodeAt(0)) | 0;
  return (Math.abs(h) % 10) + 1;
}

export interface Actions {
  actionOf: (e: StoreEntry) => CardAction;
  busyId: string | null;
  onAction: (e: StoreEntry) => void;
  /** Right-click menu items for a tile. */
  menuOf?: (e: StoreEntry) => MenuItem[];
}

export function StoreIcon({ url, size, className = "" }: { url: string | null; size: number; className?: string }) {
  return url ? (
    <img className={`store-icon ${className}`} src={url} alt="" width={size} height={size} loading="lazy" draggable={false} />
  ) : (
    <span className={`store-icon store-icon--empty ${className}`} aria-hidden style={{ width: size, height: size }}>
      <IconPuzzle size={Math.round(size / 2)} stroke={1.6} />
    </span>
  );
}

export function ActionButton({ e, actions, size = "xs" }: { e: StoreEntry; actions: Actions; size?: "xs" | "sm" }) {
  const a = actions.actionOf(e);
  return (
    <Button
      size={size}
      radius="xl"
      variant={a.label === "Get" ? "light" : a.label === "Update" ? "filled" : "default"}
      disabled={a.disabled || (actions.busyId !== null && actions.busyId !== e.id)}
      loading={actions.busyId === e.id}
      onClick={() => actions.onAction(e)}
      aria-label={`${a.label} ${e.name}`}
    >
      {a.label}
    </Button>
  );
}

/**
 * The thumbnail: the first screenshot, else the icon blown up and blurred (a wash in the icon's
 * own colours, painted by the browser, nothing decoded here), else a cover swatch.
 */
function Thumb({ e }: { e: StoreEntry }) {
  const shot = e.screenshots[0];
  return (
    <span className="store-tile__thumb" style={{ "--tile-swatch": `var(--cover-swatch-${swatchOf(e.id)})` } as CSSProperties}>
      {shot ? <img className="store-tile__shot" src={shot} alt={`${e.name} screenshot`} loading="lazy" draggable={false} /> : e.icon ? <img className="store-tile__wash" src={e.icon} alt="" aria-hidden draggable={false} /> : null}
    </span>
  );
}

/** Every tile the same size: thumbnail, icon over its edge, name, author, two lines of summary, kind and action. */
export function Tile({ e, index, actions, open }: { e: StoreEntry; index: number; actions: Actions; open: (e: StoreEntry) => void }) {
  const menu = useContextMenu();
  return (
    <motion.li
      className="store-tile"
      onContextMenu={(ev) => actions.menuOf && menu.open(ev, actions.menuOf(e))}
      initial={{ opacity: 0, y: 14 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.32, ease: [0.22, 1, 0.36, 1], delay: Math.min(index, 11) * 0.035 }}
    >
      <button type="button" className="store-tile__open" onClick={() => open(e)} aria-label={`Details of ${e.name}`}>
        <Thumb e={e} />
        <StoreIcon url={e.icon} size={48} className="store-tile__icon" />
        <span className="store-tile__text">
          <span className="store-tile__name">
            <span className="store-tile__name-text">{e.name}</span>
            {e.jl && (
              <span className="store-badge" title="By JL Studios">
                JL
              </span>
            )}
          </span>
          <span className="store-tile__author">{e.author}</span>
          <span className="store-tile__summary">{e.summary}</span>
        </span>
      </button>
      <span className="store-tile__foot">
        <span className="store-tile__kind">{kindWord(e.kinds)}</span>
        <ActionButton e={e} actions={actions} />
      </span>
    </motion.li>
  );
}

export function TileGrid({ label, entries, actions, open }: { label: string; entries: StoreEntry[]; actions: Actions; open: (e: StoreEntry) => void }) {
  return (
    <ul className="store-grid" aria-label={label}>
      {entries.map((e, i) => (
        <Tile key={e.id} e={e} index={i} actions={actions} open={open} />
      ))}
    </ul>
  );
}
