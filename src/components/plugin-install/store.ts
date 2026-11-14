import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { createStore } from "@/lib/store";

/** The package whose install sheet is open (`null` = closed). One sheet only: a second open replaces it. */
export const pluginInstallStore = createStore<{ path: string | null }>({ path: null });

export const openPluginInstall = (path: string) => pluginInstallStore.set({ path });
export const closePluginInstall = () => pluginInstallStore.set({ path: null });

/** Asks for a `.lumenplugin` file and opens its install sheet; cancelling does nothing. */
export async function pickPluginToInstall(): Promise<void> {
  const path = await openDialog({ multiple: false, title: "Install a plugin", filters: [{ name: "Lumen plugin", extensions: ["lumenplugin"] }] });
  if (typeof path === "string") openPluginInstall(path);
}
