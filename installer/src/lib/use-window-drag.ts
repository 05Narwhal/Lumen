import { getCurrentWindow } from "@tauri-apps/api/window";
import { useMemo, type MouseEvent } from "react";

const INTERACTIVE = 'button, a, input, select, textarea, [role="button"], [role="switch"], [data-no-drag]';
const onEmpty = (e: MouseEvent) => e.target instanceof Element && e.target.closest(INTERACTIVE) === null;

/** Drags the window from any empty part of the bar (the same recipe as the app's top bar). */
export function useWindowDrag() {
  return useMemo(
    () => ({
      onMouseDown(e: MouseEvent) {
        if (e.button === 0 && e.detail < 2 && onEmpty(e)) void getCurrentWindow().startDragging().catch(() => {});
      },
    }),
    [],
  );
}
