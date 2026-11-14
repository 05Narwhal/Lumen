// Browser stand-in for the Rust side, for `yarn setup:dev` and the headless-Edge screenshots.
// `?screen=welcome|options|progress|done|installed-older|installed-same|installed-newer|remove`
// `&scheme=light|dark` `&problem=needs_admin|not_empty|no_space|is_file` (Options)
// `&lumen=running` `&progress=0.42` `&models=later` `&safety=1`
// `?screen=delete-libraries&libs=1|3` (one or several libraries) `&bin=small` (they don't fit the Bin)
import type { BinCheck, FolderCheck, InstallState, LibraryDetail, SetupProgress } from "@/types";
import type { SetupApi } from "@/lib/api";

const GB = 1024 ** 3;
const MB = 1024 ** 2;
const q = (): URLSearchParams => new URLSearchParams(typeof location === "undefined" ? "" : location.search);

export type PreviewScreen = "welcome" | "options" | "progress" | "done" | "installed" | "remove" | "delete-libraries";

/** The fake libraries screen 6b lists: `libs=1` one, otherwise three (one refused). */
export function previewLibraries(): LibraryDetail[] {
  const all: LibraryDetail[] = [
    { name: "Lumen Library", path: "C:\\Users\\Sample\\Pictures\\Lumen Library", photos: 5012, bytes: Math.round(38.4 * GB), refused: null },
    { name: "Family scans", path: "D:\\Photos\\Family scans", photos: 812, bytes: Math.round(6.1 * GB), refused: null },
    { name: "Old phone", path: "C:\\Users\\Sample\\AppData\\Roaming\\JL-Studios\\Lumen\\Old phone", photos: 0, bytes: 0, refused: "holds-lumen" },
  ];
  return q().get("libs") === "1" ? all.slice(0, 1) : all;
}

function previewBin(paths: string[]): BinCheck {
  const bytes = previewLibraries().filter((l) => paths.includes(l.path)).reduce((n, l) => n + l.bytes, 0);
  const room = q().get("bin") === "small" ? 4 * GB : 120 * GB;
  return { fits: bytes <= room, bytes, room };
}

/** The screen a browser preview asks for with `?screen=`; null when it does not ask. */
export function previewScreen(): { screen: PreviewScreen; fraction: number } | null {
  if (!q().has("screen")) return null;
  const s = q().get("screen") ?? "welcome";
  const fraction = Number(q().get("progress") ?? 0.42);
  if (s.startsWith("installed")) return { screen: "installed", fraction };
  return { screen: (["welcome", "options", "progress", "done", "remove", "delete-libraries"].includes(s) ? s : "welcome") as PreviewScreen, fraction };
}

export function previewState(): InstallState {
  const p = q();
  const s = p.get("screen") ?? "welcome";
  // `long=1`: worst-case paths (deep, no spaces), to check nothing runs off the window.
  const deep = p.get("long") === "1" ? "\\Temp\\claude\\d340f0a98b1451a1\\5a26ffa8-fdea-4d1d-ad2f-dd13a3dfd4cc\\scratchpad\\p8" : "";
  const where = `C:\\Users\\Sample\\AppData\\Local${deep}\\Lumen`;
  const installed: InstallState["installed"] = s === "installed-older" ? { kind: "older", version: "26.1.0-alpha", location: where }
    : s === "installed-same" ? { kind: "same", version: "26.1.2-alpha", location: where }
    : s === "installed-newer" ? { kind: "newer", version: "26.2.0", location: where }
    : s === "remove" ? { kind: "same", version: "26.1.2-alpha", location: where }
    : { kind: "none" };
  return {
    installed,
    ours: "26.1.2-alpha",
    lumenRunning: p.get("lumen") === "running" || s.startsWith("installed"),
    defaultFolder: where,
    sizeOnDisk: 288 * MB,
    modelsBytes: 500 * MB,
    needBytes: 288 * MB,
    libraries: [`C:\\Users\\Sample\\Pictures${deep}\\Lumen Library`, ...(deep ? [`C:\\Users\\Sample\\AppData\\Roaming${deep}\\appdata\\logs\\Registered`] : [])],
    appDataBytes: Math.round(1.9 * GB),
    safetyCopies: p.get("safety") === "1" ? `C:\\Users\\Sample\\AppData\\Roaming${deep}\\JL-Studios\\Lumen\\backups\\pre-migration` : null,
    removeOnly: false,
  };
}

const problem = (): FolderCheck["kind"] => (q().get("problem") as FolderCheck["kind"] | null) ?? "ok";

const listeners = new Set<(p: SetupProgress) => void>();
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export const shim: SetupApi = {
  async getInstallState() { return previewState(); },
  async checkFolder(path) {
    const kind = path.includes("Program Files") ? "needs_admin" : problem();
    return { kind, free: 214 * GB } as FolderCheck;
  },
  async chooseFolder() { return null; },
  async install(opts) {
    const emit = (p: SetupProgress) => listeners.forEach((l) => l(p));
    for (let i = 1; i <= 10; i++) { emit({ task: "lumen", done: i, total: 10 }); await sleep(120); }
    if (!opts.downloadModels) return { models: "later" };
    for (let i = 1; i <= 10; i++) { emit({ task: "models", done: i * 50 * MB, total: 500 * MB }); await sleep(150); }
    return { models: "complete" };
  },
  async skipModels() {},
  async openLumen() {},
  async uninstall(_removeData, choice) {
    await sleep(600);
    return (choice?.paths ?? []).map((path) => ({ path, folderRemoved: true, problem: null }));
  },
  async libraryDetails() { await sleep(300); return previewLibraries(); },
  async checkRecycleBin(paths) { await sleep(200); return previewBin(paths); },
  async cancelWait() {},
  async onProgress(handler) { listeners.add(handler); return () => { listeners.delete(handler); }; },
  async onLumen() { return () => {}; },
  async minimize() {},
  async close() {},
};
