import { ActionIcon, Button, Switch } from "@mantine/core";
import { IconDots } from "@tabler/icons-react";
import { pickFolder } from "@/lib/dialogs";
import { useContextMenu } from "@/components/context-menu";
import { openPluginInstall, pickPluginToInstall, trustLine } from "@/components/plugin-install";
import { toast } from "@/components/toast";
import { setDeveloperMode, usePlugins } from "@/hooks/use-plugins";
import { useStoreStatus } from "@/hooks/use-store-status";
import { invoke, isAppError } from "@/lib/ipc";
import { storeSetAutoUpdate, storeUpdate } from "@/lib/plugin-store";
import { pluginMenu } from "@/lib/menus/plugins";
import { loadDevPlugin, reloadDevPlugin, removePlugin, setPluginEnabled, undoRemovePlugin } from "@/lib/plugins";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import type { PluginKind } from "@/types/generated/PluginKind";
import { useEffect, useState } from "react";
import { useSearchParams } from "react-router";
import { PluginPage } from "./plugin-page";
import { ProviderChoices } from "./provider-choices";
import { SettingGroup, SettingRow } from "./row";
import { useSearchQuery } from "./search";

const KIND_LABELS: Record<PluginKind, string> = {
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

function PluginRow({ plugin: p, developerMode, reload, openPage }: { plugin: PluginInfo; developerMode: boolean; reload: () => void; openPage?: (id: string) => void }) {
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
    { developerMode, dev: p.dev, hasSettings: p.sections.some((sec) => sec.controls.length > 0) },
    {
      details: () => openPage?.(p.id),
      settings: () => openPage?.(p.id),
      log: () => openPage?.(p.id),
      reload: () => void reloadDevPlugin(p.id).catch((e) => fail(e, "Could not reload that plugin.")).finally(reload),
      remove: () => void remove(),
    },
  );

  return (
    <SettingRow
      label={
        <span className="settings__plugin-name">
          {p.iconUrl ? <img className="settings__plugin-icon" src={p.iconUrl} alt="" width={28} height={28} /> : <span className="settings__plugin-icon settings__plugin-icon--empty" aria-hidden />}
          {/* Round 25 P3: every plugin has its own page (sections, details, turn off, remove) */}
          {openPage ? (
            <button type="button" className="settings__plugin-open" onClick={() => openPage(p.id)}>
              {p.name}
            </button>
          ) : (
            <span>{p.name}</span>
          )}
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
  // Round 25 P3: a plugin's page is `?section=plugins&plugin=<id>`, so search, the store's Installed tab and Back all use the address.
  const [params, setParams] = useSearchParams();
  const query = useSearchQuery();
  const pageId = params.get("plugin");
  const page = plugins.find((p) => p.id === pageId);
  const openPage = (id: string) => setParams({ section: "plugins", plugin: id }, { replace: true });
  if (page && !query) return <PluginPage plugin={page} onBack={() => setParams({ section: "plugins" }, { replace: true })} reload={reload} />;
  return (
    <>
      <PluginsView plugins={plugins} developerMode={developerMode} reload={reload} openPage={openPage} />
      {query && <PluginControlHits plugins={plugins} openPage={openPage} />}
      {!query && <StoreUpdatesGroup />}
    </>
  );
}

/** While Settings searches: every control's label and help, under its plugin, jumping to that plugin's page. */
function PluginControlHits({ plugins, openPage }: { plugins: PluginInfo[]; openPage: (id: string) => void }) {
  return (
    <>
      {plugins.map((p) => {
        const controls = p.sections.flatMap((sec) => sec.controls).filter((c) => c.kind !== "markdown" || c.label);
        if (controls.length === 0) return null;
        return (
          <SettingGroup key={p.id} title={p.name}>
            {controls.map((c, i) => (
              <SettingRow key={c.key ?? `${c.kind}:${i}`} label={c.label} hint={c.help}>
                <Button size="xs" variant="default" aria-label={`Open ${p.name}`} onClick={() => openPage(p.id)}>
                  Open
                </Button>
              </SettingRow>
            ))}
          </SettingGroup>
        );
      })}
    </>
  );
}

// Round 24 lane C: updates from the plugin store.
function StoreUpdatesGroup() {
  const { status, reload } = useStoreStatus();
  if (!status) return null;
  const { updates, updated, autoUpdate } = status;
  /** An update that asks for new permissions comes back as a package for the install sheet. */
  const updateAll = async () => {
    for (const u of updates) {
      try {
        const r = await storeUpdate(u.id);
        if (r.path) openPluginInstall(r.path);
      } catch (e) {
        fail(e, `Could not update ${u.name}.`);
      }
    }
    reload();
  };
  return (
    <SettingGroup title="Updates">
      <SettingRow label="Update plugins automatically" hint="Plugins from the store are kept up to date in the background. A new version that asks for more permissions waits for you.">
        <Switch
          aria-label="Update plugins automatically"
          checked={autoUpdate}
          onChange={(e) => void storeSetAutoUpdate(e.currentTarget.checked).catch((err) => fail(err, "Could not change this setting.")).finally(reload)}
        />
      </SettingRow>
      {updated.map((u) => (
        <SettingRow key={u.id} label={`Updated ${u.name} to ${u.version}`} />
      ))}
      {!autoUpdate && updates.length > 0 && (
        <SettingRow label={updates.length === 1 ? "1 update available" : `${updates.length} updates available`} hint={updates.map((u) => `${u.name} ${u.from} -> ${u.to}`).join(", ")}>
          <Button size="xs" variant="default" onClick={() => void updateAll()}>
            Update all
          </Button>
        </SettingRow>
      )}
    </SettingGroup>
  );
}

/** The page's content for a given list (also shown by /dev/ui, empty and filled). */
export function PluginsView({ plugins, developerMode, reload, openPage }: { plugins: PluginInfo[]; developerMode: boolean; reload: () => void; openPage?: (id: string) => void }) {

  const loadFolder = async () => {
    const folder = await pickFolder({ title: "Load a plugin folder" });
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
              <PluginRow key={p.id} plugin={p} developerMode={developerMode} reload={reload} openPage={openPage} />
            ))}
            <SettingRow label="Add a plugin">{installButton}</SettingRow>
            {/* Round 24 lane D: who answers file dialogs and Show in folder, when two plugins could */}
            <ProviderChoices plugins={plugins} />
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
        {developerMode && <LocalStoreRow />}
      </SettingGroup>
    </>
  );
}

/** Round 26 dev store: the store reads index.json, packages and images from a folder (plugins/store/). */
function LocalStoreRow() {
  const [folder, setFolder] = useState<string | null>(null);
  useEffect(() => {
    void invoke<string | null>("store_local_folder").then(setFolder, () => setFolder(null));
  }, []);
  const choose = async (next: string | null) => {
    try {
      await invoke<void>("store_set_local_folder", { folder: next });
      setFolder(next);
    } catch (e) {
      fail(e, "Could not use that folder.");
    }
  };
  const pick = async () => {
    const picked = await pickFolder({ title: "Use a local store folder" });
    if (typeof picked === "string") await choose(picked);
  };
  return (
    <SettingRow label="Store" hint={folder ? `Reads the store from ${folder}.` : "Reads the online store. A local folder holds index.json and dist/ (pack.mjs fills it)."}>
      {folder ? (
        <Button size="xs" variant="default" onClick={() => void choose(null)}>
          Use the online store
        </Button>
      ) : (
        <Button size="xs" variant="default" onClick={() => void pick()}>
          Use a local store folder…
        </Button>
      )}
    </SettingRow>
  );
}
