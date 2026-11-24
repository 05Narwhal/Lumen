import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { storeStatus } from "@/lib/plugin-store";
import type { StoreStatus } from "@/types/generated/StoreStatus";

/**
 * The plugin store's status (updates found, what the background task installed, offline), read
 * again when the plugin list changes or the background task installed an update.
 */
export function useStoreStatus(): { status: StoreStatus | null; reload: () => void } {
  const [status, setStatus] = useState<StoreStatus | null>(null);
  const reload = useCallback(() => {
    storeStatus()
      .then((s) => setStatus(s && typeof s === "object" ? s : null))
      .catch(() => {});
  }, []);
  useEffect(() => {
    reload();
    const subs = [listen("plugins://changed", reload), listen("app://plugins-updated", reload)];
    return () => subs.forEach((u) => void u.then((f) => f()).catch(() => {}));
  }, [reload]);
  return { status, reload };
}
