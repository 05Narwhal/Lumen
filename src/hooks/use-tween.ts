import { useEffect, useRef, useState } from "react";

/** How far along `a`→`b` at `t` (0–1). */
export const lerpNumber = (a: number, b: number, t: number) => (t >= 1 ? b : a + (b - a) * t);

/** Element-wise, for a histogram's counts; a length change jumps to `b`. */
export const lerpArray = (a: number[], b: number[], t: number) => (t >= 1 || a.length !== b.length ? b : b.map((v, i) => a[i] + (v - a[i]) * t));

const smooth = (t: number) => t * t * (3 - 2 * t);
const reduced = () => typeof document !== "undefined" && document.documentElement.dataset.reducedMotion === "true";

/**
 * A value that glides to each new `target` over `ms` (smoothstep), from wherever it is —
 * so moving to another photo in Develop slides the sliders and morphs the histogram instead
 * of snapping (Round 4A). `jump` (a drag in progress) and reduced motion skip the glide.
 */
export function useTween<T>(target: T, mix: (a: T, b: T, t: number) => T, ms: number, jump = false): T {
  const [shown, setShown] = useState(target);
  const shownRef = useRef(shown);
  shownRef.current = shown;
  const mixRef = useRef(mix);
  mixRef.current = mix;
  useEffect(() => {
    if (jump || reduced() || ms <= 0) {
      setShown(target);
      return;
    }
    const from = shownRef.current;
    // Nothing to travel: no frames at all (sixty sliders mount at once in Develop).
    if (Object.is(from, target)) return;
    const start = performance.now();
    let frame = 0;
    const step = () => {
      const t = Math.min(1, (performance.now() - start) / ms);
      setShown(mixRef.current(from, target, smooth(t)));
      if (t < 1) frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
    return () => cancelAnimationFrame(frame);
  }, [target, ms, jump]);
  // A jump shows at once, not a frame later.
  return jump || reduced() ? target : shown;
}
