import { describe, expect, it } from "vitest";
import type { BrowserItem } from "@/types/generated/BrowserItem";
import { applyQuickPick, buildRows, columnsFor, DEFAULT_FILTER, extendSelection, fromInputDate, invertWithin, stillPresent, toInputDate } from "./model";

const day = (y: number, m: number, d: number, h = 12) => Date.UTC(y, m - 1, d, h) / 1000;

const item = (index: number, date: number | null): BrowserItem => ({
  index,
  token: String(index),
  fileName: `IMG_${index}.JPG`,
  size: 1,
  files: 1,
  date,
  dateSource: date === null ? null : "exif",
  kind: "photo",
  imported: false,
});

describe("device browser model", () => {
  it("reads and writes dates as wall days", () => {
    expect(fromInputDate("2026-03-04")).toBe(Date.UTC(2026, 2, 4) / 1000);
    expect(fromInputDate("2026-03-04", true)).toBe(Date.UTC(2026, 2, 5) / 1000 - 1);
    expect(fromInputDate("")).toBeNull();
    expect(toInputDate(day(2026, 3, 4, 23))).toBe("2026-03-04");
    expect(toInputDate(null)).toBe("");
  });

  it("turns quick picks into ranges from the local calendar", () => {
    // Wednesday 18 March 2026, in whatever zone the test runs.
    const now = new Date(2026, 2, 18, 15, 30);
    const today = Date.UTC(2026, 2, 18) / 1000;
    const end = today + 86_399;
    expect(applyQuickPick(DEFAULT_FILTER, "today", now)).toMatchObject({ from: today, to: end, newOnly: false });
    expect(applyQuickPick(DEFAULT_FILTER, "week", now).from).toBe(Date.UTC(2026, 2, 16) / 1000);
    expect(applyQuickPick(DEFAULT_FILTER, "month", now).from).toBe(Date.UTC(2026, 2, 1) / 1000);
    expect(applyQuickPick(DEFAULT_FILTER, "year", now).from).toBe(Date.UTC(2026, 0, 1) / 1000);
    expect(applyQuickPick({ ...DEFAULT_FILTER, from: 1, to: 2 }, "new", now)).toMatchObject({ from: null, to: null, newOnly: true });
    expect(applyQuickPick({ ...DEFAULT_FILTER, newOnly: true, from: 1 }, "all", now)).toMatchObject({ from: null, to: null, newOnly: false });
  });

  it("groups items under day headers, in rows of the columns that fit", () => {
    const items = [item(0, day(2026, 3, 4, 18)), item(1, day(2026, 3, 4, 9)), item(2, day(2026, 3, 4, 8)), item(3, day(2026, 3, 1)), item(4, null)];
    const rows = buildRows(items, 2);
    expect(rows.map((r) => (r.kind === "header" ? `# ${r.ids.join(",")}` : r.items.map((i) => i.index).join(",")))).toEqual([
      "# 0,1,2",
      "0,1",
      "2",
      "# 3",
      "3",
      "# 4",
      "4",
    ]);
    const last = rows[5];
    expect(last.kind === "header" && last.label).toBe("No date");
    expect(columnsFor(1000, 120, 4)).toBe(8);
    expect(columnsFor(50, 120, 4)).toBe(1);
  });

  it("extends a selection along the order, and inverts only what is shown", () => {
    const order = [5, 3, 9, 1, 7];
    expect([...extendSelection(new Set([2]), order, 3, 1)].sort()).toEqual([1, 2, 3, 9]);
    expect([...extendSelection(new Set(), order, null, 9)]).toEqual([9]);
    // 40 hidden by the filter stays selected through an invert.
    expect([...invertWithin(new Set([5, 40]), [5, 3])].sort((a, b) => a - b)).toEqual([3, 40]);
  });

  it("knows which watcher speaks for which device", () => {
    const card = { kind: "volume", id: "E:\\" } as const;
    expect(stillPresent(card, "volumes", [{ mount: "F:\\" }])).toBe(false);
    expect(stillPresent(card, "volumes", [{ mount: "E:\\" }])).toBe(true);
    expect(stillPresent(card, "iphones", { devices: [] })).toBeNull();
    expect(stillPresent({ kind: "iphone", id: "u1" }, "iphones", { usbmuxdAvailable: true, devices: [{ udid: "u2" }] })).toBe(false);
    expect(stillPresent({ kind: "photosLibrary", disk: null, path: "C:/x.photoslibrary" }, "macdisks", [])).toBeNull();
    expect(stillPresent({ kind: "macFolder", disk: "disk3-part2", path: "/" }, "macdisks", [])).toBe(false);
  });
});
