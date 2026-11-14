import type { RepairResult } from "@/types/generated/RepairResult";
import type { RepairStatus } from "@/types/generated/RepairStatus";
import type { Job } from "./jobs-store";

export const REPAIR_JOB = "repair";

/** A re-import's result line and toast (Round 21 B5): *n imported · n repaired · n sidecars · n skipped*. */
export function repairImportedMessage(r: RepairResult): string {
  const n = (v: number) => v.toLocaleString("en-US");
  const parts = [`${n(r.imported)} imported`, `${n(r.repaired)} repaired`, `${n(r.sidecars)} ${r.sidecars === 1 ? "sidecar" : "sidecars"}`, `${n(r.skipped)} skipped`];
  return (r.cancelled ? ["Stopped", ...parts] : parts).join(" · ");
}

/** A running repair check's line in the activity ring (Round 21 B4). */
export function repairProgressJob(s: RepairStatus): Job {
  const listing = s.phase === "listing" || s.total === 0;
  return {
    id: REPAIR_JOB,
    kind: "maintenance",
    label: "Checking a source against the library",
    progress: listing ? 0 : Math.min(1, s.done / s.total),
    done: s.done,
    total: s.total,
    state: "running",
    detail: listing ? "Looking at what is on it" : `${s.done.toLocaleString("en-US")} of ${s.total.toLocaleString("en-US")} files`,
  };
}
