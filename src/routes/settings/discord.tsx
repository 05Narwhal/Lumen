import { Switch } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { useSettingValue, type useSettings } from "@/hooks/use-settings";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { DiscordInfo } from "@/types/generated/DiscordInfo";
import type { DiscordStatus } from "@/types/generated/DiscordStatus";
import type { Settings } from "@/types/generated/Settings";
import { SettingGroup, SettingRow } from "./row";

type Update = ReturnType<typeof useSettings>["update"];

const OFF: DiscordStatus = { enabled: false, showActivity: true, showElapsed: true };

export const discordInfoQuery = {
  queryKey: [...queryKeys.settings, "discord-info"] as const,
  queryFn: () => invoke<DiscordInfo>("discord_info"),
};

/** The status line: off, on but not shown yet, or shown. */
export function statusLine(info: DiscordInfo | undefined, enabled: boolean): string {
  if (!enabled) return "Off. Nothing is shared.";
  if (!info) return "";
  return info.state === "showing" ? "Showing in Discord." : "Waiting for Discord. Nothing is shown until it is running.";
}

/** Settings → Discord status (Round 17): opt-in, and only ever a fixed word for what Lumen is doing. */
export function DiscordSection({ settings, update }: { settings: Settings; update: Update }) {
  const d = settings.discord ?? OFF;
  const [enabled, setEnabled] = useSettingValue(d.enabled);
  const info = useQuery({ ...discordInfoQuery, refetchInterval: enabled ? 3000 : false });
  const set = (patch: Partial<DiscordStatus>) => {
    update((s) => ({ ...s, discord: { ...(s.discord ?? OFF), ...patch } }));
    void info.refetch();
  };

  return (
    <SettingGroup>
      <SettingRow
        label="Show Lumen in my Discord status"
        hint="Discord shows “Competing in Lumen” beside any game. Only the words below are sent, and only to your own Discord on this computer. Never a photo, name, place or date."
      >
        <Switch
          aria-label="Show Lumen in my Discord status"
          checked={enabled}
          onChange={(e) => {
            const v = e.currentTarget.checked;
            setEnabled(v);
            set({ enabled: v });
          }}
        />
      </SettingRow>
      <SettingRow label="Show what I'm doing" hint="Browsing the library, Looking through photos, Developing a photo or Watching a slideshow.">
        <Switch
          aria-label="Show what I'm doing"
          disabled={!enabled}
          checked={d.showActivity}
          onChange={(e) => set({ showActivity: e.currentTarget.checked })}
        />
      </SettingRow>
      <SettingRow label="Show elapsed time" hint="How long Lumen has been open.">
        <Switch
          aria-label="Show elapsed time"
          disabled={!enabled}
          checked={d.showElapsed}
          onChange={(e) => set({ showElapsed: e.currentTarget.checked })}
        />
      </SettingRow>
      <SettingRow label="Status" hint={statusLine(info.data, enabled)} />
      {enabled && info.data && <SettingRow label="What others see" hint={info.data.preview} />}
    </SettingGroup>
  );
}
