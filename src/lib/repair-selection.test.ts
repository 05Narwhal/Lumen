import { describe, expect, it } from "vitest";
import { isTicked, NOTHING, selectAll, tickedCount, toggle } from "./repair-selection";

describe("repair selection", () => {
  it("ticks rows one by one", () => {
    const s = toggle(toggle(NOTHING, 3), 5);
    expect(isTicked(s, 3)).toBe(true);
    expect(isTicked(toggle(s, 3), 3)).toBe(false);
    expect(tickedCount(s, 100)).toBe(2);
  });

  it("select all covers rows that are not loaded, minus the ones unticked", () => {
    const s = toggle(selectAll("new"), 7);
    expect(isTicked(s, 1)).toBe(true);
    expect(isTicked(s, 7)).toBe(false);
    expect(tickedCount(s, 1200)).toBe(1199);
    expect(isTicked(toggle(s, 7), 7)).toBe(true);
  });
});
