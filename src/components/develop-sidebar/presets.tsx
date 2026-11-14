import { IconDots, IconPlus } from "@tabler/icons-react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useContextMenu } from "@/components/context-menu";
import { PanelChoice } from "@/components/panel-choice";
import { toast } from "@/components/toast";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { mergePanels, withDevelop } from "@/lib/develop";
import { cameraRawFolderQuery, importFromLightroom, pickLightroomPresets, presetsKey } from "@/lib/lightroom-presets";
import { pluginPresetMenu, presetsMenu } from "@/lib/menus";
import { invoke, isAppError } from "@/lib/ipc";
import type { Preset } from "@/types/generated/Preset";
import { InlineName, Section } from "./parts";

export { presetsKey };

const failed = (e: unknown, what: string) => toast({ message: isAppError(e) ? e.message : what });

/**
 * Presets (spec §1): Lumen's six, then the user's. Hovering one previews it on the canvas
 * without changing anything; clicking applies it as one step. A preset sets only the panels
 * it was saved with.
 */
export function Presets({ session }: { session: DevelopSession }) {
  const qc = useQueryClient();
  const presets = useQuery({ queryKey: presetsKey, queryFn: () => invoke<Preset[]>("list_presets") });
  const lightroomFolder = useQuery(cameraRawFolderQuery).data ?? null;
  const { open: openMenu } = useContextMenu();
  const [saving, setSaving] = useState(false);
  const [renaming, setRenaming] = useState<number | null>(null);
  const refresh = () => qc.invalidateQueries({ queryKey: presetsKey });

  const applied = (p: Preset) => withDevelop(session.draft, mergePanels(session.develop, p.develop, p.panels));
  const apply = (p: Preset) => {
    session.setHover(null);
    session.apply((d) => mergePanels(d, p.develop, p.panels), `Preset: ${p.name}`);
  };
  const save = async ({ name, panels }: { name: string; panels: Preset["panels"] }) => {
    try {
      await invoke<Preset>("save_preset", { name, develop: session.develop, panels });
      setSaving(false);
      await refresh();
      toast({ message: `Saved preset “${name}”` });
    } catch (e) {
      failed(e, "The preset could not be saved.");
    }
  };
  const rename = async (p: Preset, name: string | null) => {
    setRenaming(null);
    if (!name || name === p.name) return;
    try {
      await invoke("rename_preset", { id: p.id, name });
      await refresh();
    } catch (e) {
      failed(e, "The preset could not be renamed.");
    }
  };
  const duplicate = async (p: Preset) => {
    try {
      await invoke<Preset>("duplicate_plugin_preset", { key: p.key });
      await refresh();
      toast({ message: `Saved preset “${p.name}” to your presets` });
    } catch (e) {
      failed(e, "The preset could not be duplicated.");
    }
  };
  const remove = async (p: Preset) => {
    try {
      await invoke("delete_preset", { id: p.id });
      await refresh();
      toast({
        message: `Deleted preset “${p.name}”`,
        undo: () => void invoke("save_preset", { name: p.name, develop: p.develop, panels: p.panels }).then(refresh),
      });
    } catch (e) {
      failed(e, "The preset could not be deleted.");
    }
  };

  const all = presets.data ?? [];
  const row = (p: Preset) => (
    <li key={p.key}>
      {renaming === p.id && p.source === "mine" ? (
        <InlineName value={p.name} label="Preset name" onDone={(n) => void rename(p, n)} />
      ) : (
        <button
          type="button"
          className="develop-sidebar__row"
          onMouseEnter={() => session.setHover(applied(p))}
          onMouseLeave={() => session.setHover(null)}
          onFocus={() => session.setHover(applied(p))}
          onBlur={() => session.setHover(null)}
          onClick={() => apply(p)}
          onContextMenu={
            p.builtin
              ? undefined
              : typeof p.source === "object"
                ? (e) => openMenu(e, pluginPresetMenu({ duplicate: () => void duplicate(p) }))
                : (e) =>
                    openMenu(e, [
                      { label: "Rename", onSelect: () => setRenaming(p.id) },
                      { label: "Delete", danger: true, onSelect: () => void remove(p) },
                    ])
          }
        >
          {p.name}
        </button>
      )}
    </li>
  );
  const builtins = all.filter((p) => p.builtin);
  const mine = all.filter((p) => p.source === "mine");
  // One group per plugin, in the order the list came (plugin id, then file).
  const groups: { plugin: string; title: string; items: Preset[] }[] = [];
  for (const p of all) {
    if (typeof p.source !== "object") continue;
    const { plugin, name } = p.source.plugin;
    const group = groups.find((g) => g.plugin === plugin) ?? groups[groups.push({ plugin, title: name, items: [] }) - 1];
    group.items.push(p);
  }
  return (
    <Section
      title="Presets"
      action={
        <>
          <button type="button" className="develop-sidebar__icon" aria-label="Save preset…" title="Save the current settings as a preset" onClick={() => setSaving(true)}>
            <IconPlus size={14} />
          </button>
          <button
            type="button"
            className="develop-sidebar__icon"
            aria-label="Preset actions"
            title="More preset actions"
            onClick={(e) =>
              openMenu(
                e,
                presetsMenu(
                  { lightroomFolder },
                  {
                    save: () => setSaving(true),
                    importFiles: () => void pickLightroomPresets("files", qc),
                    importFolder: () => void pickLightroomPresets("folder", qc),
                    importFromLightroom: () => void importFromLightroom(qc),
                  },
                ),
              )
            }
          >
            <IconDots size={14} />
          </button>
        </>
      }
    >
      <ul className="develop-sidebar__list" aria-label="Lumen presets">
        {builtins.map(row)}
      </ul>
      {mine.length > 0 && (
        <>
          <div className="develop-sidebar__subhead">Your presets</div>
          <ul className="develop-sidebar__list" aria-label="Your presets">
            {mine.map(row)}
          </ul>
        </>
      )}
      {groups.map((g) => (
        <div key={g.plugin}>
          <div className="develop-sidebar__subhead">{g.title}</div>
          <ul className="develop-sidebar__list" aria-label={g.title}>
            {g.items.map(row)}
          </ul>
        </div>
      ))}
      <PanelChoice opened={saving} title="Save preset" confirmLabel="Save preset" name="" onConfirm={(c) => void save(c)} onClose={() => setSaving(false)} />
    </Section>
  );
}
