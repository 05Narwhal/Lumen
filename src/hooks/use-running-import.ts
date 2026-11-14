import { useQuery } from "@tanstack/react-query";
import { BATCH_RUNNING, useImport } from "@/hooks/use-import";
import { invoke } from "@/lib/ipc";
import { jobsStore } from "@/lib/jobs-store";

/**
 * The import running from one card (`VolumeInfo.key`) or phone (`ios:<udid>`), found again
 * after its page was left: the batch id, its progress (0–1) and the line the activity ring
 * shows. A device imports one batch at a time, so its Import buttons wait while this is set.
 */
export function useRunningImport(key: string | null | undefined) {
  const { batches } = useImport();
  const running = batches.filter((b) => b.state === BATCH_RUNNING).map((b) => b.id);
  const q = useQuery({
    queryKey: ["running-import", key ?? "", running.join(",")],
    queryFn: () => invoke<number | null>("running_import", { key }),
    enabled: !!key && running.length > 0,
  });
  const batchId = running.length > 0 && q.data != null && running.includes(q.data) ? q.data : null;
  const job = jobsStore.use().jobs.find((j) => j.id === `import:${batchId}`);
  return { batchId, progress: job?.progress ?? 0, label: job?.label ?? null };
}

/** The Import button while a device's import runs: "Importing… 42%". */
export function importingLabel(progress: number) {
  return `Importing… ${Math.round(progress * 100)}%`;
}
