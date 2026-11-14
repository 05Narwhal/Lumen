import { describe, expect, it } from "vitest";
import type { Section } from "@/types/generated/Section";
import { buildMarks, dateAt, offsetForPointer, thumbFor, visibleLabels, type Mark } from "./marks";

const section = (index: number, year: number, month: number, y: number, height: number, title = `${month}/${year}`): Section => ({
  index,
  year,
  month,
  title,
  y,
  height,
  firstRow: index,
  rowCount: 1,
  itemCount: 1,
  albumId: null,
});

// Newest first, as the grid lays them out: 2024 is most of the library, 2009 a sliver.
const SECTIONS = [
  section(0, 2024, 5, 0, 3000),
  section(1, 2024, 2, 3000, 3000),
  section(2, 2023, 12, 6000, 3000),
  section(3, 2009, 7, 9000, 1000),
];
const TOTAL = 10_000;

describe("buildMarks", () => {
  it("puts one year mark at the start of each year, as a share of the height", () => {
    const years = buildMarks(SECTIONS, TOTAL, "month").filter((m) => m.kind === "year");
    expect(years.map((m) => [m.label, m.at])).toEqual([
      ["2024", 0],
      ["2023", 0.6],
      ["2009", 0.9],
    ]);
  });

  it("adds a month mark per section when grouped by day or month, never by year", () => {
    expect(buildMarks(SECTIONS, TOTAL, "month").filter((m) => m.kind === "month")).toHaveLength(4);
    expect(buildMarks(SECTIONS, TOTAL, "year").filter((m) => m.kind === "month")).toHaveLength(0);
  });

  it("gives a month one mark however many days it holds", () => {
    const days = [section(0, 2024, 5, 0, 100), section(1, 2024, 5, 100, 100), section(2, 2024, 4, 200, 100)];
    const months = buildMarks(days, 300, "day").filter((m) => m.kind === "month");
    expect(months.map((m) => m.label)).toEqual(["2024-5", "2024-4"]);
  });

  it("an_album_in_manual_order_has_no_labels", () => {
    expect(buildMarks([section(0, 2024, 5, 0, 5000, "")], 5000, null)).toEqual([]);
  });
});

describe("visibleLabels", () => {
  const year = (label: string, at: number): Mark => ({ kind: "year", label, at, span: 0 });

  it("labels_never_overlap", () => {
    // On a 400 px track, 0.9 and 0.93 are 12 px apart: only one can be drawn.
    const marks = buildMarks([...SECTIONS, section(4, 2008, 3, 9300, 700)], TOTAL, "month");
    const shown = visibleLabels(marks, 400);
    const ys = shown.map((m) => m.at * 400);
    for (let i = 1; i < ys.length; i++) expect(ys[i] - ys[i - 1]).toBeGreaterThanOrEqual(18);
    // 2009 spans 300 px of scroll, 2008 spans 700 px: the bigger year keeps its label.
    expect(shown.map((m) => m.label)).toEqual(["2024", "2023", "2008"]);
  });

  it("keeps every year when there is room", () => {
    expect(visibleLabels([year("2024", 0), year("2023", 0.5)], 400).map((m) => m.label)).toEqual(["2024", "2023"]);
  });
});

describe("dateAt", () => {
  it("names the month under the offset, or the year in the Years view", () => {
    const named = [section(0, 2024, 5, 0, 3000, "May 2024 · Lisbon"), section(1, 2024, 2, 3000, 3000, "February 2024")];
    expect(dateAt(named, 3500, "month")).toBe("February 2024");
    expect(dateAt(named, 100, "day")).toBe("May 2024");
    expect(dateAt(named, 100, "year")).toBe("2024");
    expect(dateAt(named, 100, null)).toBe("");
  });
});

describe("thumb and pointer", () => {
  it("places the thumb by scroll position and maps a pointer back to an offset", () => {
    // 10,000 px of content in an 800 px view, on a 400 px track.
    expect(thumbFor(0, 800, TOTAL, 400)).toBe(0);
    expect(thumbFor(TOTAL - 800, 800, TOTAL, 400)).toBe(400);
    expect(thumbFor((TOTAL - 800) / 2, 800, TOTAL, 400)).toBe(200);
    expect(offsetForPointer(200, 400, TOTAL, 800)).toBe((TOTAL - 800) / 2);
    expect(offsetForPointer(-50, 400, TOTAL, 800)).toBe(0);
    expect(offsetForPointer(900, 400, TOTAL, 800)).toBe(TOTAL - 800);
  });
});
