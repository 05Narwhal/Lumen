import { Button, Switch } from "@mantine/core";
import { Segmented } from "@/components/segmented";
import { useSettingValue, type useSettings } from "@/hooks/use-settings";
import { useUpdateStatus } from "@/hooks/use-updates";
import { formatRelative } from "@/utils/date";
import type { Settings } from "@/types/generated/Settings";
import type { UpdateChannel } from "@/types/generated/UpdateChannel";
import type { UpdateSettings } from "@/types/generated/UpdateSettings";
import type { UpdateState } from "@/types/generated/UpdateState";
import { SettingGroup, SettingRow } from "./row";

export { useUpdateNotices } from "@/hooks/use-updates";

type Update = ReturnType<typeof useSettings>["update"];

const DEFAULTS: UpdateSettings = { automatic: true, channel: "stable" };

const WARNING = "Alpha and Beta are development builds. They are unstable and may have bugs that Stable doesn't. Your library is copied before any upgrade.";

/** The state in one line (Round 17). */
export function updateLine(s: UpdateState): string {
  switch (s.state) {
    case "off":
      return "Updates are off in development builds.";
    case "notSetUp":
      return "Updates are not set up in this build.";
    case "idle":
      return "Not checked yet.";
    case "checking":
      return "Checking…";
    case "upToDate":
      return `Up to date. Checked ${formatRelative(s.checkedAt)}.`;
    case "available":
      return `${s.version} is available.`;
    case "downloading":
      return `Downloading ${s.version} · ${s.total > 0 ? Math.floor((s.done / s.total) * 100) : 0} %`;
    case "ready":
      return s.attempts > 0
        ? `Ready: ${s.version}. The last try to install it didn't finish; Lumen tries again when you close it.`
        : `Ready: ${s.version} installs when you close Lumen.`;
    case "failed":
      switch (s.why) {
        case "offline":
          return "Could not check: this computer seems to be offline. Lumen tries again later.";
        case "noServer":
          return "Could not check: Lumen can't reach the update server. It tries again later.";
        case "couldNotCheck":
          return "Could not check right now. Lumen tries again later.";
        case "badSignature":
          return "The downloaded update could not be verified, so it was deleted.";
        case "noSpace":
          return "Not enough disk space to download the update.";
        default:
          return "The update did not finish. Lumen tries again later.";
      }
  }
}

/** Settings → Updates (Round 17). */
export function UpdatesSection({ settings, update }: { settings: Settings; update: Update }) {
  const u = settings.updates ?? DEFAULTS;
  const [automatic, setAutomatic] = useSettingValue(u.automatic);
  const [channel, setChannel] = useSettingValue<UpdateChannel>(u.channel);
  const { status, check, download, installNow } = useUpdateStatus();
  const state = status?.state;
  const set = (patch: Partial<UpdateSettings>) => update((s) => ({ ...s, updates: { ...(s.updates ?? DEFAULTS), ...patch } }));
  const working = state?.state === "off" || state?.state === "notSetUp" ? null : state;
  const notes = state?.state === "ready" || state?.state === "available" ? state.notes : status?.whatsNew?.notes;
  const nothingYet = channel === "stable" && (state?.state === "upToDate" || state?.state === "idle");

  return (
    <>
      <SettingGroup>
        <SettingRow label="Version">
          <span className="settings__value">Lumen {status?.version ?? "…"}</span>
        </SettingRow>
        <SettingRow label="Status" hint={state ? updateLine(state) : ""}>
          {working?.state === "ready" ? (
            <Button size="xs" onClick={() => void installNow()}>
              Install now
            </Button>
          ) : working?.state === "available" ? (
            <Button size="xs" onClick={() => void download()}>
              Download
            </Button>
          ) : working && working.state !== "downloading" ? (
            <Button size="xs" variant="default" loading={working.state === "checking"} onClick={() => void check()}>
              Check for updates
            </Button>
          ) : null}
        </SettingRow>
        {notes && (
          <SettingRow label={state?.state === "ready" || state?.state === "available" ? "What's in it" : "What's new"} wide>
            <p className="settings__notes">{notes}</p>
          </SettingRow>
        )}
      </SettingGroup>
      <SettingGroup title="How updates arrive">
        <SettingRow label="Automatic updates" hint="Downloads in the background and installs when you close Lumen. Off, Lumen checks only when you ask.">
          <Switch
            aria-label="Automatic updates"
            checked={automatic}
            onChange={(e) => {
              const v = e.currentTarget.checked;
              setAutomatic(v);
              set({ automatic: v });
            }}
          />
        </SettingRow>
        <SettingRow
          label="Update channel"
          hint={nothingYet ? "Stable gets its first build later. Choose Alpha to get updates now." : "A channel takes its own builds and every more stable one."}
        >
          <Segmented<UpdateChannel>
            label="Update channel"
            value={channel}
            onChange={(v) => {
              setChannel(v);
              set({ channel: v });
            }}
            data={[
              { label: "Stable", value: "stable" },
              { label: "Beta", value: "beta" },
              { label: "Alpha", value: "alpha" },
            ]}
          />
        </SettingRow>
        {channel !== "stable" && (
          <SettingRow wide>
            <p className="settings__unstable" role="note">
              {WARNING}
            </p>
          </SettingRow>
        )}
        <SettingRow hint="Checks ask GitHub for the latest version. Lumen sends no identifier and no usage data." />
      </SettingGroup>
    </>
  );
}
