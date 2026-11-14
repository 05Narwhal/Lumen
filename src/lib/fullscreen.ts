import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * Put the app window in (or out of) full screen. In the app that is the Tauri window, so the
 * picture covers the whole display and the taskbar; outside Tauri (the browser shim) the
 * document's own Fullscreen API stands in.
 */
export async function setWindowFullscreen(on: boolean): Promise<void> {
  try {
    await getCurrentWindow().setFullscreen(on);
  } catch {
    try {
      if (on) await document.documentElement.requestFullscreen?.();
      else if (document.fullscreenElement) await document.exitFullscreen?.();
    } catch {
      // Refused (no user gesture, or unsupported): the viewer still fills the window.
    }
  }
}
