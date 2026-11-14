// The one place the UI talks to Rust. Outside Tauri (`yarn dev` in a browser, headless-Edge
// screenshots) the same calls are answered by dev/browser-shim.ts.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { BinCheck, Closing, DeleteChoice, Deleted, FolderCheck, InstallOptions, InstallResult, InstallState, LibraryDetail, SetupProgress } from "@/types";
import { shim } from "@/dev/browser-shim";

export const inTauri = (): boolean => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const PROGRESS_EVENT = "setup://progress";
export const LUMEN_EVENT = "setup://lumen";

export interface SetupApi {
  getInstallState(): Promise<InstallState>;
  checkFolder(path: string, needBytes: number): Promise<FolderCheck>;
  /** Opens the folder picker; null when the person cancels. */
  chooseFolder(current: string): Promise<string | null>;
  /** Runs to the end: Lumen, then the models. Progress arrives as events. */
  install(opts: InstallOptions): Promise<InstallResult>;
  skipModels(): Promise<void>;
  /** Stops Setup's download first (the `.part` stays), then starts Lumen. */
  openLumen(): Promise<void>;
  /** Closes Lumen first (no time limit), removes it, then deletes the typed-for libraries. Rejects with a sentence. */
  uninstall(removeData: boolean, deleteLibraries: DeleteChoice | null): Promise<Deleted[]>;
  /** Screen 6b: every registered library with its photos and size, or why Setup won't delete it. */
  libraryDetails(): Promise<LibraryDetail[]>;
  /** Screen 6b: do these libraries fit the Recycle Bin? */
  checkRecycleBin(paths: string[]): Promise<BinCheck>;
  /** Cancel while Lumen closes: stop waiting; nothing is changed. */
  cancelWait(): Promise<void>;
  onProgress(handler: (p: SetupProgress) => void): Promise<() => void>;
  /** A running Lumen closing before install, update, repair or removal. */
  onLumen(handler: (c: Closing) => void): Promise<() => void>;
  minimize(): Promise<void>;
  close(): Promise<void>;
}

const real: SetupApi = {
  getInstallState: () => invoke("get_install_state"),
  checkFolder: (path, needBytes) => invoke("check_folder", { path, needBytes }),
  chooseFolder: (current) => invoke("choose_folder", { start: current }),
  install: (opts) => invoke("install", { opts }),
  skipModels: () => invoke("skip_models"),
  openLumen: () => invoke("open_lumen"),
  uninstall: (removeData, deleteLibraries) => invoke("uninstall", { removeData, deleteLibraries }),
  libraryDetails: () => invoke("library_details"),
  checkRecycleBin: (paths) => invoke("check_recycle_bin", { paths }),
  cancelWait: () => invoke("cancel_wait"),
  onProgress: async (handler) => listen<SetupProgress>(PROGRESS_EVENT, (e) => handler(e.payload)),
  onLumen: async (handler) => listen<Closing>(LUMEN_EVENT, (e) => handler(e.payload)),
  minimize: async () => (await import("@tauri-apps/api/window")).getCurrentWindow().minimize(),
  close: async () => (await import("@tauri-apps/api/window")).getCurrentWindow().close(),
};

export const api: SetupApi = inTauri() ? real : shim;
