import { Button, Select } from "@mantine/core";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "@/components/toast";
import { pickFile, pickFolder } from "@/lib/dialogs";
import { isMac } from "@/components/window-controls";
import { useExternalApps } from "@/lib/external-apps";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { ExternalApp } from "@/types/generated/ExternalApp";
import type { Media } from "@/types/generated/Media";
import type { SendMode } from "@/types/generated/SendMode";
import { SettingGroup, SettingRow } from "./row";

const MEDIA: { media: Media; label: string; hint: string }[] = [
  { media: "photo", label: "Photos", hint: "What Edit in opens a photo with." },
  { media: "raw", label: "RAW files", hint: "What Edit in opens a RAW photo with." },
  { media: "video", label: "Videos", hint: "What Edit in opens a video with." },
];

const SEND: { value: SendMode; label: string }[] = [
  { value: "automatic", label: "Automatic" },
  { value: "rendered", label: "With Lumen adjustments" },
  { value: "originalFile", label: "Original file" },
];

/** The line Settings' list shows under *External apps*. */
export function externalAppsLine(apps: readonly ExternalApp[]): string | null {
  const found = apps.filter((a) => !a.custom).length;
  return found === 0 ? "None found" : `${found} found`;
}

/** What a `Developer` or `Video` app is given, as muted text: a sidecar writer works on a copy. */
const givenTo = (a: ExternalApp) => (a.writes === "sidecar" ? "Gets a copy" : "Opens the original");

function Icon({ app }: { app: ExternalApp }) {
  return <img className="settings__app-icon" src={app.iconUrl} alt="" width={20} height={20} />;
}

/**
 * Settings → Connections → External apps (Round 22 §9): the apps Lumen can hand a photo or video
 * to, which one *Edit in* uses for each kind, and what an editor is sent.
 */
export function ExternalAppsSection() {
  const qc = useQueryClient();
  const { apps, defaults } = useExternalApps();
  const refresh = () => qc.invalidateQueries({ queryKey: queryKeys.externalApps });

  const run = async (go: () => Promise<unknown>, failed: string) => {
    try {
      await go();
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : failed });
    }
    void refresh();
  };

  const choose = () =>
    run(async () => {
      const mac = isMac();
      const opts = {
        title: "Choose an app",
        filters: mac ? undefined : [{ name: "Apps", extensions: ["exe", "bat", "cmd", "sh", "AppImage"] }],
      };
      const path = mac ? await pickFolder(opts) : await pickFile(opts);
      if (typeof path === "string") await invoke("add_custom_app", { path });
    }, "That isn't an app Lumen can open photos with.");

  const installed = apps.filter((a) => !a.custom);
  const custom = apps.filter((a) => a.custom);
  const chooseButton = (
    <Button size="xs" variant="default" onClick={() => void choose()}>
      Choose another app…
    </Button>
  );

  if (apps.length === 0) {
    return (
      <SettingGroup>
        <SettingRow label="No photo or video editor was found on this computer.">{chooseButton}</SettingRow>
      </SettingGroup>
    );
  }

  return (
    <>
      <SettingGroup title="Defaults">
        {MEDIA.map(({ media, label, hint }) => {
          const fit = apps.filter((a) => a.accepts.includes(media));
          const current = fit.find((a) => a.id === defaults?.[media]);
          return (
            <SettingRow key={media} label={label} hint={hint}>
              {current && <Icon app={current} />}
              <Select
                aria-label={`Default app for ${label.toLowerCase()}`}
                value={current?.id ?? ""}
                data={[{ value: "", label: "None" }, ...fit.map((a) => ({ value: a.id, label: a.name }))]}
                onChange={(v) => {
                  if (v === null) return;
                  const id = v || null;
                  void run(() => invoke("set_default_app", { media, id }).then(() => qc.invalidateQueries({ queryKey: queryKeys.defaultApps })), "Could not save the default.");
                }}
              />
            </SettingRow>
          );
        })}
      </SettingGroup>
      {installed.length > 0 && (
        <SettingGroup title="Installed apps">
          {installed.map((a) => (
            <SettingRow
              key={a.id}
              label={
                <span className="settings__app-name">
                  <Icon app={a} />
                  {a.name}
                </span>
              }
              hint={a.addedBy ? `added by ${a.addedBy}` : undefined}
            >
              {a.kind === "editor" ? (
                <Select
                  aria-label={`What ${a.name} is sent`}
                  value={a.send}
                  data={SEND}
                  onChange={(v) => v !== null && void run(() => invoke("set_app_send_mode", { id: a.id, mode: v }), "Could not save that choice.")}
                />
              ) : (
                <span className="settings__value">{givenTo(a)}</span>
              )}
            </SettingRow>
          ))}
        </SettingGroup>
      )}
      <SettingGroup title="Other apps">
        {custom.map((a) => (
          <SettingRow
            key={a.id}
            label={
              <span className="settings__app-name">
                <Icon app={a} />
                {a.name}
              </span>
            }
            hint="Always given a copy."
          >
            <Button size="xs" variant="default" aria-label={`Remove ${a.name}`} onClick={() => void run(() => invoke("remove_custom_app", { id: a.id }), "Could not remove it.")}>
              Remove
            </Button>
          </SettingRow>
        ))}
        <SettingRow label="Another app" hint="Pick a program; Lumen gives it a copy of the photo.">
          {chooseButton}
        </SettingRow>
      </SettingGroup>
    </>
  );
}
