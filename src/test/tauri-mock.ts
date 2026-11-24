import { vi } from "vitest";

type Handler = (args: Record<string, unknown> | undefined) => unknown;
const handlers = new Map<string, Handler>();

export const invokeMock = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
  const h = handlers.get(cmd);
  if (!h) throw { code: "unmocked", message: `No mock for command ${cmd}` };
  return h(args);
});

export function mockCommand(cmd: string, handler: Handler) {
  handlers.set(cmd, handler);
}

/** Every command a fresh, empty library answers, so tests only override what they care about. */
export function mockLibraryDefaults() {
  mockCommand("count_scope", () => 0);
  // Phase 8f: Facet off unless a test turns it on.
  mockCommand("ai_masks_list", () => ({ available: false, scanned: false, masks: [] }));
  mockCommand("system_date_order", () => "dmy");
  mockCommand("find_lightroom_catalogs", () => []);
  mockCommand("map_bounds", () => null);
  mockCommand("map_clusters", () => []);
  mockCommand("assets_in_bounds", () => []);
  mockCommand("search_places", () => []);
  mockCommand("place_for", () => null);
  mockCommand("set_rating", () => 1);
  mockCommand("copy_assets_to", (a) => (a?.ids as number[])?.length ?? 0);
  mockCommand("rebuild_search_index", () => 0);
  mockCommand("list_collections", () => []);
  // Round 22 lane C: no editor installed, none chosen.
  mockCommand("list_external_apps", () => []);
  mockCommand("get_default_apps", () => ({ photo: null, raw: null, video: null }));
  mockCommand("start_files_import", () => 1);
  // Everything fits unless a test says otherwise.
  mockCommand("plan_import", () => ({ count: 1, bytes: 1, freeBytes: null, fitsCount: 1, fitsBytes: 1, buffer: 20_000_000_000 }));
  mockCommand("list_devices", () => []);
  mockCommand("list_volumes", () => []);
  mockCommand("list_mac_disks", () => []);
  mockCommand("list_edit_versions", () => []);
  mockCommand("get_edit", () => null);
  mockCommand("save_edit", () => Date.now());
  mockCommand("get_develop_draft", () => null);
  mockCommand("save_develop_draft", () => Date.now());
  mockCommand("revert_edit", () => null);
  mockCommand("render_edit_preview", () => "1");
  mockCommand("export_assets", () => 1);
  mockCommand("list_memories", () => []);
  mockCommand("get_memory", (a) => ({ id: a?.id, kind: 1, title: "Memory", subtitle: null, startAt: 0, endAt: 0, coverAssetId: null, itemCount: 0, curatedCount: 0, hidden: false }));
  mockCommand("memory_curated_ids", () => []);
  mockCommand("set_memory_hidden", () => null);
  mockCommand("regenerate_memories", () => null);
  mockCommand("list_slideshows", () => []);
  mockCommand("create_slideshow", () => 1);
  mockCommand("create_slideshow_from_memory", () => 1);
  mockCommand("save_slideshow", () => null);
  mockCommand("delete_slideshow", () => null);
  mockCommand("export_slideshow", () => null);
  mockCommand("music_search", () => []);
  mockCommand("music_download", (a) => ({ ...(a?.track as object), credit: "credit", path: "C:/lib/music/1.mp3" }));
  mockCommand("list_library_music", () => []);
  mockCommand("delete_library_music", () => null);
  mockCommand("rebuild_slideshow", (a) => ({ id: a?.id, name: "Slideshow", doc: { version: 1, resolution: [1920, 1080], fps: 30, audio: null, clips: [] }, bpm: null }));
  mockCommand("sync_slideshow_to_music", (a) => ({ id: a?.id, name: "Slideshow", doc: { version: 1, resolution: [1920, 1080], fps: 30, audio: null, clips: [] }, bpm: 120 }));
  mockCommand("list_mac_folder", (a) => ({ path: (a?.path as string) ?? "/", folders: [], mediaHere: 0 }));
  mockCommand("start_mac_folder_import", () => 1);
  mockCommand("get_photos_library", () => ({ summary: { name: "Photos Library", items: 0, livePhotos: 0, notDownloaded: 0, referenced: 0, burstFrames: 0, trashed: 0 }, totalFiles: 0, newItems: 0, newest: [] }));
  mockCommand("start_photos_library_import", () => 1);
  mockCommand("get_connected_device", () => ({ kind: "volume", id: "", name: "", storage: null, totalItems: 0, newItems: 0, totalBytes: 0, newest: [], removable: true }));
  mockCommand("list_iphones", () => ({ usbmuxdAvailable: true, devices: [] }));
  mockCommand("start_iphone_import", () => 1);
  mockCommand("rename_device", () => null);
  mockCommand("set_device_hidden", () => null);
  mockCommand("start_volume_import", () => 1);
  mockCommand("forget_volume", () => 0);
  mockCommand("get_exiftool_status", () => ({ available: true, version: "13.59", path: "/exiftool", envVar: "LUMEN_EXIFTOOL_DIR" }));
  mockCommand("get_raw_formats_status", () => ({ converter: "not_found", converterPath: null, cacheBytes: 0, cacheFiles: 0, envVar: "LUMEN_DNG_CONVERTER" }));
  mockCommand("clear_dng_cache", () => 0);
  mockCommand("list_unreadable", () => ({ items: [], nextCursor: null }));
  mockCommand("trashed_unreadable_ids", () => []);
  mockCommand("unreadable_source_folders", () => []);
  mockCommand("get_check_status", () => ({ running: null, folder: null, library: null, lastFullCheckAt: null }));
  mockCommand("get_ffmpeg_status", () => ({ available: true, version: "ffmpeg version test", path: "/ffmpeg", envVar: "LUMEN_FFMPEG_DIR" }));
  mockCommand("get_playback", () => ({ mode: "native", proxyReady: false, videoCodec: "h264", audioCodec: "aac", container: "mov,mp4", durationMs: 2000, width: 320, height: 240 }));
  mockCommand("prepare_proxy", () => null);
  mockCommand("get_proxy_cache_size", () => 0);
  mockCommand("clear_proxy_cache", () => 0);
  mockCommand("list_libraries", () => ({ libraries: [], current: null }));
  mockCommand("get_current_library", () => null);
  mockCommand("get_missing_library", () => null);
  mockCommand("get_library_problem", () => null);
  mockCommand("update_status", () => ({ version: "26.1.0-alpha", state: { state: "off" }, whatsNew: null }));
  mockCommand("take_update_receipt", () => null);
  mockCommand("list_import_batches", () => []);
  mockCommand("get_library_stats", () => ({ items: 0, videos: 0, bytes: 0 }));
  mockCommand("get_layout", () => ({ totalHeight: 0, itemCount: 0, rowHeights: [], sections: [] }));
  mockCommand("get_layout_rows", () => []);
  mockCommand("find_layout_row", () => null);
  mockCommand("get_ml_status", () => ({ enabled: true, models: [], embedded: 0, photos: 0, faces: 0, people: 0 }));
  mockCommand("fetch_models", () => null);
  mockCommand("list_people", () => []);
  mockCommand("faces_for_asset", () => []);
  mockCommand("update_settings", (a) => a?.settings);
  mockCommand("prefetch_previews", () => null);
  mockCommand("find_apple_photos_libraries", () => []);
  mockCommand("prefetch_playback", () => null);
  mockCommand("set_video_support", () => null);
  mockCommand("set_favorite", () => 1);
  mockCommand("trash_assets", () => 1);
  mockCommand("restore_assets", () => 1);
  mockCommand("delete_now", () => 1);
  mockCommand("empty_trash", () => 0);
  mockCommand("get_favorite_ids", () => []);
  mockCommand("list_albums", () => []);
  mockCommand("list_album_collections", () => []);
  mockCommand("list_pets", () => []);
  mockCommand("list_removed_pets", () => []);
  mockCommand("asset_pets", () => []);
  mockCommand("get_backup", () => ({ config: { destination: null, schedule: "off", lastSuccessAt: null, last: null }, target: null, available: false, running: false }));
  mockCommand("create_album", (a) => ({ id: 1, name: a?.name, kind: 0, parentId: null, coverAssetId: null, itemCount: 0, createdAt: 1, updatedAt: 1, query: null }));
  mockCommand("create_smart_album", (a) => ({ id: 2, name: a?.name, kind: 1, parentId: null, coverAssetId: null, itemCount: 0, createdAt: 1, updatedAt: 1, query: a?.query ?? null }));
  mockCommand("update_smart_album", () => null);
  mockCommand("add_to_album", () => 1);
  mockCommand("remove_from_album", () => 1);
  mockCommand("hold_close", () => null);
  // Round 21 lane C: nothing remembered for Develop's filmstrip.
  mockCommand("get_develop_source", () => ({ source: null, last: null }));
  mockCommand("set_develop_source", () => null);
  mockCommand("list_draft_ids", () => []);
}

