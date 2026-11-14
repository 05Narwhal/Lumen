import { describe, expect, it } from "vitest";
import { buildMediaUrl } from "./media";

describe("buildMediaUrl", () => {
  it("uses the scheme form on mac/linux", () => {
    expect(buildMediaUrl(false, "thumb", 42, { s: 256 })).toBe("photos://localhost/thumb/42?s=256");
  });

  it("uses the http host form on windows", () => {
    expect(buildMediaUrl(true, "video", 7)).toBe("http://photos.localhost/video/7");
  });
});
