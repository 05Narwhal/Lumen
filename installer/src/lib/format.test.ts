import { describe, expect, it } from "vitest";
import { formatBytes, shortPath } from "./format";

const DEEP = "C:\\Users\\Sample\\AppData\\Local\\Temp\\claude\\d340f0a98b1451a1\\5a26ffa8-fdea-4d1d-ad2f-dd13a3dfd4cc\\scratchpad\\p8\\Lumen Library";

describe("shortPath", () => {
  it("leaves a path that fits alone", () => {
    expect(shortPath("C:\\Users\\Sample\\Pictures\\Lumen Library")).toBe("C:\\Users\\Sample\\Pictures\\Lumen Library");
  });

  it("keeps the drive, the user folder and the end, eliding the middle", () => {
    const s = shortPath(DEEP);
    expect(s).toBe("C:\\Users\\Sample\\…\\scratchpad\\p8\\Lumen Library");
    expect(s.length).toBeLessThanOrEqual(56);
  });

  it("always keeps the last folder whole, even when it alone is long", () => {
    const name = "A very long library folder name that goes on and on and on";
    expect(shortPath(`C:\\Users\\Sample\\Pictures\\Deep\\${name}`, 40)).toBe(`C:\\Users\\Sample\\…\\${name}`);
  });

  it("works with forward slashes too", () => {
    expect(shortPath("/home/sample/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/Lumen Library", 40)).toBe("/home/sample/…/k/l/m/n/o/p/Lumen Library");
  });
});

describe("formatBytes", () => {
  it("says MB below a GB and tenths of a GB above", () => {
    expect(formatBytes(500 * 1024 * 1024)).toBe("500 MB");
    expect(formatBytes(1.9 * 1024 ** 3)).toBe("1.9 GB");
  });
});
