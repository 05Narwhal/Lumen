import { describe, expect, it } from "vitest";
import { packageSize, permissionWords } from "./plugin-permission-words";

describe("plugin permission words", () => {
  it("words the known permissions and shows an unknown one as written", () => {
    expect(permissionWords("library.read")).toMatch(/library/i);
    expect(permissionWords("future.power")).toBe("future.power");
  });

  it("words the API 1.3 proposal permissions as review-first", () => {
    for (const id of ["dates.write", "tags.write", "captions.write", "people.write"]) {
      expect(permissionWords(id)).toMatch(/^Suggest .*until you review and accept$/);
    }
  });

  it("sizes a package", () => {
    expect(packageSize(900)).toBe("900 B");
    expect(packageSize(20 * 1024)).toBe("20 KB");
    expect(packageSize(3 * 1024 * 1024)).toBe("3.0 MB");
  });
});
