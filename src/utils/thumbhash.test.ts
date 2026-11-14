import { describe, expect, it } from "vitest";
import { thumbhashToDataUrl } from "./thumbhash";

// A real ThumbHash (the library README's example image).
const SAMPLE = "3OcRJYB4d3h/iIeHeEh3eIhw+j3A";

describe("thumbhashToDataUrl", () => {
  it("decodes a hash to a png data url and memoises", () => {
    const url = thumbhashToDataUrl(SAMPLE);
    expect(url.startsWith("data:image/png;base64,")).toBe(true);
    expect(thumbhashToDataUrl(SAMPLE)).toBe(url);
  });

  it("never throws on garbage", () => {
    expect(() => thumbhashToDataUrl("AA==")).not.toThrow();
    expect(() => thumbhashToDataUrl("not base64!")).not.toThrow();
  });
});
