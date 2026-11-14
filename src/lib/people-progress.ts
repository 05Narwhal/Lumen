import { countdown, type Job } from "./jobs-store";

export type Countdown = ReturnType<typeof countdown>;

/**
 * The face scan's line in the activity ring (*Looking for people · 1,204 left*). Rust says
 * only how many photos are left after each batch; `counter` turns that into done of total, so
 * newly imported photos add to the total instead of sending the line back to 0 (Round 9B).
 * `null` once nothing is left.
 */
export function peopleProgressJob(left: number, counter: Countdown): Job | null {
  if (left <= 0) {
    counter.reset();
    return null;
  }
  const { done, total } = counter.update(left);
  return {
    id: "people",
    kind: "maintenance",
    label: "Looking for people",
    progress: total > 0 ? done / total : 0,
    done,
    total,
    state: "running",
    detail: `${left.toLocaleString("en-US")} left`,
  };
}

/** The pet scan's line (Round 4C), measured the same way. */
export function petsProgressJob(left: number, counter: Countdown): Job | null {
  const job = peopleProgressJob(left, counter);
  return job && { ...job, id: "pets", label: "Looking for pets" };
}

/** The text scan's line (Round 13, *Reading text · 1,204 left*), measured the same way. */
export function ocrProgressJob(left: number, counter: Countdown): Job | null {
  const job = peopleProgressJob(left, counter);
  return job && { ...job, id: "ocr", label: "Reading text" };
}
