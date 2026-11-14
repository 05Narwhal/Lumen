import { describe, expect, it } from "vitest";
import { proxyFailureMessage } from "./proxy-store";

describe("proxy failures", () => {
  it("speaks up only for a video someone asked to watch (Round 9A)", () => {
    expect(proxyFailureMessage({ assetId: 1, ok: false, background: false })).toBe("This video could not be converted for playback.");
    expect(proxyFailureMessage({ assetId: 1, ok: false, background: true })).toBeNull();
    expect(proxyFailureMessage({ assetId: 1, ok: true, background: false })).toBeNull();
  });
});
