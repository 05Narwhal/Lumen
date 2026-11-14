import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getPluginDeveloperMode, listPlugins, setPluginDeveloperMode } from "@/lib/plugins";
import { createStore } from "@/lib/store";
import type { PluginInfo } from "@/types/generated/PluginInfo";

/**
 * Developer mode, as the backend keeps it (it survives a restart, with or without a dev plugin
 * loaded). The store holds the last answer for every `usePlugins` caller.
 */
const developerStore = createStore<{ on: boolean }>({ on: false });

/** Turns Developer mode on or off, and remembers the answer for every `usePlugins` caller. */
export async function setDeveloperMode(on: boolean): Promise<void> {
  await setPluginDeveloperMode(on);
  developerStore.set({ on });
}

/** The installed plugins, refetched on `plugins://changed`, and whether Developer mode is on. */
export function usePlugins(): { plugins: PluginInfo[]; developerMode: boolean; reload: () => void } {
  const [plugins, setPlugins] = useState<PluginInfo[]>([]);
  const dev = developerStore.use();
  const reload = useCallback(() => {
    listPlugins()
      .then(setPlugins)
      .catch(() => {});
    getPluginDeveloperMode()
      .then((on) => developerStore.set({ on: on === true }))
      .catch(() => {});
  }, []);
  useEffect(() => {
    reload();
    const un = listen("plugins://changed", reload);
    return () => void un.then((f) => f()).catch(() => {});
  }, [reload]);
  return { plugins, developerMode: dev.on, reload };
}
