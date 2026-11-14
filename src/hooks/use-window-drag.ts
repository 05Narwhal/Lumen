import { getCurrentWindow } from "@tauri-apps/api/window";
import { useMemo, type MouseEvent } from "react";

/** Anything a press on should act on, not drag the window. */
const INTERACTIVE =
  'button, a, input, select, textarea, [role="button"], [role="slider"], [role="tab"], [contenteditable], [data-no-drag]';

const onEmpty = (e: MouseEvent) => e.target instanceof Element && e.target.closest(INTERACTIVE) === null;

/**
 * Drags the window from any empty part of an element (Round 15). `data-tauri-drag-region` only
 * works on the element itself, so a screen's toolbar in the title bar used to block dragging
 * everywhere but the gaps it left. Spread the result on the bar: `<div {...useWindowDrag()}>`.
 */
export function useWindowDrag() {
  return useMemo(
    () => ({
      onMouseDown(e: MouseEvent) {
        if (e.button === 0 && e.detail < 2 && onEmpty(e)) void getCurrentWindow().startDragging();
      },
      onDoubleClick(e: MouseEvent) {
        if (onEmpty(e)) void getCurrentWindow().toggleMaximize();
      },
    }),
    [],
  );
}
