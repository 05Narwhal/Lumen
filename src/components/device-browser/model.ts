// Pure parts of the device browser: date ranges, rows for the virtual grid, which device
// events mean the device has gone. No DOM, so they are tested without a browser.

import type { BrowseTarget } from "@/types/generated/BrowseTarget";
import type { BrowserFilter } from "@/types/generated/BrowserFilter";
import type { BrowserItem } from "@/types/generated/BrowserItem";
import type { IphonesUpdate } from "@/types/generated/IphonesUpdate";
import type { MacDiskInfo } from "@/types/generated/MacDiskInfo";
import type { VolumeInfo } from "@/types/generated/VolumeInfo";
import { formatLongDate } from "@/utils/date";

const DAY = 86_400;

/**
 * Item dates are wall times written as unix seconds (like `captured_at`): 14:00 on the
 * camera is 14:00 here, whatever the zone. They are read and built in UTC.
 */
export const wallDate = (s: number) => new Date(s * 1000);

/** `2026-03-04` for a date input, from wall seconds. */
export const toInputDate = (s: number | null) => (s === null ? "" : wallDate(s).toISOString().slice(0, 10));

/** Wall seconds at the start (or, with `end`, the last second) of a `yyyy-mm-dd` day. */
export function fromInputDate(v: string, end = false): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(v);
  if (!m) return null;
  const start = Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])) / 1000;
  return end ? start + DAY - 1 : start;
}

export type QuickPick = "all" | "new" | "today" | "week" | "month" | "year";

export const QUICK_PICKS: { id: QuickPick; label: string }[] = [
  { id: "new", label: "New since last import" },
  { id: "today", label: "Today" },
  { id: "week", label: "This week" },
  { id: "month", label: "This month" },
  { id: "year", label: "This year" },
  { id: "all", label: "All" },
];

/**
 * The filter a quick pick sets, from `now` (a local `Date`): the range is the local
 * calendar's today, week (from Monday), month or year, in wall seconds. *New since last
 * import* is what the source has not given the library before, at any date.
 */
export function applyQuickPick(f: BrowserFilter, pick: QuickPick, now: Date): BrowserFilter {
  const wall = (y: number, m: number, d: number) => Date.UTC(y, m, d) / 1000;
  const [y, m, d] = [now.getFullYear(), now.getMonth(), now.getDate()];
  const today = wall(y, m, d);
  const end = today + DAY - 1;
  switch (pick) {
    case "all":
      return { ...f, from: null, to: null, newOnly: false };
    case "new":
      return { ...f, from: null, to: null, newOnly: true };
    case "today":
      return { ...f, from: today, to: end, newOnly: false };
    case "week":
      return { ...f, from: today - ((now.getDay() + 6) % 7) * DAY, to: end, newOnly: false };
    case "month":
      return { ...f, from: wall(y, m, 1), to: end, newOnly: false };
    case "year":
      return { ...f, from: wall(y, 0, 1), to: end, newOnly: false };
  }
}

export const DEFAULT_FILTER: BrowserFilter = { from: null, to: null, kinds: [], newOnly: false, includeUndated: true };

/** One stable string per filter, for query keys. */
export const filterKey = (f: BrowserFilter) => JSON.stringify([f.from, f.to, [...f.kinds].sort(), f.newOnly, f.includeUndated]);

export type Row = { kind: "header"; key: string; label: string; ids: number[] } | { kind: "tiles"; key: string; items: BrowserItem[] };

const dayKey = (item: BrowserItem) => (item.date === null ? "none" : wallDate(item.date).toISOString().slice(0, 10));

const dayLabel = (key: string) =>
  key === "none"
    ? "No date"
    : formatLongDate(Date.parse(`${key}T00:00:00Z`) / 1000, { weekday: true });

/** Day headers, each followed by rows of `columns` tiles; items arrive newest first. */
export function buildRows(items: BrowserItem[], columns: number): Row[] {
  const rows: Row[] = [];
  const cols = Math.max(1, columns);
  let day: string | null = null;
  let header: Extract<Row, { kind: "header" }> | null = null;
  let run: BrowserItem[] = [];
  const flush = () => {
    for (let i = 0; i < run.length; i += cols) rows.push({ kind: "tiles", key: `${day}:${i}`, items: run.slice(i, i + cols) });
    run = [];
  };
  for (const item of items) {
    const k = dayKey(item);
    if (k !== day) {
      flush();
      day = k;
      header = { kind: "header", key: `h:${k}`, label: dayLabel(k), ids: [] };
      rows.push(header);
    }
    header!.ids.push(item.index);
    run.push(item);
  }
  flush();
  return rows;
}

/** How many tiles of `tile` px fit in `width` with `gap` between them. */
export const columnsFor = (width: number, tile: number, gap: number) => Math.max(1, Math.floor((width + gap) / (tile + gap)));

/** Shift-click: every index from the anchor to `index` along `order` joins the selection. */
export function extendSelection(selected: ReadonlySet<number>, order: readonly number[], anchor: number | null, index: number): Set<number> {
  const next = new Set(selected);
  const a = anchor === null ? -1 : order.indexOf(anchor);
  const b = order.indexOf(index);
  if (a === -1 || b === -1) {
    next.add(index);
    return next;
  }
  const [from, to] = a < b ? [a, b] : [b, a];
  for (let i = from; i <= to; i++) next.add(order[i]);
  return next;
}

/** *Invert*: within what the filter shows; selected items it hides stay selected. */
export function invertWithin(selected: ReadonlySet<number>, shown: readonly number[]): Set<number> {
  const next = new Set(selected);
  for (const i of shown) {
    if (next.has(i)) next.delete(i);
    else next.add(i);
  }
  return next;
}

/** Whether the device behind `target` is still in a watcher's latest list. */
export function stillPresent(target: BrowseTarget, event: "volumes" | "iphones" | "macdisks", payload: unknown): boolean | null {
  switch (target.kind) {
    case "volume":
      return event === "volumes" ? (payload as VolumeInfo[]).some((v) => v.mount === target.id) : null;
    case "iphone":
      return event === "iphones" ? (payload as IphonesUpdate).devices.some((d) => d.udid === target.id) : null;
    case "macFolder":
      return event === "macdisks" ? (payload as MacDiskInfo[]).some((d) => d.id === target.disk) : null;
    case "photosLibrary":
      return event === "macdisks" && target.disk !== null ? (payload as MacDiskInfo[]).some((d) => d.id === target.disk) : null;
  }
}
