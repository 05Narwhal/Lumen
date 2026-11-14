import type { Segment } from "@/types/generated/Segment";
import type { Trim } from "@/types/generated/Trim";

/** Shorter than this is a slip of the handle, not something anyone meant to keep. */
export const MIN_SEGMENT_MS = 50;

const clean = (segs: Segment[]) => segs.filter((s) => s.end_ms - s.start_ms >= MIN_SEGMENT_MS);

/** The kept segments, or the whole clip when there is no trim. */
export function segmentsOf(trim: Trim | null | undefined, durationMs: number): Segment[] {
  return trim && trim.segments.length > 0 ? trim.segments : [{ start_ms: 0, end_ms: Math.max(0, Math.round(durationMs)) }];
}

/** Remove `[a, b)` from what is kept. Null when nothing would be left. */
export function cutRange(segs: Segment[], a: number, b: number): Segment[] | null {
  const [lo, hi] = a <= b ? [a, b] : [b, a];
  const out: Segment[] = [];
  for (const s of segs) {
    if (hi <= s.start_ms || lo >= s.end_ms) {
      out.push(s);
      continue;
    }
    if (lo > s.start_ms) out.push({ start_ms: s.start_ms, end_ms: lo });
    if (hi < s.end_ms) out.push({ start_ms: hi, end_ms: s.end_ms });
  }
  const kept = clean(out);
  return kept.length > 0 ? kept : null;
}

/** Keep only `[a, b)` of what is kept. Null when that is nothing. */
export function keepRange(segs: Segment[], a: number, b: number): Segment[] | null {
  const [lo, hi] = a <= b ? [a, b] : [b, a];
  const kept = clean(segs.map((s) => ({ start_ms: Math.max(s.start_ms, lo), end_ms: Math.min(s.end_ms, hi) })));
  return kept.length > 0 ? kept : null;
}

export const keptMs = (segs: Segment[]) => segs.reduce((n, s) => n + (s.end_ms - s.start_ms), 0);

/**
 * Where playback belongs at `t`: `t` itself inside a kept segment, the next segment's start
 * inside a cut, or null past the last one (the end of the edited clip).
 */
export function playableAt(segs: Segment[], t: number): number | null {
  for (const s of segs) {
    if (t < s.start_ms) return s.start_ms;
    if (t < s.end_ms) return t;
  }
  return null;
}

/** A trim that keeps the whole clip is no trim. */
export function trimFor(segs: Segment[], durationMs: number): Trim | null {
  const whole = segs.length === 1 && segs[0].start_ms <= 0 && segs[0].end_ms >= Math.round(durationMs) - MIN_SEGMENT_MS;
  return whole ? null : { segments: segs };
}

/**
 * What the player does at `t` while playing a trimmed clip: nothing, seek to the next kept
 * moment, or — past the last segment — loop to the first one or stop at the end.
 */
export function stepAt(segs: Segment[], t: number, loop: boolean): { seek: number | null; pause: boolean } {
  const at = playableAt(segs, t);
  if (at === t) return { seek: null, pause: false };
  if (at !== null) return { seek: at, pause: false };
  return loop ? { seek: segs[0].start_ms, pause: false } : { seek: segs[segs.length - 1].end_ms, pause: true };
}
