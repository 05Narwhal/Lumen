import { useCallback, useEffect, useRef, useState } from "react";

/** Pixels the pointer must travel before a press becomes a drag (a click stays a click). */
const THRESHOLD = 5;

export interface PersonDrag {
  /** The person being dragged, once the pointer has moved past the threshold. */
  person: number | null;
  /** The `data-drop` key under the pointer. */
  over: string | null;
}

/**
 * Dragging people on the People page with pointer events. HTML5 drag and drop never reaches
 * the page on Windows: Tauri's file drop (which Import needs for real paths) takes every drag
 * over the webview. Drop targets carry `data-drop="<key>"`; `onDrop(person, key)` gets the
 * one under the pointer on release. The ghost element follows the pointer through `ghostRef`
 * without re-rendering the page on every move.
 */
export function usePersonDrag(onDrop: (person: number, target: string) => void) {
  const [drag, setDrag] = useState<PersonDrag>({ person: null, over: null });
  const ghostRef = useRef<HTMLElement | null>(null);
  const press = useRef<{ person: number; x: number; y: number; active: boolean; over: string | null } | null>(null);
  const drop = useRef(onDrop);
  drop.current = onDrop;

  const end = useCallback(() => {
    press.current = null;
    setDrag({ person: null, over: null });
  }, []);

  useEffect(() => {
    const move = (e: PointerEvent) => {
      const p = press.current;
      if (!p) return;
      if (!p.active) {
        if (Math.hypot(e.clientX - p.x, e.clientY - p.y) < THRESHOLD) return;
        p.active = true;
        setDrag({ person: p.person, over: null });
      }
      const ghost = ghostRef.current;
      if (ghost) ghost.style.transform = `translate(${e.clientX}px, ${e.clientY}px)`;
      const hit = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>("[data-drop]");
      const key = hit?.dataset.drop ?? null;
      const over = key === `person:${p.person}` ? null : key;
      if (over !== p.over) {
        p.over = over;
        setDrag({ person: p.person, over });
      }
    };
    const up = () => {
      const p = press.current;
      if (!p) return;
      if (p.active) {
        // The release ends a drag, not a click: keep the card's link from opening.
        // The click follows pointerup in the same turn; if none comes, the timer drops it.
        const swallow = (e: MouseEvent) => (e.preventDefault(), e.stopPropagation());
        window.addEventListener("click", swallow, { capture: true, once: true });
        setTimeout(() => window.removeEventListener("click", swallow, { capture: true }), 0);
        if (p.over) drop.current(p.person, p.over);
      }
      end();
    };
    const key = (e: KeyboardEvent) => e.key === "Escape" && press.current?.active && end();
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", end);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", end);
      window.removeEventListener("keydown", key);
    };
  }, [end]);

  /** Spread on a person's card. */
  const handle = useCallback(
    (person: number) => ({
      onPointerDown: (e: React.PointerEvent) => {
        if (e.button !== 0 || (e.target as HTMLElement).closest("input")) return;
        press.current = { person, x: e.clientX, y: e.clientY, active: false, over: null };
      },
      // Native drags of the link or image would start instead of ours.
      onDragStart: (e: React.DragEvent) => e.preventDefault(),
    }),
    [],
  );

  return { drag, handle, ghostRef };
}
