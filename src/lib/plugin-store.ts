import { invoke } from "@/lib/ipc";
import type { StoreDownload } from "@/types/generated/StoreDownload";
import type { StoreIndex } from "@/types/generated/StoreIndex";
import type { StoreStatus } from "@/types/generated/StoreStatus";

// Round 24 lane C: the plugin store's commands (spec §7).

/** The store's list; the cached one when offline (`storeStatus().offline`). */
export const storeIndex = () => invoke<StoreIndex>("store_index");
/** *Get*: downloads and verifies; open the install sheet on `path`. */
export const storeInstall = (id: string) => invoke<StoreDownload>("store_install", { id });
/** *Update*: installed at once, or `path` for the install sheet when it asks for new permissions. */
export const storeUpdate = (id: string) => invoke<StoreDownload>("store_update", { id });
export const storeStatus = () => invoke<StoreStatus>("store_status");
export const storeSetAutoUpdate = (on: boolean) => invoke<void>("store_set_auto_update", { on });
/** A plugin's README text from the store (cached, so it works offline); `null` when it lists none. */
export const storeReadme = (id: string) => invoke<string | null>("store_readme", { id });
