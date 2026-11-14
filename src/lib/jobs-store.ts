import { createStore } from "./store";

export type JobKind = "import" | "thumbnail" | "proxy" | "export" | "maintenance" | "backup";

export interface Job {
  id: string;
  kind: JobKind;
  /** What the app is doing, in words a person understands ("Looking for faces"). */
  label: string;
  /** 0–1 */
  progress: number;
  /**
   * `finishing` is a finished line held at 100 % for a moment before it goes (Round 9B), so
   * the ring completes rather than vanishing mid-arc.
   */
  state: "running" | "finishing" | "done" | "failed";
  /** Free-form detail line (current file, time left). Never lanes or workers. */
  detail?: string;
  /** How much is done and how much there is, in the job's own unit (photos, files, MB). */
  done?: number;
  total?: number;
}

/** Fed by Tauri events; the activity ring is its only consumer. */
export const jobsStore = createStore<{ jobs: Job[] }>({ jobs: [] });

/** Lines the ring shows: running ones and those just finished. */
export const selectRunning = (s: { jobs: Job[] }) => s.jobs.filter((j) => j.state === "running" || j.state === "finishing");

/** How long a finished line stays at 100 % before it goes. */
export const FINISH_HOLD_MS = 600;

const removals = new Map<string, ReturnType<typeof setTimeout>>();

export function upsertJob(job: Job) {
  const pending = removals.get(job.id);
  if (pending) {
    clearTimeout(pending);
    removals.delete(job.id);
  }
  jobsStore.set((s) => {
    const i = s.jobs.findIndex((j) => j.id === job.id);
    if (i === -1) return { jobs: [...s.jobs, job] };
    const next = s.jobs.slice();
    next[i] = job;
    return { jobs: next };
  });
}

export function removeJob(id: string) {
  const pending = removals.get(id);
  if (pending) clearTimeout(pending);
  removals.delete(id);
  jobsStore.set((s) => ({ jobs: s.jobs.filter((j) => j.id !== id) }));
}

/** Ends a line: full for `FINISH_HOLD_MS`, then gone. A line that starts again meanwhile stays. */
export function finishJob(id: string) {
  if (removals.has(id)) return;
  const job = jobsStore.get().jobs.find((j) => j.id === id);
  if (!job) return;
  jobsStore.set((s) => ({ jobs: s.jobs.map((j) => (j.id === id ? { ...j, state: "finishing", progress: 1, done: j.total ?? j.done } : j)) }));
  removals.set(
    id,
    setTimeout(() => {
      removals.delete(id);
      jobsStore.set((s) => ({ jobs: s.jobs.filter((j) => j.id !== id || j.state !== "finishing") }));
    }, FINISH_HOLD_MS),
  );
}

/**
 * The ring's one value: everything done over everything there is, so a big scan weighs more
 * than a three-photo import and a line starting at 0 does not halve the ring. A line without
 * counts weighs as one item at its progress.
 */
export function ringProgress(jobs: Job[]): number {
  let done = 0;
  let total = 0;
  for (const j of jobs) {
    if (j.total && j.total > 0) {
      done += Math.min(j.done ?? j.progress * j.total, j.total);
      total += j.total;
    } else {
      done += j.progress;
      total += 1;
    }
  }
  return total > 0 ? Math.min(1, Math.max(0, done / total)) : 0;
}

/**
 * For work Rust reports only as *N left* (the face and pet scans, a lane's queue): counts
 * what got done between reports, so progress never drops back to 0 when more work arrives.
 * More work raises the total; what was done stays done. `reset` when the work is finished.
 */
export function countdown() {
  let last: number | null = null;
  let done = 0;
  return {
    update(left: number): { done: number; total: number } {
      if (last !== null && left < last) done += last - left;
      last = left;
      return { done, total: done + left };
    },
    reset() {
      last = null;
      done = 0;
    },
  };
}
