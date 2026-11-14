import { afterEach, describe, expect, it, vi } from "vitest";
import { invokeMock } from "@/test/tauri-mock";
import { detectVideoSupport, reportVideoSupport, resetVideoSupport } from "./video-support";

describe("video support", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    resetVideoSupport();
  });

  it("asks the WebView, not the system, what it plays", () => {
    vi.stubGlobal("MediaSource", { isTypeSupported: (t: string) => t.includes("hvc1.1.") });
    expect(detectVideoSupport()).toEqual({ hevc: true, hevcMain10: false });
    vi.stubGlobal("MediaSource", { isTypeSupported: () => true });
    expect(detectVideoSupport()).toEqual({ hevc: true, hevcMain10: true });
  });

  it("tells Rust once per run", async () => {
    vi.stubGlobal("MediaSource", { isTypeSupported: () => false });
    await Promise.all([reportVideoSupport(), reportVideoSupport()]);
    await reportVideoSupport();
    const calls = invokeMock.mock.calls.filter(([cmd]) => cmd === "set_video_support");
    expect(calls).toHaveLength(1);
    expect(calls[0][1]).toEqual({ support: { hevc: false, hevcMain10: false } });
  });
});
