import { afterEach, describe, expect, it, vi } from "vitest";
import type { JobsUpdate } from "@/types/generated/JobsUpdate";
import { activityJob, backgroundJob } from "./activity-lines";
import { countdown, finishJob, FINISH_HOLD_MS, jobsStore, removeJob, ringProgress, selectRunning, upsertJob, type Job } from "./jobs-store";

const job = (id: string, done: number, total: number): Job => ({ id, kind: "maintenance", label: id, progress: done / total, done, total, state: "running" });

describe("the activity ring's numbers (Round 9B)", () => {
  afterEach(() => {
    vi.useRealTimers();
    for (const j of jobsStore.get().jobs) removeJob(j.id);
  });

  it("weighs every line by its size instead of averaging fractions", () => {
    // A 4-photo import just started beside a scan 900 of 1,000 in: the ring stays near 90 %,
    // where an average would have halved it.
    expect(ringProgress([job("scan", 900, 1000), job("import", 0, 4)])).toBeCloseTo(900 / 1004);
    expect(ringProgress([])).toBe(0);
    const uncounted: Job = { id: "film", kind: "export", label: "Exporting memory", progress: 0.5, state: "running" };
    expect(ringProgress([uncounted])).toBeCloseTo(0.5);
  });

  it("counts a countdown up, so more work never sends it back to 0", () => {
    const c = countdown();
    c.update(50);
    expect(c.update(10)).toEqual({ done: 40, total: 50 });
    expect(c.update(60)).toEqual({ done: 40, total: 100 });
    c.reset();
    expect(c.update(5)).toEqual({ done: 0, total: 5 });
  });

  it("holds a finished line full before it goes, unless it starts again", () => {
    vi.useFakeTimers();
    upsertJob(job("people", 3, 10));
    finishJob("people");
    expect(selectRunning(jobsStore.get())).toMatchObject([{ id: "people", state: "finishing", progress: 1 }]);
    vi.advanceTimersByTime(FINISH_HOLD_MS + 1);
    expect(selectRunning(jobsStore.get())).toEqual([]);

    upsertJob(job("pets", 1, 2));
    finishJob("pets");
    upsertJob(job("pets", 0, 5));
    vi.advanceTimersByTime(FINISH_HOLD_MS + 1);
    expect(selectRunning(jobsStore.get())).toMatchObject([{ id: "pets", state: "running" }]);
  });

  it("never names lanes, queues or workers", () => {
    const u: JobsUpdate = { running: 6, queued: 40, interactive: 30, prefetch: 5, import: 0, maintenance: 5, workers: 24 };
    const c = countdown();
    const line = backgroundJob(u, c, false);
    expect(line?.label).toBe("Tidying up the library");
    expect(JSON.stringify(line)).not.toMatch(/worker|lane|queue|thumbnail/i);
    expect(backgroundJob({ ...u, maintenance: 0 }, c, false)).toBeNull();
    expect(backgroundJob(u, countdown(), true), "a named line covers it").toBeNull();
    // Scrolling's thumbnails alone are not background work.
    expect(backgroundJob({ ...u, maintenance: 0, interactive: 300 }, countdown(), false)).toBeNull();
  });

  it("reads a named activity from Rust as done of total", () => {
    expect(activityJob({ id: "video-prep", label: "Preparing videos", done: 41, total: 322, detail: null })).toMatchObject({
      id: "activity:video-prep",
      label: "Preparing videos",
      detail: "41 of 322",
      done: 41,
      total: 322,
    });
  });
});
