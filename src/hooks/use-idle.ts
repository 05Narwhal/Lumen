import { useEffect, useState, type Dispatch, type SetStateAction } from "react";

/** How long the viewer and the film player wait before their controls step aside. */
export const IDLE_MS = 2000;

/**
 * Idle after `ms` without a pointer move, a press or a key (Round 8B; the viewer's was
 * pointer moves only). Never idle while the pointer rests on something marked
 * `data-keep-awake` (the playback bar) or while a menu is open. Tab is left alone: it is the
 * viewer's own key for hiding the controls by hand, through the returned setter.
 */
export function useIdle(enabled: boolean, ms = IDLE_MS): [boolean, Dispatch<SetStateAction<boolean>>] {
  const [idle, setIdle] = useState(false);
  useEffect(() => {
    if (!enabled) {
      setIdle(false);
      return;
    }
    let over: Element | null = null;
    let timer: ReturnType<typeof setTimeout>;
    const arm = () => {
      clearTimeout(timer);
      timer = setTimeout(() => {
        if (over?.closest("[data-keep-awake]") || document.querySelector('[role="menu"]')) arm();
        else setIdle(true);
      }, ms);
    };
    const wake = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key === "Tab") return;
      // Where the pointer is, not where a key was pressed: a focused seek bar is not hovered.
      if (e instanceof PointerEvent && e.target instanceof Element) over = e.target;
      setIdle(false);
      arm();
    };
    const events = ["pointermove", "pointerdown", "keydown"] as const;
    for (const ev of events) window.addEventListener(ev, wake, true);
    arm();
    return () => {
      clearTimeout(timer);
      for (const ev of events) window.removeEventListener(ev, wake, true);
    };
  }, [enabled, ms]);
  return [idle, setIdle];
}