// Tauri event listeners registered by the app; tests fire them with `emitMock`.
type Listener = (event: { payload: unknown }) => void;
const listeners = new Map<string, Set<Listener>>();

export function emitMock(name: string, payload: unknown) {
  listeners.get(name)?.forEach((l) => l({ payload }));
}

export function resetInvokeMock() {
  handlers.clear();
  listeners.clear();
  invokeMock.mockClear();
}

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock, convertFileSrc: (p: string) => p }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, cb: Listener) => {
    if (!listeners.has(name)) listeners.set(name, new Set());
    listeners.get(name)!.add(cb);
    return () => listeners.get(name)?.delete(cb);
  }),
}));
/** The one window every `getCurrentWindow()` returns, so tests can see what was asked of it. */
export const windowMock = {
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  close: vi.fn(),
  isMaximized: async () => false,
  startDragging: vi.fn(),
  startResizeDragging: vi.fn(),
  setBackgroundColor: vi.fn(async () => {}),
  onResized: async () => () => {},
};
// Read when a test calls it, long after this module has set it.
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => windowMock }));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(async () => null) }));
// Round 24 D2: every dialog goes through `@/lib/dialogs`; here it lands on the same `open` mock the
// tests already steer, so a test sets one answer and reads one call shape. dialogs.test.ts uses the real module.
vi.mock("@/lib/dialogs", async () => {
  const { open } = await import("@tauri-apps/plugin-dialog");
  type O = Record<string, unknown>;
  return {
    pickFile: (o: O = {}) => open({ multiple: false, directory: false, ...o }),
    pickFiles: (o: O = {}) => open({ multiple: true, ...o }),
    pickFolder: (o: O = {}) => open({ directory: true, multiple: false, ...o }),
    pickSave: (o: O = {}) => (open as (o: O) => Promise<string | null>)({ save: true, ...o }),
    revealPath: vi.fn(async () => {}),
  };
});
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: vi.fn(async () => {}), openPath: vi.fn(async () => {}), openUrl: vi.fn(async () => {}) }));
