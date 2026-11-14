import { MEMORY_KIND } from "@/hooks/use-memories";
import type { MemorySummary } from "@/types/generated/MemorySummary";

/** How many of the newest memories the daily rotation chooses among. */
const ROTATION = 5;

/** Whole local days since 1970-01-01: the same number all day, a new one at local midnight. */
export function daysSinceEpoch(today: Date): number {
  return Math.floor(Date.UTC(today.getFullYear(), today.getMonth(), today.getDate()) / 86_400_000);
}

/** Memories carry wall-clock seconds, so their calendar day is read as UTC. */
function isToday(memory: MemorySummary, today: Date): boolean {
  const d = new Date(memory.startAt * 1000);
  return d.getUTCMonth() === today.getMonth() && d.getUTCDate() === today.getDate();
}

/**
 * Home's hero (Round 20 C): today's *On this day* memory if there is one; otherwise one of the
 * five newest visible memories, by the day number, so it changes daily and holds all day.
 */
export function pickHero(memories: MemorySummary[], today: Date): MemorySummary | null {
  const visible = memories.filter((m) => !m.hidden);
  const onThisDay = visible.find((m) => m.kind === MEMORY_KIND.onThisDay && isToday(m, today));
  if (onThisDay) return onThisDay;
  const pool = visible.slice(0, ROTATION);
  return pool.length ? pool[daysSinceEpoch(today) % pool.length] : null;
}
