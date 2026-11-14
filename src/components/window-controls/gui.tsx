import { IconMinus, IconSquare, IconX } from "@tabler/icons-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./styles.scss";

export const isMac = () => typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);

/** Minimize / maximize / close for Windows and Linux. macOS keeps its native traffic lights. */
export function WindowControls() {
  if (isMac()) return null;
  const w = getCurrentWindow();
  return (
    <div className="window-controls" role="group" aria-label="Window">
      <button type="button" className="window-controls__btn" aria-label="Minimize" onClick={() => w.minimize()}>
        <IconMinus size={14} />
      </button>
      <button type="button" className="window-controls__btn" aria-label="Maximize" onClick={() => w.toggleMaximize()}>
        <IconSquare size={12} />
      </button>
      <button
        type="button"
        className="window-controls__btn window-controls__btn--close"
        aria-label="Close"
        aria-keyshortcuts="Alt+F4"
        // Never awaited: nothing on the page may hold Close (Round 15).
        onClick={() => void w.close()}
      >
        <IconX size={15} />
      </button>
    </div>
  );
}
