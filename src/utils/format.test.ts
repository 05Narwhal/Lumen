import { describe, expect, it } from "vitest";
import { formatStorage } from "./format";

describe("formatStorage", () => {
  it("counts in thousands, as card labels, Apple and camera makers do", () => {
    expect(formatStorage(127_900_000_000)).toBe("128 GB");
    expect(formatStorage(20_100_000_000)).toBe("20 GB");
    expect(formatStorage(2_040_000_000)).toBe("2.0 GB");
    expect(formatStorage(4_000_000_000_000)).toBe("4.0 TB");
    expect(formatStorage(850_000_000)).toBe("850 MB");
  });
});

describe("formatTimeLeft", () => {
  it("rounds to what a person would say", async () => {
    const { formatTimeLeft } = await import("./format");
    expect(formatTimeLeft(20_000)).toBe("less than a minute left");
    expect(formatTimeLeft(80_000)).toBe("about 1 min left");
    expect(formatTimeLeft(4 * 60_000 + 20_000)).toBe("about 4 min left");
    expect(formatTimeLeft(80 * 60_000)).toBe("about 1 h 20 min left");
    expect(formatTimeLeft(3 * 3_600_000)).toBe("about 3 h left");
  });
});
