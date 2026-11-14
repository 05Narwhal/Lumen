import type { CheckKind } from "@/types/generated/CheckKind";
import type { CheckProgressEvent } from "@/types/generated/CheckProgressEvent";
import type { CheckFinished } from "@/types/generated/CheckFinished";
import type { Job } from "./jobs-store";

/** A running check's line in the activity ring (Round 19 C4), measured in bytes. */
export function checkProgressJob(e: CheckProgressEvent): Job {
  const p = e.progress;
  return {
    id: "check",
    kind: "maintenance",
    label: e.kind === "folder" ? "Checking a folder" : "Checking the library",
    progress: p.totalBytes > 0 ? p.doneBytes / p.totalBytes : 1,
    // Weighed in the ring by files, the unit its line counts in.
    done: p.totalBytes > 0 ? (p.doneBytes / p.totalBytes) * p.totalFiles : p.totalFiles,
    total: p.totalFiles,
    state: "running",
    detail: `${p.doneFiles.toLocaleString("en-US")} of ${p.totalFiles.toLocaleString("en-US")} files`,
  };
}

/** How a check ended, in one sentence for its toast. */
export function checkFinishedMessage(f: CheckFinished): string {
  const what = f.kind === "folder" ? "Folder check" : "Library check";
  if (f.error) return f.error;
  return f.cancelled ? `${what} stopped` : `${what} finished`;
}

export const CHECK_KIND_LABEL: Record<CheckKind, string> = { folder: "folder", library: "library" };

/** One sentence for a reason code a check gives (the Rust side has the same words for the report). */
export function checkReason(code: string | null): string {
  switch (code) {
    case "not_imported":
      return "Never imported";
    case "in_trash":
      return "In Lumen's trash";
    case "copy_missing":
      return "The library's copy is missing";
    case "copy_differs":
      return "The library's copy differs from this file";
    case "copy_unreadable":
      return "The library's copy cannot be read";
    case "cannot_read":
      return "This file cannot be read";
    case "sidecar_not_kept":
      return "Its sidecar was not kept";
    case "failed_verify":
      return "Its copy did not verify when it was imported";
    case "unreadable":
      return "No reader could open it";
    case "burst_frame":
      return "A burst frame that was left out";
    case "not_downloaded":
      return "Not downloaded to the device";
    case "unsupported":
      return "A type Lumen does not import";
    case "duplicate":
      return "A duplicate";
    default:
      return "";
  }
}
