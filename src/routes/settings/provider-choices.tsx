import { Select } from "@mantine/core";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import type { ProviderChoice } from "@/types/generated/ProviderChoice";
import { SettingRow } from "./row";

const LABELS: Record<string, { label: string; hint: string }> = {
  "file-picker": { label: "File dialogs", hint: "More than one plugin offers its own file dialog. Pick the one Lumen opens." },
  reveal: { label: "Show in folder", hint: "More than one plugin can show a file in its folder. Pick the one Lumen uses." },
};

/**
 * Round 24 D2: a kind of provider (file dialogs, *Show in folder*) is answered by one plugin at a
 * time. A row appears only when two or more could answer it; the first installed answers by default.
 */
export function ProviderChoices({ plugins }: { plugins: PluginInfo[] }) {
  const qc = useQueryClient();
  // Turning a plugin or its switch on or off changes who can answer, so the list is asked again.
  const signature = plugins.map((p) => `${p.id}:${p.enabled}:${JSON.stringify(p.settings.map((s) => s.value))}`).join("|");
  const { data } = useQuery({
    queryKey: ["plugin-provider-choices", signature],
    queryFn: () => invoke<ProviderChoice[]>("plugin_provider_choices").catch(() => [] as ProviderChoice[]),
  });
  const rows = (data ?? []).filter((c) => c.options.length > 1 && LABELS[c.kind]);
  if (rows.length === 0) return null;
  return (
    <>
      {rows.map((c) => (
        <SettingRow key={c.kind} label={LABELS[c.kind].label} hint={LABELS[c.kind].hint}>
          <Select
            size="xs"
            aria-label={LABELS[c.kind].label}
            allowDeselect={false}
            data={c.options.map((o) => ({ value: o.pluginId, label: o.name }))}
            value={c.chosen}
            onChange={(id) => {
              if (!id) return;
              void invoke("set_provider_choice", { kind: c.kind, pluginId: id }).then(() =>
                qc.invalidateQueries({ queryKey: ["plugin-provider-choices"] }),
              );
            }}
          />
        </SettingRow>
      ))}
    </>
  );
}
