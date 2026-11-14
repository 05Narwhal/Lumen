import { describe, expect, it } from "vitest";
import { cutRange, keepRange, keptMs, playableAt, segmentsOf, stepAt, trimFor } from "./trim";

const seg = (startMs: number, endMs: number) => ({ start_ms: startMs, end_ms: endMs });

describe("segmentsOf", () => {
  it("is the whole clip when there is no trim", () => {
    expect(segmentsOf(null, 10_000)).toEqual([seg(0, 10_000)]);
    expect(segmentsOf({ segments: [] }, 10_000)).toEqual([seg(0, 10_000)]);
    expect(segmentsOf({ segments: [seg(1000, 2000)] }, 10_000)).toEqual([seg(1000, 2000)]);
  });
});

describe("cutRange", () => {
  const whole = [seg(0, 10_000)];

  it("cuts inside a segment, leaving two", () => {
    expect(cutRange(whole, 3000, 5000)).toEqual([seg(0, 3000), seg(5000, 10_000)]);
  });

  it("cuts across a boundary", () => {
    expect(cutRange([seg(0, 3000), seg(5000, 10_000)], 2000, 6000)).toEqual([seg(0, 2000), seg(6000, 10_000)]);
  });

  it("takes a head or a tail off", () => {
    expect(cutRange(whole, 0, 1500)).toEqual([seg(1500, 10_000)]);
    expect(cutRange(whole, 8000, 10_000)).toEqual([seg(0, 8000)]);
  });

  it("accepts the ends in either order", () => {
    expect(cutRange(whole, 5000, 3000)).toEqual([seg(0, 3000), seg(5000, 10_000)]);
  });

  it("refuses to cut everything", () => {
    expect(cutRange(whole, 0, 10_000)).toBeNull();
    expect(cutRange([seg(2000, 4000)], 1000, 5000)).toBeNull();
  });

  it("drops slivers shorter than a frame or so", () => {
    expect(cutRange(whole, 20, 5000)).toEqual([seg(5000, 10_000)]);
  });
});

describe("keepRange", () => {
  it("keeps only the selection, across earlier cuts", () => {
    expect(keepRange([seg(0, 3000), seg(5000, 10_000)], 2000, 8000)).toEqual([seg(2000, 3000), seg(5000, 8000)]);
  });

  it("refuses an empty selection", () => {
    expect(keepRange([seg(0, 3000)], 4000, 6000)).toBeNull();
  });
});

describe("keptMs and playableAt", () => {
  const segs = [seg(1000, 3000), seg(5000, 8000)];

  it("adds up what plays", () => {
    expect(keptMs(segs)).toBe(5000);
  });

  it("stays inside a kept segment and jumps over a cut", () => {
    expect(playableAt(segs, 2000)).toBe(2000);
    expect(playableAt(segs, 0)).toBe(1000);
    expect(playableAt(segs, 3000)).toBe(5000);
    expect(playableAt(segs, 4200)).toBe(5000);
    expect(playableAt(segs, 8000)).toBeNull();
    expect(playableAt(segs, 9000)).toBeNull();
  });
});

describe("stepAt (the player skipping cuts)", () => {
  const segs = [seg(1000, 3000), seg(5000, 8000)];

  it("plays on inside a segment and jumps a cut", () => {
    expect(stepAt(segs, 2500, false)).toEqual({ seek: null, pause: false });
    expect(stepAt(segs, 3010, false)).toEqual({ seek: 5000, pause: false });
  });

  it("stops at the end, or loops to the first kept moment", () => {
    expect(stepAt(segs, 8005, false)).toEqual({ seek: 8000, pause: true });
    expect(stepAt(segs, 8005, true)).toEqual({ seek: 1000, pause: false });
  });
});

describe("trimFor", () => {
  it("is no trim when the whole clip is kept", () => {
    expect(trimFor([seg(0, 10_000)], 10_000)).toBeNull();
    expect(trimFor([seg(0, 9000)], 10_000)).toEqual({ segments: [seg(0, 9000)] });
  });
});
