import { describe, expect, it } from "vitest";
import type { MemorySummary } from "@/types/generated/MemorySummary";
import { daysSinceEpoch, pickHero } from "./pick-hero";

const DAY = 86400;
// Wall-clock seconds, as `captured_at` is stored.
const at = (y: number, m: number, d: number) => Date.UTC(y, m - 1, d) / 1000;

function memory(id: number, over: Partial<MemorySummary> = {}): MemorySummary {
  return { id, kind: 0, title: `Memory ${id}`, subtitle: null, startAt: at(2024, 6, 1) - id * DAY, endAt: at(2024, 6, 2), coverAssetId: id, itemCount: 20, curatedCount: 12, hidden: false, ...over };
}

describe("pickHero", () => {
  const today = new Date(2026, 10, 12); // 12 November 2026, local

  it("is none without memories", () => {
    expect(pickHero([], today)).toBeNull();
    expect(pickHero([memory(1, { hidden: true })], today)).toBeNull();
  });

  it("takes today's On this day memory first", () => {
    const memories = [memory(1), memory(2, { kind: 2, startAt: at(2023, 11, 12), endAt: at(2023, 11, 12) }), memory(3)];
    expect(pickHero(memories, today)?.id).toBe(2);
  });

  it("does not take an On this day memory made for another day", () => {
    const memories = [memory(1, { kind: 2, startAt: at(2023, 11, 3), endAt: at(2023, 11, 3) }), memory(2)];
    expect(pickHero(memories, today)?.id).not.toBe(1);
  });

  it("rotates among the five newest visible memories, one per day", () => {
    const memories = [1, 2, 3, 4, 5, 6, 7].map((i) => memory(i));
    const n = daysSinceEpoch(today);
    expect(pickHero(memories, today)?.id).toBe(memories[n % 5].id);
    const tomorrow = new Date(2026, 10, 13);
    expect(pickHero(memories, tomorrow)?.id).toBe(memories[(n + 1) % 5].id);
    // Stable within a day, whatever the hour.
    expect(pickHero(memories, new Date(2026, 10, 12, 23, 59))?.id).toBe(pickHero(memories, new Date(2026, 10, 12, 0, 1))?.id);
  });

  it("skips hidden memories and copes with fewer than five", () => {
    const memories = [memory(1, { hidden: true }), memory(2), memory(3)];
    expect([2, 3]).toContain(pickHero(memories, today)?.id);
  });
});
