import { ActionIcon, Button, Switch } from "@mantine/core";
import { IconDots } from "@tabler/icons-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useContextMenu } from "@/components/context-menu";
import { pickPluginToInstall, trustLine } from "@/components/plugin-install";
import { toast } from "@/components/toast";
import { setDeveloperMode, usePlugins } from "@/hooks/use-plugins";
import { isAppError } from "@/lib/ipc";
import { pluginMenu } from "@/lib/menus/plugins";
import { loadDevPlugin, reloadDevPlugin, removePlugin, setPluginEnabled, undoRemovePlugin } from "@/lib/plugins";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import type { PluginKind } from "@/types/generated/PluginKind";
import { openPluginDetails, PluginDetailsHost } from "./plugin-details";
import { SettingGroup, SettingRow } from "./row";

const KIND_LABELS: Record<PluginKind, string> = {
  theme: "Theme",
  presets: "Presets",
  apps: "Apps",
  prompts: "Prompts",
  importer: "Importer",
  tools: "Tools",
};

const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });

/** The line under a plugin's name: its trust, and why it isn't running when it isn't. */
function statusLine(p: PluginInfo): string {
  const s = p.state;
  if (s.type === "needsNewerLumen") return "Needs a newer Lumen";
  if (s.type === "problem") return s.reason;
  if (s.type === "stopped") return `Stopped working: ${s.reason}`;
  return trustLine(p.trust);
}

function PluginRow({ plugin: p, developerMode, reload }: { plugin: PluginInfo; developerMode: boolean; reload: () => void }) {
  const { open } = useContextMenu();
  const blocked = p.state.type === "needsNewerLumen" || p.state.type === "problem" || p.state.type === "stopped";

  const toggle = (on: boolean) => setPluginEnabled(p.id, on).catch((e) => fail(e, "Could not change that plugin.")).finally(reload);
  const remove = () =>
    removePlugin(p.id).then(
      (token) => {
        reload();
        toast({
          message: `Removed ${p.name}`,
          undo: () => void undoRemovePlugin(token).catch((e) => fail(e, "Could not bring it back.")).finally(reload),
        });
      },
      (e) => fail(e, "Could not remove that plugin."),
    );
  const items = pluginMenu(
    { developerMode, dev: p.dev, hasSettings: p.settings.length > 0 },
    {
      details: () => openPluginDetails(p.id, "details"),
      settings: () => openPluginDetails(p.id, "settings"),
      log: () => openPluginDetails(p.id, "log"),
      reload: () => void reloadDevPlugin(p.id).catch((e) => fail(e, "Could not reload that plugin.")).finally(reload),
      remove: () => void remove(),
    },
  );

  return (
    <SettingRow
      label={
        <span className="settings__plugin-name">
          {p.iconUrl ? <img className="settings__plugin-icon" src={p.iconUrl} alt="" width={28} height={28} /> : <span className="settings__plugin-icon settings__plugin-icon--empty" aria-hidden />}
          <span>{p.name}</span>
          <span className="settings__plugin-version">{p.version}</span>
          {p.dev && <span className="settings__plugin-badge">Dev</span>}
        </span>
      }
      hint={
        <>
          <span className={blocked ? "settings__plugin-problem" : undefined}>{statusLine(p)}</span>{" "}
          {p.kinds.map((k) => (
            <span key={k} className="settings__plugin-chip">
              {KIND_LABELS[k]}
            </span>
          ))}
        </>
      }
    >
      <Switch aria-label={`Turn ${p.name} on`} checked={p.enabled && !blocked} disabled={blocked} onChange={(e) => void toggle(e.currentTarget.checked)} />
      <ActionIcon variant="subtle" color="gray" aria-label={`More for ${p.name}`} onClick={(e) => open(e, items)}>
        <IconDots size={16} />
      </ActionIcon>
    </SettingRow>
  );
}

/** Settings → Plugins (Round 23 §3): what is installed, a switch and a ⋯ menu each, Developer mode below. */
export function PluginsSection() {
  const { plugins, developerMode, reload } = usePlugins();
  return (
    <>
      <PluginDetailsHost />
      <PluginsView plugins={plugins} developerMode={developerMode} reload={reload} />
    </>
  );
}

/** The page's content for a given list (also shown by /dev/ui, empty and filled). */
export function PluginsView({ plugins, developerMode, reload }: { plugins: PluginInfo[]; developerMode: boolean; reload: () => void }) {

  const loadFolder = async () => {
    const folder = await openDialog({ directory: true, multiple: false, title: "Load a plugin folder" });
    if (typeof folder !== "string") return;
    try {
      await loadDevPlugin(folder);
    } catch (e) {
      fail(e, "Could not load that folder.");
    }
    reload();
  };
  const installButton = (
    <Button size="xs" variant="default" onClick={() => void pickPluginToInstall()}>
      Install from file…
    </Button>
  );

  return (
    <>
      <SettingGroup>
        {plugins.length === 0 ? (
          <SettingRow label="No plugins yet.">{installButton}</SettingRow>
        ) : (
          <>
            {plugins.map((p) => (
              <PluginRow key={p.id} plugin={p} developerMode={developerMode} reload={reload} />
            ))}
            <SettingRow label="Add a plugin">{installButton}</SettingRow>
          </>
        )}
      </SettingGroup>
      <SettingGroup title="For plugin makers">
        <SettingRow label="Developer mode" hint="Lets you load a plugin from a folder and read each plugin's log. Off unless you make plugins.">
          <Switch aria-label="Developer mode" checked={developerMode} onChange={(e) => void setDeveloperMode(e.currentTarget.checked).catch((err) => fail(err, "Could not change Developer mode.")).finally(reload)} />
        </SettingRow>
        {developerMode && (
          <SettingRow label="Load a folder" hint="Runs an unpacked plugin straight from its folder, unsigned.">
            <Button size="xs" variant="default" onClick={() => void loadFolder()}>
              Load a folder…
            </Button>
          </SettingRow>
        )}
      </SettingGroup>
    </>
  );
}
