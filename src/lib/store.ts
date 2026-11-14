import { useSyncExternalStore } from "react";

/** Minimal external store for UI-only state (toasts, jobs, palette). */
export function createStore<T>(initial: T) {
  let state = initial;
  const subs = new Set<() => void>();
  const get = () => state;
  const set = (updater: T | ((prev: T) => T)) => {
    state = typeof updater === "function" ? (updater as (p: T) => T)(state) : updater;
    subs.forEach((fn) => fn());
  };
  const subscribe = (fn: () => void) => {
    subs.add(fn);
    return () => {
      subs.delete(fn);
    };
  };
  const use = () => useSyncExternalStore(subscribe, get, get);
  return { get, set, subscribe, use };
}
