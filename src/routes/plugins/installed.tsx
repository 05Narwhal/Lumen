import { Button } from "@mantine/core";
import { IconPuzzle } from "@tabler/icons-react";
import { useNavigate } from "react-router";
import { useContextMenu, type MenuItem } from "@/components/context-menu";
import { EmptyState } from "@/components/empty-state";
import { trustLine } from "@/components/plugin-install";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import type { StoreUpdate } from "@/types/generated/StoreUpdate";

/** Installed: the same plugins as Settings → Plugins, with their update state. */
export function Installed({
  plugins,
  updates,
  autoUpdate,
  busyId,
  onUpdate,
  onUpdateAll,
  onDiscover,
  menuOf,
}: {
  plugins: PluginInfo[];
  updates: StoreUpdate[];
  autoUpdate: boolean;
  busyId: string | null;
  onUpdate: (id: string) => void;
  onUpdateAll: () => void;
  onDiscover: () => void;
  menuOf?: (e: { id: string; name: string }) => MenuItem[];
}) {
  const navigate = useNavigate();
  const menu = useContextMenu();
  if (plugins.length === 0) {
    return <EmptyState icon={<IconPuzzle />} title="No plugins yet." description="Plugins add themes, presets, importers and tools." action={<Button variant="default" onClick={onDiscover}>Discover</Button>} />;
  }
  return (
    <>
      {!autoUpdate && updates.length > 1 && (
        <div className="store-banner">
          <span>{updates.length} updates available</span>
          <Button size="xs" onClick={onUpdateAll}>Update all</Button>
        </div>
      )}
      <ul className="store-list" aria-label="Installed plugins">
        {plugins.map((p) => {
          const u = updates.find((x) => x.id === p.id);
          return (
            <li key={p.id} className="store-row" onContextMenu={(ev) => menuOf && menu.open(ev, menuOf(p))}>
              <button type="button" className="store-card__body" onClick={() => navigate(`/plugins/${encodeURIComponent(p.id)}`)} aria-label={`Details of ${p.name}`}>
                {p.iconUrl ? <img className="store-card__icon" src={p.iconUrl} alt="" width={44} height={44} /> : <span className="store-card__icon store-card__icon--empty" aria-hidden><IconPuzzle size={22} /></span>}
                <span className="store-card__text">
                  <span className="store-card__name">
                    {p.name} <span className="store-row__version">{p.version}</span>
                  </span>
                  <span className="store-card__author">{trustLine(p.trust)}</span>
                  {u && <span className="store-row__update">{`Update to ${u.to}${u.needsReview ? " (asks for new permissions)" : ""}`}</span>}
                </span>
              </button>
              {u && (
                <Button size="xs" loading={busyId === p.id} onClick={() => onUpdate(p.id)} aria-label={`Update ${p.name}`}>
                  Update
                </Button>
              )}
              {(p.settings.length > 0 || p.detectIds.length > 0) && (
                <Button size="xs" variant="default" onClick={() => navigate(`/settings?section=plugins&plugin=${encodeURIComponent(p.id)}`)} aria-label={`Settings of ${p.name}`}>
                  Settings
                </Button>
              )}
            </li>
          );
        })}
      </ul>
    </>
  );
}
