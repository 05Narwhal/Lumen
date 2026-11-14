import { useCallback, useRef, useState } from "react";

function read(key: string): Set<string> | null {
  try {
    const raw = localStorage.getItem(key);
    if (raw == null) return null;
    const v = JSON.parse(raw);
    return Array.isArray(v) ? new Set(v.filter((x): x is string => typeof x === "string")) : null;
  } catch {
    return null;
  }
}

function write(key: string, ids: Set<string>) {
  try {
    localStorage.setItem(key, JSON.stringify([...ids]));
  } catch {
    // Private windows refuse; the fold still works until the app closes.
  }
}

/**
 * The ids of the open folds in a tree, remembered per viewer in `localStorage[key]`. Until the
 * person opens or closes something, `defaultOpen` says what is open; it is asked on every render,
 * so it may depend on data that arrives late. A refusing or garbled storage is the same as none.
 */
export function useOpenSet(key: string, defaultOpen: () => Iterable<string> = () => []) {
  const [stored, setStored] = useState<Set<string> | null>(() => read(key));
  // The latest default, so a toggle made before the data arrived and one made after agree.
  const defaults = useRef(defaultOpen);
  defaults.current = defaultOpen;
  const ids = stored ?? new Set(defaultOpen());
  const toggle = useCallback(
    (id: string) => {
      setStored((prev) => {
        const next = new Set(prev ?? defaults.current());
        if (!next.delete(id)) next.add(id);
        write(key, next);
        return next;
      });
    },
    [key],
  );
  const set = useCallback(
    (id: string, open: boolean) => {
      setStored((prev) => {
        const next = new Set(prev ?? defaults.current());
        if (next.has(id) === open) return prev ?? next;
        if (open) next.add(id);
        else next.delete(id);
        write(key, next);
        return next;
      });
    },
    [key],
  );
  return { ids, has: (id: string) => ids.has(id), toggle, set };
}
