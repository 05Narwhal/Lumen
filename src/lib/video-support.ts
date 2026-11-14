import { invoke } from "@/lib/ipc";
import type { WebviewVideoSupport } from "@/types/generated/WebviewVideoSupport";

/**
 * What this WebView can play (Round 8A). HEVC needs Windows' HEVC Video Extensions, and an
 * iPhone's HDR video (10-bit HEVC, Main 10) a decoder for it. Rust decides from this which
 * videos play as they are and which need a proxy, so it is asked of the WebView itself rather
 * than guessed from the system.
 */
export function detectVideoSupport(): WebviewVideoSupport {
  const plays = (type: string) => {
    try {
      if (typeof MediaSource !== "undefined" && MediaSource.isTypeSupported(type)) return true;
      return document.createElement("video").canPlayType(type) === "probably";
    } catch {
      return false;
    }
  };
  return {
    hevc: plays('video/mp4; codecs="hvc1.1.6.L93.B0"'),
    hevcMain10: plays('video/mp4; codecs="hvc1.2.4.L153.B0"'),
  };
}

let reported: Promise<void> | null = null;

/** Tells Rust once per run; every later call waits on the same answer. */
export function reportVideoSupport(): Promise<void> {
  reported ??= invoke("set_video_support", { support: detectVideoSupport() }).then(
    () => undefined,
    () => undefined,
  );
  return reported;
}

/** For tests: ask again. */
export function resetVideoSupport() {
  reported = null;
}
