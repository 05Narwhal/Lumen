import { describe, expect, it } from "vitest";
import { pickAlbumId, sidebarRail } from "./pick-mode";

describe("sidebarRail", () => {
  it("is the saved setting outside pick mode, whatever peek says", () => {
    expect(sidebarRail(false, false, false)).toBe(false);
    expect(sidebarRail(true, false, false)).toBe(true);
    expect(sidebarRail(false, false, true)).toBe(false);
    expect(sidebarRail(true, false, true)).toBe(true);
  });

  it("rails by itself while picking, until the person peeks", () => {
    expect(sidebarRail(false, true, false)).toBe(true);
    expect(sidebarRail(false, true, true)).toBe(false);
  });

  it("never opens a sidebar the person had railed", () => {
    expect(sidebarRail(true, true, true)).toBe(true);
    expect(sidebarRail(true, true, false)).toBe(true);
  });
});

describe("pickAlbumId", () => {
  it("reads the album from ?pick=", () => {
    expect(pickAlbumId("?pick=album:7")).toBe(7);
    expect(pickAlbumId("?pick=person:7")).toBeNull();
  });
});
