import { useCallback, useEffect, useState } from "react";
import { isAppError } from "@/lib/ipc";
import { storeIndex } from "@/lib/plugin-store";
import type { StoreIndex } from "@/types/generated/StoreIndex";

export type StoreIndexState =
  | { phase: "loading" }
  | { phase: "ready"; index: StoreIndex }
  /** No network and nothing cached. */
  | { phase: "offline" }
  | { phase: "failed"; message: string };

/** The store's list, fetched when the page opens (the cached one when offline) and on `reload`. */
export function useStoreIndex(): { state: StoreIndexState; reload: () => void } {
  const [state, setState] = useState<StoreIndexState>({ phase: "loading" });
  const [tick, setTick] = useState(0);
  const reload = useCallback(() => setTick((n) => n + 1), []);
  useEffect(() => {
    let live = true;
    setState((s) => (s.phase === "ready" ? s : { phase: "loading" }));
    storeIndex().then(
      (index) => {
        if (live) setState(index && Array.isArray(index.plugins) ? { phase: "ready", index } : { phase: "failed", message: "The store's list could not be read." });
      },
      (e) => {
        if (!live) return;
        if (isAppError(e) && e.code === "store_offline") setState({ phase: "offline" });
        else setState({ phase: "failed", message: isAppError(e) ? e.message : "The plugin store could not be reached." });
      },
    );
    return () => {
      live = false;
    };
  }, [tick]);
  return { state, reload };
}
