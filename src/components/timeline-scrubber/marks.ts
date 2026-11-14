import type { GroupBy } from "@/types/generated/GroupBy";
import type { Section } from "@/types/generated/Section";
import { formatMonthOf } from "@/utils/date";

/**
 * A place on the scrubber's track (Round 4H). `at` is a share of the content's height (0..1)
 * and `span` how much of it belongs to the mark, up to the next mark of its kind. A bigger
 * year keeps its label when two would touch.
 */
export interface Mark {
  kind: "year" | "month";
  label: string;
  at: number;
  span: number;
}

/**
 * The marks for a grid's sections: one per year, and one per month in the Days and
 * Months views. An ungrouped grid (an album in manual order) has none: its sections carry
 * no dates worth naming.
 */
export function buildMarks(sections: Section[], total: number, grouping: GroupBy | null): Mark[] {
  if (grouping === null || total <= 0) return [];
  const years: Mark[] = [];
  const months: Mark[] = [];
  for (const s of sections) {
    const at = s.y / total;
    if (years.length === 0 || years[years.length - 1].label !== String(s.year)) {
      years.push({ kind: "year", label: String(s.year), at, span: 0 });
    }
    const month = `${s.year}-${s.month}`;
    // Days: a month holds many sections, and gets one mark at its first.
    if (grouping !== "year" && months[months.length - 1]?.label !== month) months.push({ kind: "month", label: month, at, span: 0 });
  }
  const withSpans = (marks: Mark[]) => marks.map((m, i) => ({ ...m, span: (marks[i + 1]?.at ?? 1) - m.at }));
  return [...withSpans(years), ...withSpans(months)];
}

/**
 * The year labels that fit on a track `trackPx` tall: never two closer than `minGapPx`,
 * and where two clash the year that covers more of the library wins.
 */
export function visibleLabels(marks: Mark[], trackPx: number, minGapPx = 18): Mark[] {
  const kept: Mark[] = [];
  const byImportance = marks.filter((m) => m.kind === "year").sort((a, b) => b.span - a.span);
  for (const m of byImportance) {
    if (kept.every((k) => Math.abs(k.at - m.at) * trackPx >= minGapPx)) kept.push(m);
  }
  return kept.sort((a, b) => a.at - b.at);
}

/** The section that holds the offset: the last one starting at or above it. Sections are in
 *  order of `y`, and a Days view has thousands, so this is a binary search. */
function sectionAt(sections: Section[], offset: number): Section | undefined {
  let lo = 0;
  let hi = sections.length - 1;
  let found = 0;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (sections[mid].y <= offset) {
      found = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return sections[found];
}

/** The date the bubble shows for a scroll offset: the section's title without its place. */
export function dateAt(sections: Section[], offset: number, grouping: GroupBy | null): string {
  if (grouping === null) return "";
  const s = sectionAt(sections, offset);
  if (!s) return "";
  if (grouping === "year") return String(s.year);
  if (grouping === "month") return s.title.split(" · ")[0];
  return formatMonthOf(s.year, s.month);
}

/** The thumb's position on the track for a scroll position. */
export function thumbFor(scrollTop: number, viewport: number, total: number, track: number): number {
  const range = total - viewport;
  return range <= 0 ? 0 : Math.min(1, Math.max(0, scrollTop / range)) * track;
}

/** The scroll offset for a pointer `y` pixels down the track. */
export function offsetForPointer(y: number, track: number, total: number, viewport: number): number {
  const range = Math.max(0, total - viewport);
  return track <= 0 ? 0 : Math.min(1, Math.max(0, y / track)) * range;
}
