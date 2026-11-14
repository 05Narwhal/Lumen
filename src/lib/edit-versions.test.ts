import { beforeEach, describe, expect, it } from "vitest";
import { mediaUrl } from "@/utils/media";
import { editVersion, setEditVersions } from "./edit-versions";

describe("edit versions", () => {
  beforeEach(() => setEditVersions([]));

  it("adds the version to every image URL of an edited asset", () => {
    setEditVersions([{ assetId: 12, version: 1789000000001 }]);
    const url = mediaUrl("thumb", 12, { s: 256 });
    expect(url).toContain("s=256");
    expect(url).toContain("v=1789000000001");
    expect(mediaUrl("preview", 12)).toContain("v=1789000000001");
    expect(mediaUrl("full", 12)).toContain("v=1789000000001");
  });

  it("leaves unedited assets, videos and device previews alone", () => {
    setEditVersions([{ assetId: 12, version: 5 }]);
    expect(mediaUrl("thumb", 13, { s: 256 })).not.toContain("v=");
    expect(mediaUrl("video", 12)).not.toContain("v=");
    // A device preview's id is a token, not an asset id.
    expect(mediaUrl("device-thumb", 12)).not.toContain("v=");
  });

  it("forgets a reverted asset", () => {
    setEditVersions([{ assetId: 12, version: 5 }]);
    setEditVersions([]);
    expect(editVersion(12)).toBeUndefined();
    expect(mediaUrl("thumb", 12, { s: 256 })).not.toContain("v=");
  });
});
