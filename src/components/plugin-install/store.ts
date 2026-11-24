import { pickFile } from "@/lib/dialogs";
import { createStore } from "@/lib/store";

/** The package whose install sheet is open (`null` = closed). One sheet only: a second open replaces it. */
export const pluginInstallStore = createStore<{ path: string | null }>({ path: null });

/** The `.lumentheme` whose install sheet is open (`null` = closed). */
export const themeInstallStore = createStore<{ path: string | null }>({ path: null });
export const closeThemeInstall = () => themeInstallStore.set({ path: null });

const isTheme = (path: string) => path.toLowerCase().endsWith(".lumentheme");

/** Opens the install sheet for a package from the system or a picker: a theme's own, or a plugin's. */
export const openPluginInstall = (path: string) => (isTheme(path) ? themeInstallStore.set({ path }) : pluginInstallStore.set({ path }));
export const closePluginInstall = () => pluginInstallStore.set({ path: null });

/** Asks for a `.lumenplugin` file and opens its install sheet; cancelling does nothing. */
export async function pickPluginToInstall(): Promise<void> {
  const path = await pickFile({ title: "Install a plugin", filters: [{ name: "Lumen plugin", extensions: ["lumenplugin"] }] });
  if (typeof path === "string") openPluginInstall(path);
}

/** Asks for a `.lumentheme` file and opens its install sheet; cancelling does nothing. */
export async function pickThemeToInstall(): Promise<void> {
  const path = await pickFile({ title: "Install a theme", filters: [{ name: "Lumen theme", extensions: ["lumentheme"] }] });
  if (typeof path === "string") themeInstallStore.set({ path });
}
