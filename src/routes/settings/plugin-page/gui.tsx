import { ActionIcon, Anchor, Button, Switch } from "@mantine/core";
import { IconArrowLeft } from "@tabler/icons-react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "@/components/toast";
import { pickFile } from "@/lib/dialogs";
import { isAppError } from "@/lib/ipc";
import { usePlugins } from "@/hooks/use-plugins";
import { detectPluginApps, locatePluginApp, removePlugin, setPluginEnabled, undoRemovePlugin } from "@/lib/plugins";
import type { DetectStatus } from "@/types/generated/DetectStatus";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import { PluginControlView } from "./controls";
import { PluginDetails, PluginLog } from "./details";
import { SettingGroup, SettingRow } from "../row";
import "./styles.scss";

const open = (url: string) => void openUrl(url).catch(() => window.open(url, "_blank", "noopener"));

function StatusRow({ plugin, status, reload }: { plugin: PluginInfo; status: DetectStatus; reload: () => void }) {
  const locate = async () => {
    const exe = await pickFile({ title: `Locate ${status.name}` });
    if (typeof exe !== "string") return;
    try {
      await locatePluginApp(plugin.id, status.id, exe);
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : `Could not use that for ${status.name}.` });
    }
    reload();
  };
  return (
    <SettingRow
      label={status.path ? `${status.name} found${status.version ? ` · ${status.version}` : ""}` : `${status.name} not found`}
      hint={status.path ? <span className="plugin-page__path">{status.path}</span> : <span className="plugin-page__missing">{plugin.name} needs it for the options below.</span>}
    >
      <div className="plugin-page__status">
        {plugin.homepage && !status.path && (
          <Anchor component="button" type="button" size="sm" onClick={() => open(plugin.homepage as string)}>
            Get {status.name}
          </Anchor>
        )}
        <Button size="xs" variant="default" onClick={() => void locate()}>
          Locate…
        </Button>
      </div>
    </SettingRow>
  );
}

/**
 * Settings → Plugins → a plugin (Round 24 §8.3): header with the enable switch, one status row
 * per app it declares (found, version, path, *Locate…*), then its settings under their group
 * headings; a setting that `requires` an app is off until that app is found.
 */
export function PluginPage({ plugin: p, onBack, reload }: { plugin: PluginInfo; onBack: () => void; reload: () => void }) {
  const qc = useQueryClient();
  const key = ["plugin-detect", p.id, p.enabled];
  const { data, refetch } = useQuery({ queryKey: key, queryFn: () => detectPluginApps(p.id).catch(() => [] as DetectStatus[]), enabled: p.detectIds.length > 0 });
  const statuses = data ?? [];
  const found = (id: string) => statuses.some((s) => s.id === id && s.path);
  const afterLocate = () => {
    void qc.invalidateQueries({ queryKey: ["plugin-provider-choices"] });
    void refetch();
    reload();
  };
  const blocked = p.state.type === "needsNewerLumen" || p.state.type === "problem" || p.state.type === "stopped";
  const { developerMode } = usePlugins();
  const valueOf = (key: string) => p.sections.flatMap((sec) => sec.controls).find((c) => c.key === key)?.value;
  const remove = () =>
    removePlugin(p.id).then(
      (token) => {
        reload();
        onBack();
        toast({
          message: `Removed ${p.name}`,
          undo: () => void undoRemovePlugin(token).catch((e) => toast({ message: isAppError(e) ? e.message : "Could not bring it back." })).finally(reload),
        });
      },
      (e) => toast({ message: isAppError(e) ? e.message : "Could not remove that plugin." }),
    );
  const toggle = (on: boolean) =>
    setPluginEnabled(p.id, on)
      .catch((e) => toast({ message: isAppError(e) ? e.message : "Could not change that plugin." }))
      .finally(reload);

  return (
    <div className="plugin-page">
      <ActionIcon className="plugin-page__back" variant="subtle" color="gray" aria-label="Back to plugins" onClick={onBack}>
        <IconArrowLeft size={16} />
      </ActionIcon>
      <div className="plugin-page__header">
        {p.iconUrl ? <img className="plugin-page__icon" src={p.iconUrl} alt="" width={40} height={40} /> : <span className="plugin-page__icon plugin-page__icon--empty" aria-hidden />}
        <div className="plugin-page__title">
          <span className="plugin-page__name">{p.name}</span>
          <span className="plugin-page__meta">
            {p.version} · {p.author}
          </span>
        </div>
        <Switch aria-label={`Turn ${p.name} on`} checked={p.enabled && !blocked} disabled={blocked} onChange={(e) => void toggle(e.currentTarget.checked)} />
      </div>
      {statuses.length > 0 && (
        <SettingGroup>
          {statuses.map((s) => (
            <StatusRow key={s.id} plugin={p} status={s} reload={afterLocate} />
          ))}
        </SettingGroup>
      )}
      {p.sections.length === 0 ? (
        <SettingGroup>
          <SettingRow label="This plugin has no settings." />
        </SettingGroup>
      ) : (
        p.sections.map((sec, i) => (
          <SettingGroup key={`${sec.title ?? ""}:${i}`} title={sec.title ?? undefined}>
            {sec.description && <SettingRow hint={sec.description} />}
            {sec.controls.map((c, j) => (
              <PluginControlView key={c.key ?? `${c.kind}:${j}`} p={p} c={c} valueOf={valueOf} appFound={found} />
            ))}
          </SettingGroup>
        ))
      )}
      <SettingGroup title="Details">
        <SettingRow wide>
          <PluginDetails p={p} />
        </SettingRow>
      </SettingGroup>
      {developerMode && (
        <SettingGroup title="Log">
          <SettingRow wide>
            <PluginLog id={p.id} />
          </SettingRow>
        </SettingGroup>
      )}
      <SettingGroup>
        <SettingRow label="Remove this plugin" hint="You can undo it for a few seconds.">
          <Button size="xs" variant="default" color="red" aria-label={`Remove ${p.name}`} onClick={() => void remove()}>
            Remove
          </Button>
        </SettingRow>
      </SettingGroup>
    </div>
  );
}
