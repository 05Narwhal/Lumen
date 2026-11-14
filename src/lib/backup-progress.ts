import type { BackupProgress } from "@/types/generated/BackupProgress";
import type { BackupRun } from "@/types/generated/BackupRun";
import { formatBytes } from "@/utils/format";
import type { Job } from "./jobs-store";

/** A running backup's line in the activity ring (Round 5A), measured in bytes. */
export function backupProgressJob(p: BackupProgress): Job {
  return {
    id: "backup",
    kind: "backup",
    label: "Backing up",
    progress: p.totalBytes > 0 ? p.doneBytes / p.totalBytes : 1,
    // Weighed in the ring by files, the unit its line counts in.
    done: p.totalBytes > 0 ? (p.doneBytes / p.totalBytes) * p.totalFiles : p.totalFiles,
    total: p.totalFiles,
    state: "running",
    detail: `${p.doneFiles.toLocaleString("en-US")} of ${p.totalFiles.toLocaleString("en-US")} files`,
  };
}

/** How a backup ended, in one sentence for its toast and the Backup settings. */
export function backupFinishedMessage(run: BackupRun): string {
  if (run.error) return run.error;
  const r = run.report;
  if (!r) return "Backup finished";
  if (r.cancelled) return "Backup stopped; it picks up from here next time";
  const done = r.copied === 0 ? "Backup is up to date" : `Backed up ${r.copied.toLocaleString("en-US")} new ${r.copied === 1 ? "file" : "files"} (${formatBytes(r.copiedBytes)})`;
  return r.failed.length > 0 ? `${done}; ${r.failed.length} could not be copied and will be tried again` : done;
}
