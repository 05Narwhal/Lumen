import { IconMinus, IconSquare, IconX } from "@tabler/icons-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";
import "./styles.scss";

export const isMac = () => typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);

/** Two offset squares, the Windows "Restore down" glyph. */
function RestoreGlyph() {
  return (
    <svg width={12} height={12} viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth={1.2} aria-hidden>
      <rect x={1} y={3.5} width={7.5} height={7.5} rx={1} />
      <path d="M3.5 3.5V2a1 1 0 0 1 1-1H10a1 1 0 0 1 1 1v5.5a1 1 0 0 1-1 1H8.5" />
    </svg>
  );
}

/** Whether the window is maximized, following resizes; false where the window API is missing. */
function useMaximized() {
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    let live = true;
    let unlisten: (() => void) | undefined;
    const read = () =>
      getCurrentWindow()
        .isMaximized()
        .then((m) => live && setMaximized(m))
        .catch(() => {});
    try {
      read();
      getCurrentWindow()
        .onResized(read)
        .then((u) => (live ? (unlisten = u) : u()))
        .catch(() => {});
    } catch {
      // The browser shim has no window.
    }
    return () => {
      live = false;
      unlisten?.();
    };
  }, []);
  return maximized;
}

/** Minimize / maximize / close for Windows and Linux. macOS keeps its native traffic lights. */
export function WindowControls() {
  if (isMac()) return null;
  return <Controls />;
}

function Controls() {
  const w = getCurrentWindow();
  const maximized = useMaximized();
  return (
    <div className="window-controls" data-part="window-controls" role="group" aria-label="Window">
      <button type="button" className="window-controls__btn" aria-label="Minimize" onClick={() => w.minimize()}>
        <IconMinus size={14} />
      </button>
      <button
        type="button"
        className="window-controls__btn"
        aria-label={maximized ? "Restore" : "Maximize"}
        onClick={() => w.toggleMaximize()}
      >
        {maximized ? <RestoreGlyph /> : <IconSquare size={12} />}
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
