import { invoke } from "@/lib/ipc";
import type { InstalledTheme } from "@/types/generated/InstalledTheme";

// Round 26 lane B: themes as `.lumentheme` packages, apart from plugins.

export const themesList = () => invoke<InstalledTheme[]>("themes_list");
/** *Get*: downloads the store's package, checks it against the index and installs it. */
export const themeInstall = (storeId: string) => invoke<InstalledTheme>("theme_install", { storeId });
export const themeRemove = (id: string) => invoke<void>("theme_remove", { id });
/** *Try*: shows a store theme without installing it. */
export const themeTry = (storeId: string) => invoke<InstalledTheme>("theme_try", { storeId });
export const themeTryEnd = () => invoke<void>("theme_try_end");
/** *Keep* after *Try*: installs the theme being tried. */
export const themeKeep = () => invoke<InstalledTheme>("theme_keep");
/** Makes an installed (or the tried) theme the app's theme. */
export const themeApply = (id: string) => invoke<void>("theme_apply", { id });
