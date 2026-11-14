import type { DateOrder } from "@/types/generated/DateOrder";

/**
 * Every date Lumen shows is written here (Round 11): numbers in the date order (Settings →
 * Search & dates, else the system's), month names and times from the locale.
 */
let order: DateOrder = "dmy";

/** Called at startup with `get_date_order` and again when the setting changes. */
export function setDateOrder(o: DateOrder): void {
  order = o;
}

export function getDateOrder(): DateOrder {
  return order;
}

/** `utc` reads the capture clock (wall-clock time stored as if it were UTC); `local` a true instant. */
type Zone = "utc" | "local";

const monthNames = new Map<string, Intl.DateTimeFormat>();
function monthName(d: Date, style: "long" | "short", zone: Zone): string {
  const key = `${style}${zone}`;
  let f = monthNames.get(key);
  if (!f) {
    f = new Intl.DateTimeFormat(undefined, { month: style, ...(zone === "utc" ? { timeZone: "UTC" } : {}) });
    monthNames.set(key, f);
  }
  return f.format(d);
}

const weekdayUtc = new Intl.DateTimeFormat(undefined, { weekday: "short", timeZone: "UTC" });
const weekdayLocal = new Intl.DateTimeFormat(undefined, { weekday: "short" });
const timeUtc = new Intl.DateTimeFormat(undefined, { timeStyle: "short", timeZone: "UTC" });
const timeLocal = new Intl.DateTimeFormat(undefined, { timeStyle: "short" });

function parts(d: Date, zone: Zone) {
  return zone === "utc"
    ? { y: d.getUTCFullYear(), m: d.getUTCMonth() + 1, day: d.getUTCDate() }
    : { y: d.getFullYear(), m: d.getMonth() + 1, day: d.getDate() };
}

/** Day, month name and year in the date order: "4 June 2026", "June 4, 2026", "2026 June 4". */
function written(d: Date, style: "long" | "short", zone: Zone): string {
  const { y, day } = parts(d, zone);
  const month = monthName(d, style, zone);
  if (order === "mdy") return `${month} ${day}, ${y}`;
  if (order === "ymd") return `${y} ${month} ${day}`;
  return `${day} ${month} ${y}`;
}

/** Unix seconds → "4.6.2026" / "6/4/2026" / "2026-06-04". */
export function formatShortDate(ts: number, zone: Zone = "utc"): string {
  const { y, m, day } = parts(new Date(ts * 1000), zone);
  const pad = (n: number) => String(n).padStart(2, "0");
  if (order === "mdy") return `${m}/${day}/${y}`;
  if (order === "ymd") return `${y}-${pad(m)}-${pad(day)}`;
  return `${day}.${m}.${y}`;
}

/** Unix seconds → "4 June 2026" (with `weekday`, "Thu, 4 June 2026"). */
export function formatLongDate(ts: number, opts: { weekday?: boolean; zone?: Zone } = {}): string {
  const zone = opts.zone ?? "utc";
  const d = new Date(ts * 1000);
  const date = written(d, "long", zone);
  return opts.weekday ? `${(zone === "utc" ? weekdayUtc : weekdayLocal).format(d)}, ${date}` : date;
}

/** A month of a year → "September 2026" ("2026 September" year first). */
export function formatMonthOf(year: number, month: number): string {
  const name = monthName(new Date(Date.UTC(year, month - 1, 15)), "long", "utc");
  return order === "ymd" ? `${year} ${name}` : `${name} ${year}`;
}

/** Unix seconds → "September 2026", in this computer's zone. */
export function formatMonth(ts: number): string {
  const d = new Date(ts * 1000);
  return formatMonthOf(d.getFullYear(), d.getMonth() + 1);
}

/**
 * A photo's `capturedAt` → "15 Sept 2025, 23:30". It is the camera's wall clock stored as if it
 * were UTC, so it is read in UTC: converting it to this computer's zone would move it.
 */
export function formatCaptured(ts: number): string {
  const d = new Date(ts * 1000);
  return `${written(d, "short", "utc")}, ${timeUtc.format(d)}`;
}

/** A date on the capture clock (a query's `dateFrom`/`dateTo`, a memory's span) → "15 Sept 2025". */
export function formatCapturedDate(ts: number): string {
  return written(new Date(ts * 1000), "short", "utc");
}

/** Unix seconds (true UTC, e.g. `importedAt`) → "17 Sept 2026, 18:30" in local time. */
export function formatDateTime(ts: number): string {
  const d = new Date(ts * 1000);
  return `${written(d, "short", "local")}, ${timeLocal.format(d)}`;
}

/** Unix seconds (true UTC) → "17 Sept 2026" in local time. */
export function formatDate(ts: number): string {
  return written(new Date(ts * 1000), "short", "local");
}

/** Unix seconds → "just now", "5 min ago", "3 h ago", "yesterday", "4 days ago", else a date. */
export function formatRelative(ts: number, now = Date.now()): string {
  const diff = Math.max(0, Math.floor(now / 1000) - ts);
  if (diff < 45) return "just now";
  if (diff < 3600) return `${Math.round(diff / 60)} min ago`;
  if (diff < 86_400) return `${Math.round(diff / 3600)} h ago`;
  const days = Math.round(diff / 86_400);
  if (days === 1) return "yesterday";
  if (days < 14) return `${days} days ago`;
  return formatDate(ts);
}

export function formatDuration(ms: number): string {
  const s = Math.round(ms / 1000);
  const m = Math.floor(s / 60);
  const h = Math.floor(m / 60);
  const pad = (n: number) => String(n).padStart(2, "0");
  return h ? `${h}:${pad(m % 60)}:${pad(s % 60)}` : `${m}:${pad(s % 60)}`;
}
