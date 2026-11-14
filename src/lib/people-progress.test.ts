import { describe, expect, it } from "vitest";
import { countdown } from "./jobs-store";
import { ocrProgressJob, peopleProgressJob } from "./people-progress";

describe("ocrProgressJob (Round 13)", () => {
  it("says how much text is left to read, and is gone at 0", () => {
    const c = countdown();
    expect(ocrProgressJob(1204, c)).toMatchObject({ id: "ocr", kind: "maintenance", label: "Reading text", detail: "1,204 left" });
    expect(ocrProgressJob(0, c)).toBeNull();
  });
});

describe("peopleProgressJob", () => {
  it("counts what the scan got done", () => {
    const c = countdown();
    expect(peopleProgressJob(1204, c)).toMatchObject({ id: "people", label: "Looking for people", progress: 0, detail: "1,204 left" });
    expect(peopleProgressJob(602, c)?.progress).toBeCloseTo(0.5);
  });

  it("never drops back to 0 when new photos arrive mid-scan (Round 9B)", () => {
    const c = countdown();
    peopleProgressJob(100, c);
    peopleProgressJob(20, c); // 80 of 100
    const more = peopleProgressJob(120, c); // 100 photos imported: 80 of 200
    expect(more).toMatchObject({ done: 80, total: 200 });
    expect(more?.progress).toBeCloseTo(0.4);
    expect(peopleProgressJob(60, c)?.progress).toBeCloseTo(0.7);
  });

  it("is gone when the scan is done, and starts over the next time", () => {
    const c = countdown();
    peopleProgressJob(100, c);
    expect(peopleProgressJob(0, c)).toBeNull();
    expect(peopleProgressJob(10, c)?.progress).toBe(0);
  });
});
