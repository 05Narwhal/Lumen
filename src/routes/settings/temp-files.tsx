import { useState } from "react";
import { Button, Checkbox } from "@mantine/core";
import { IconAlertTriangle } from "@tabler/icons-react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "@/components/toast";
import { invoke, isAppError } from "@/lib/ipc";
import { formatStorage } from "@/utils/format";
import type { TempKind } from "@/types/generated/TempKind";
import type { TempUsage } from "@/types/generated/TempUsage";
import { SettingGroup, SettingRow } from "./row";

const NAMES: Record<TempKind, { label: string; hint: string }> = {
  keptAside: { label: "Replaced by Repair", hint: "Originals Repair swapped for a good copy. They go to the Recycle Bin, not away for good." },
  unfinished: { label: "Unfinished copies", hint: "Half-written copies an interrupted import or repair left behind." },
  recovered: { label: "Recoveries not kept", hint: "What Recover read out of damaged files and nobody kept." },
  dngCache: { label: "Conversion cache", hint: "DNG conversions of RAW files; Lumen makes them again when it needs them." },
};

/** Above the groups when the temporary files have grown past the limit (the backend's `warn`). */
export function TempWarning({ usage }: { usage: TempUsage | undefined }) {
  if (!usage?.warn) return null;
  return (
    <p className="settings-storage__warn" role="status">
      <IconAlertTriangle size={16} aria-hidden />
      Temporary files use {formatStorage(usage.total)}.
    </p>
  );
}

/** *Temporary files*: one ticked row per kind that holds something, and the button that clears them. */
export function TempFilesGroup({ usage }: { usage: TempUsage }) {
  const qc = useQueryClient();
  const [off, setOff] = useState<ReadonlySet<TempKind>>(new Set());
  const chosen = usage.rows.filter((r) => !off.has(r.kind));
  const clear = useMutation({
    mutationFn: (kinds: TempKind[]) => invoke<number>("clear_temp", { kinds }),
    onSuccess: (freed, kinds) => {
      toast({ message: `Freed ${formatStorage(freed)}.${kinds.includes("keptAside") ? " The replaced originals are in the Recycle Bin." : ""}` });
      // Both the usage and the overview start with "storage".
      void qc.invalidateQueries({ queryKey: ["storage"] });
    },
    onError: (e) => toast({ message: isAppError(e) && e.code === "busy" ? "Wait for Repair to finish." : isAppError(e) ? e.message : "Lumen couldn't clear the temporary files." }),
  });
  return (
    <SettingGroup title={`Temporary files · ${formatStorage(usage.total)}`}>
      {usage.rows.map((r) => (
        <SettingRow key={r.kind} label={NAMES[r.kind].label} hint={NAMES[r.kind].hint}>
          <span className="settings-storage__temp">
            <span>{formatStorage(r.bytes)}</span>
            <Checkbox
              aria-label={NAMES[r.kind].label}
              checked={!off.has(r.kind)}
              onChange={(e) =>
                setOff((was) => {
                  const next = new Set(was);
                  if (e.currentTarget.checked) next.delete(r.kind);
                  else next.add(r.kind);
                  return next;
                })
              }
            />
          </span>
        </SettingRow>
      ))}
      <SettingRow>
        <Button size="xs" variant="default" disabled={chosen.length === 0} loading={clear.isPending} onClick={() => clear.mutate(chosen.map((r) => r.kind))}>
          Clear temporary files
        </Button>
      </SettingRow>
    </SettingGroup>
  );
}
