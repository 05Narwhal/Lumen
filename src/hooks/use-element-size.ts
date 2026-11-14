import { useEffect, useRef, useState } from "react";

/** Debounced content-box size of an element, for layouts that must be recomputed on resize. */
export function useElementSize<T extends HTMLElement>(debounceMs = 100) {
  const ref = useRef<T>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const apply = () => setSize({ width: Math.round(el.clientWidth), height: Math.round(el.clientHeight) });
    apply();
    const ro = new ResizeObserver(() => {
      clearTimeout(timer);
      timer = setTimeout(apply, debounceMs);
    });
    ro.observe(el);
    return () => {
      clearTimeout(timer);
      ro.disconnect();
    };
  }, [debounceMs]);
  return [ref, size] as const;
}
