import { afterEach, describe, expect, it } from "vitest";
import { formatCaptured, formatCapturedDate, formatDuration, formatLongDate, formatMonth, formatMonthOf, formatRelative, formatShortDate, getDateOrder, setDateOrder } from "./date";

const june4 = Date.UTC(2026, 5, 4, 23, 30) / 1000;
const monthName = (m: number, style: "long" | "short") => new Intl.DateTimeFormat(undefined, { month: style, timeZone: "UTC" }).format(new Date(Date.UTC(2026, m - 1, 15)));
const june = monthName(6, "long");
const jun = monthName(6, "short");

describe("date", () => {
  afterEach(() => setDateOrder("dmy"));

  it("writes a short date in the date order", () => {
    setDateOrder("dmy");
    expect(formatShortDate(june4)).toBe("4.6.2026");
    setDateOrder("mdy");
    expect(formatShortDate(june4)).toBe("6/4/2026");
    setDateOrder("ymd");
    expect(formatShortDate(june4)).toBe("2026-06-04");
    expect(getDateOrder()).toBe("ymd");
  });

  it("keeps the locale's month names in long forms, ordered by the setting", () => {
    setDateOrder("dmy");
    expect(formatLongDate(june4)).toBe(`4 ${june} 2026`);
    expect(formatCapturedDate(june4)).toBe(`4 ${jun} 2026`);
    setDateOrder("mdy");
    expect(formatLongDate(june4)).toBe(`${june} 4, 2026`);
    expect(formatCapturedDate(june4)).toBe(`${jun} 4, 2026`);
    setDateOrder("ymd");
    expect(formatLongDate(june4)).toBe(`2026 ${june} 4`);
  });

  it("names the weekday when asked", () => {
    setDateOrder("dmy");
    const weekday = new Intl.DateTimeFormat(undefined, { weekday: "short", timeZone: "UTC" }).format(new Date(june4 * 1000));
    expect(formatLongDate(june4, { weekday: true })).toBe(`${weekday}, 4 ${june} 2026`);
  });

  it("formats months, the year after it except year first", () => {
    const ts = Date.UTC(2026, 8, 17, 12) / 1000;
    const september = monthName(9, "long");
    expect(formatMonth(ts)).toBe(`${september} 2026`);
    expect(formatMonthOf(2026, 9)).toBe(`${september} 2026`);
    setDateOrder("ymd");
    expect(formatMonthOf(2026, 9)).toBe(`2026 ${september}`);
  });

  it("shows a capture time as the camera's clock read it, whatever this computer's zone", () => {
    // captured_at is wall-clock time stored as if it were UTC: 23:30 on 4 June.
    const time = new Intl.DateTimeFormat(undefined, { timeStyle: "short", timeZone: "UTC" }).format(new Date(june4 * 1000));
    expect(formatCaptured(june4)).toBe(`4 ${jun} 2026, ${time}`);
    setDateOrder("mdy");
    expect(formatCaptured(june4)).toBe(`${jun} 4, 2026, ${time}`);
  });

  it("formats relative times", () => {
    const now = 1_758_000_000_000;
    const at = (secondsAgo: number) => formatRelative(now / 1000 - secondsAgo, now);
    expect(at(10)).toBe("just now");
    expect(at(300)).toBe("5 min ago");
    expect(at(7200)).toBe("2 h ago");
    expect(at(86_400)).toBe("yesterday");
    expect(at(3 * 86_400)).toBe("3 days ago");
    expect(at(60 * 86_400)).not.toMatch(/ago/);
  });

  it("formats durations", () => {
    expect(formatDuration(65_000)).toBe("1:05");
    expect(formatDuration(3_725_000)).toBe("1:02:05");
  });
});
