import { describe, expect, it } from "vitest";
import { byteToIndex, removeSpan, replaceSpan } from "./search-text";

describe("search text", () => {
  it("converts Rust's byte offsets to string indexes", () => {
    expect(byteToIndex("anna beach", 5)).toBe(5);
    // "ü" is two bytes in UTF-8 and one unit in JavaScript.
    expect(byteToIndex("zürich 2023", 8)).toBe(7);
    expect(byteToIndex("📷 dog", 5)).toBe(3);
  });

  it("removes a chip's words and tidies the spaces", () => {
    expect(removeSpan("anna beach 2023", [5, 10])).toBe("anna 2023");
    expect(removeSpan("zürich beach", [0, 7])).toBe("beach");
  });

  it("writes another reading as its token", () => {
    expect(replaceSpan("florence 2024", [0, 8], 'person:"Florence"')).toBe('person:"Florence" 2024');
  });
});
