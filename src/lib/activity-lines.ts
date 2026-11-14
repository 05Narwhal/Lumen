import type { ActivityLine } from "@/types/generated/ActivityLine";
import type { JobsUpdate } from "@/types/generated/JobsUpdate";
import type { Countdown } from "./people-progress";
import type { Job } from "./jobs-store";

/**
 * The activity ring's lines from Rust's reports (Round 9B). The ring names work a person
 * would recognise and how far it got. Lanes, queues and workers stay inside the app:
 * thumbnails and previews rendered while scrolling are not background work, so they never
 * show at all.
 */

/** A named activity (`photos_jobs::activity`): *Preparing videos · 41 of 322*. */
export function activityJob(line: ActivityLine): Job {
  const total = Math.max(line.total, line.done);
  return {
    id: `activity:${line.id}`,
    kind: "maintenance",
    label: line.label,
    progress: total > 0 ? line.done / total : 0,
    done: line.done,
    total,
    state: "running",
    detail: line.detail ?? (total > 0 ? `${line.done.toLocaleString("en-US")} of ${total.toLocaleString("en-US")}` : undefined),
  };
}

export const BACKGROUND_ID = "background";

/**
 * Maintenance work that no named line covers (metadata, tidying after an import). Shown only
 * while nothing named is running, so the ring does not list the same work twice. Its progress
 * counts the queue down without ever starting over (`countdown`). `null` when there is none.
 */
export function backgroundJob(u: JobsUpdate, counter: Countdown, namedRunning: boolean): Job | null {
  const left = u.maintenance;
  if (left <= 0) {
    counter.reset();
    return null;
  }
  const { done, total } = counter.update(left);
  if (namedRunning) return null;
  return {
    id: BACKGROUND_ID,
    kind: "maintenance",
    label: "Tidying up the library",
    progress: total > 0 ? done / total : 0,
    done,
    total,
    state: "running",
  };
}
