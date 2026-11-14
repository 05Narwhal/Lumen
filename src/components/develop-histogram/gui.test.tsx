import { act, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DevelopHistogram as Histogram } from "@/types/generated/DevelopHistogram";
import { areaPath, DevelopHistogram } from "./gui";

const flat = (v: number): Histogram => {
  const c = Array.from({ length: 256 }, () => v);
  return { r: c, g: c, b: c, luma: c, clippedShadows: 0, clippedHighlights: 0 };
};
const ramp = (): Histogram => {
  const c = Array.from({ length: 256 }, (_, i) => i);
  return { r: c, g: c, b: c, luma: c, clippedShadows: 0, clippedHighlights: 0 };
};
const red = (c: HTMLElement) => c.querySelector(".develop-histogram__r")?.getAttribute("d");

describe("DevelopHistogram", () => {
  beforeEach(() => vi.useFakeTimers({ toFake: ["requestAnimationFrame", "cancelAnimationFrame", "performance", "setTimeout"] }));
  afterEach(() => vi.useRealTimers());

  it("morphs from one photo's counts to the next instead of snapping", () => {
    const { container, rerender } = render(<DevelopHistogram histogram={flat(100)} clipping={false} onClipping={vi.fn()} />);
    const before = red(container);
    rerender(<DevelopHistogram histogram={ramp()} clipping={false} onClipping={vi.fn()} />);
    act(() => vi.advanceTimersByTime(120));
    const mid = red(container);
    expect(mid).not.toBe(before);
    act(() => vi.advanceTimersByTime(600));
    const end = ramp();
    expect(red(container)).toBe(areaPath(end.r, Math.max(...end.r, ...end.g, ...end.b, 1)));
    expect(mid).not.toBe(red(container));
  });
});
