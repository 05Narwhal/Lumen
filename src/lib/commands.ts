import { petBoxesRequest, requestFaceTagging } from "@/lib/face-tagging";
import type { QueryClient } from "@tanstack/react-query";
import type { NavigateFunction } from "react-router";
import type { SettingsPatch } from "@/hooks/use-settings";
import type { TileSize } from "@/types/generated/TileSize";
import { THEMES } from "@/types/theme";
import { requestAuto, requestRetouch, retouchReady } from "@/components/develop-panels";
import { requestAiMask } from "@/components/develop-masks/requests";
import { aiMasksReady } from "@/lib/ai-masks";
import type { ExperimentalTask } from "@/types/generated/ExperimentalTask";
import { queryKeys } from "@/lib/query-keys";
import type { Settings } from "@/types/generated/Settings";
import { openExportSheet } from "@/components/export-sheet";
import { selectionStore } from "@/hooks/use-selection";
import { clearDevelop, openDevelop, restoreDevelop, toggleDevelopStrip } from "@/lib/develop-source";
import { pickPluginToInstall } from "@/components/plugin-install";
import { toast } from "@/components/toast";
import { importFromLightroom, pickLightroomPresets } from "@/lib/lightroom-presets";
import { backUpNow, duplicateAlbum, duplicateSlideshow, editAlbumCover, editCollectionCover, ejectVolume, exportAlbum, moveAlbumToCollection, newCollection, openCollection, openDevice, openId, setMemoryHidden, shuffleAlbumCover, stopBackup } from "@/lib/library-actions";
import { reloadWindow, restartLumen } from "@/lib/health";
import { pickAlbumUrl } from "@/lib/pick-mode";
import { defaultLabel, editInDefault, openWithPicker } from "@/lib/external-apps";
import { rotatePhotos } from "@/lib/rotate";
import { moveToHidden } from "@/lib/vault";
import { selectAllHere, selectNone, undoLast } from "@/lib/select-here";
import { invoke } from "@/lib/ipc";
import { allText, copyText, HAS_TEXT_TAG, readTextOn, textOverlayRequest } from "@/lib/photo-text";
import { writeTags } from "@/lib/search-tags";
import type { ShownTextLine } from "@/types/generated/ShownTextLine";

export type SettingsUpdater = (patch: SettingsPatch) => void;

export interface AppCommand {
  id: string;
  label: string;
  group: "Navigate" | "View" | "Library";
  /** `Mod` is Cmd on macOS, Ctrl elsewhere. Single characters are bare keys. */
  shortcut?: string;
  run: (ctx: { navigate: NavigateFunction; update: SettingsUpdater; qc: QueryClient }) => void;
  /** Whether the palette offers it now (an Experimental switch); always when absent. */
  available?: (qc: QueryClient) => boolean;
  /** The label to show now, when it depends on something loaded (*Edit in Photoshop*). */
  labelFor?: (qc: QueryClient) => string | null;
}

/** Offered only while text in photos is read (Round 13-3). */
const textAvailable = (qc: QueryClient) => readTextOn(qc.getQueryData<Settings>(queryKeys.settings));

const nav = (id: string, label: string, to: string, shortcut?: string): AppCommand => ({
  id,
  label,
  group: "Navigate",
  shortcut,
  run: ({ navigate }) => navigate(to),
});

const SIZES: TileSize[] = ["s", "m", "l"];
export const stepTileSize = (current: TileSize, delta: 1 | -1 | 0): TileSize =>
  delta === 0 ? "m" : SIZES[Math.min(SIZES.length - 1, Math.max(0, SIZES.indexOf(current) + delta))];

const tileCmd = (id: string, label: string, delta: 1 | -1 | 0, shortcut: string): AppCommand => ({
  id,
  label,
  group: "View",
  shortcut,
  run: ({ update }) =>
    update((s) => ({ ...s, appearance: { ...s.appearance, tileSize: stepTileSize(s.appearance.tileSize, delta) } })),
});

/** Every action reachable from the command palette. Hotkeys route through the same list. */
export const appCommands: AppCommand[] = [
  nav("go-home", "Go to Home", "/", "1"),
  nav("go-library", "Go to Library", "/library", "2"),
  nav("go-albums", "Go to Albums", "/albums", "3"),
  nav("go-map", "Go to Map", "/map", "4"),
  nav("go-memories", "Go to Memories", "/memories", "5"),
  nav("go-slideshows", "Go to Slideshows", "/memories?tab=slideshows", "6"),
  nav("go-people", "Go to People & Pets", "/people", "7"),
  nav("go-develop", "Go to Develop", "/develop", "8"),
  nav("go-pets", "Go to Pets", "/people?show=pets", "9"),
  nav("go-import", "Import photos", "/import", "Mod+I"),
  nav("repair-import", "Repair from a disk or device", "/import?mode=repair"),
  nav("go-trash", "Go to Trash", "/trash"),
  nav("go-duplicates", "Find duplicates", "/duplicates"),
  nav("go-search", "Search photos", "/search"),
  nav("go-settings", "Open Settings", "/settings", "Mod+,"),
  nav("change-date-order", "Change date order", "/settings?section=search"),
  {
    // Round 17: checks, then shows the answer where the person can act on it.
    id: "check-updates",
    label: "Check for updates",
    group: "Library",
    run: ({ navigate }) => {
      void invoke("check_for_updates").catch(() => undefined);
      navigate("/settings?section=updates");
    },
  },
  nav("raw-formats", "RAW formats and Adobe DNG Converter", "/settings?section=raw-formats"),
  { id: "import-lightroom-presets", label: "Import Lightroom presets…", group: "Library", run: ({ qc }) => void pickLightroomPresets("files", qc) },
  { id: "import-presets-from-lightroom", label: "Import presets from Lightroom", group: "Library", run: ({ qc }) => void importFromLightroom(qc) },
  { id: "install-plugin", label: "Install plugin from file…", group: "Library", run: () => void pickPluginToInstall() },
  // Round 10.
  { id: "go-back", label: "Go back", group: "Navigate", shortcut: "Alt+←", run: ({ navigate }) => navigate(-1) },
  { id: "go-forward", label: "Go forward", group: "Navigate", shortcut: "Alt+→", run: ({ navigate }) => navigate(1) },
  { id: "focus-search", label: "Search this library", group: "Navigate", shortcut: "Mod+F", run: () => document.getElementById("global-search")?.focus() },
  tileCmd("tiles-larger", "Larger tiles", 1, "Mod++"),
  tileCmd("tiles-smaller", "Smaller tiles", -1, "Mod+-"),
  tileCmd("tiles-reset", "Reset tile size", 0, "Mod+0"),
  {
    id: "toggle-sidebar",
    label: "Toggle sidebar",
    group: "View",
    shortcut: "Mod+B",
    run: ({ update }) =>
      update((s) => ({ ...s, appearance: { ...s.appearance, sidebarCollapsed: !s.appearance.sidebarCollapsed } })),
  },
  {
    id: "toggle-scheme",
    label: "Toggle light / dark",
    group: "View",
    run: ({ update }) =>
      update((s) => ({
        ...s,
        appearance: { ...s.appearance, scheme: s.appearance.scheme === "dark" ? "light" : "dark" },
      })),
  },
  ...THEMES.map<AppCommand>((t) => ({
    id: `theme-${t.id}`,
    label: `Use ${t.name} theme`,
    group: "View",
    run: ({ update }) => update((s) => ({ ...s, appearance: { ...s.appearance, theme: t.id } })),
  })),
  {
    // The selection when there is one, else the photo open in the viewer.
    id: "export-selection",
    label: "Export selection…",
    group: "Library",
    shortcut: "Mod+E",
    run: () => {
      const selected = [...selectionStore.get().ids];
      const asset = Number(new URLSearchParams(window.location.search).get("asset"));
      const ids = selected.length > 0 ? selected : asset ? [asset] : [];
      if (ids.length > 0) openExportSheet(ids);
    },
  },
  {
    // Acts on the photo open in the viewer; the viewer's own `e` key does the same.
    id: "edit-photo",
    label: "Edit photo",
    group: "Library",
    shortcut: "e",
    run: ({ navigate }) => {
      const p = new URLSearchParams(window.location.search);
      if (!p.get("asset")) return;
      p.set("edit", "1");
      navigate({ search: `?${p.toString()}` });
    },
  },
  {
    // The photo open in the viewer: its faces' boxes and the People list (Round 4I).
    id: "tag-people",
    label: "Tag people in this photo",
    group: "Library",
    shortcut: "p",
    run: () => {
      const asset = Number(new URLSearchParams(window.location.search).get("asset"));
      if (asset) requestFaceTagging(asset);
    },
  },
  {
    // The photo open in the viewer: its pets' boxes, on or off (Round 8B).
    id: "show-pets",
    label: "Show pets in this photo",
    group: "Library",
    shortcut: "Shift+P",
    run: () => {
      const asset = Number(new URLSearchParams(window.location.search).get("asset"));
      if (asset) petBoxesRequest.set(asset);
    },
  },
  // Round 13-3: the text read in the photo open in the viewer, while reading text is on.
  {
    id: "show-text",
    label: "Show text in photo",
    group: "Library",
    shortcut: "t",
    run: () => {
      const asset = Number(new URLSearchParams(window.location.search).get("asset"));
      if (asset) textOverlayRequest.set(asset);
    },
    available: textAvailable,
  },
  {
    id: "copy-text",
    label: "Copy text from photo",
    group: "Library",
    run: () => {
      const asset = Number(new URLSearchParams(window.location.search).get("asset"));
      if (!asset) return;
      void invoke<ShownTextLine[]>("asset_text", { id: asset }).then(
        async (lines) =>
          toast({ message: !lines.length ? "No text found in this photo." : (await copyText(allText(lines))) ? "Copied the text" : "Could not copy the text." }),
        () => toast({ message: "Could not read the text." }),
      );
    },
    available: textAvailable,
  },
  {
    id: "filter-has-text",
    label: "Filter: has text",
    group: "Library",
    run: ({ navigate }) => {
      const p = new URLSearchParams();
      writeTags(p, [HAS_TEXT_TAG]);
      navigate(`/library?${p.toString()}`);
    },
    available: textAvailable,
  },
  {
    // The selection when there is one, else the photo open in the viewer.
    id: "develop-selection",
    label: "Develop selection",
    group: "Library",
    shortcut: "Mod+Shift+D",
    run: ({ navigate }) => {
      const selected = [...selectionStore.get().ids];
      const asset = Number(new URLSearchParams(window.location.search).get("asset"));
      if (selected.length > 0) openDevelop(navigate, { only: selected, label: "Selection" }, selected[0]);
      else if (asset) openDevelop(navigate, {}, asset);
      else openDevelop(navigate);
    },
  },
  {
    id: "develop-filmstrip",
    label: "Show or hide Develop's filmstrip",
    group: "View",
    shortcut: "F6",
    run: () => toggleDevelopStrip(),
  },
  {
    // Develop runs it (`autoRequests`); elsewhere nothing is open to adjust.
    id: "develop-auto",
    label: "Auto tone",
    group: "Library",
    shortcut: "Mod+U",
    run: () => requestAuto(),
  },
  {
    // Round 12, Experimental: only while Settings → Experimental → Retouch is on, and (Round 14)
    // a Lumen Prism model is installed.
    id: "develop-retouch",
    label: "Retouch photo",
    group: "Library",
    shortcut: "Mod+Shift+U",
    run: () => requestRetouch(),
    available: (qc) => retouchReady(qc.getQueryData<Settings>(queryKeys.settings), qc.getQueryData<ExperimentalTask[]>(queryKeys.experimentalCatalog)),
  },
  // Phase 8f: the open Masks panel adds the mask (`aiMaskRequests`), finding it first if Facet
  // hasn't looked. Only while Settings → Experimental → AI masks is on and Facet is installed.
  ...(["sky", "subject", "background"] as const).map(
    (kind): AppCommand => ({
      id: `develop-ai-mask-${kind}`,
      label: `New ${kind} mask`,
      group: "Library",
      run: () => requestAiMask(kind),
      available: (qc) => aiMasksReady(qc.getQueryData<Settings>(queryKeys.settings), qc.getQueryData<ExperimentalTask[]>(queryKeys.experimentalCatalog)),
    }),
  ),
  {
    // Round 16: only off from here; turning AI assistants on stays in Settings, beside the
    // sentence saying what they see.
    id: "ai-assistants-off",
    label: "Turn AI assistants off",
    group: "Library",
    run: ({ update }) => update((s) => ({ ...s, aiAssistants: { ...s.aiAssistants, enabled: false } })),
    available: (qc) => !!qc.getQueryData<Settings>(queryKeys.settings)?.aiAssistants?.enabled,
  },
  {
    // Round 17: only off from here; turning the Discord status on stays in Settings, beside the
    // sentence saying what it shows.
    id: "discord-off",
    label: "Turn Discord status off",
    group: "Library",
    run: ({ update }) => update((s) => ({ ...s, discord: { ...s.discord, enabled: false } })),
    available: (qc) => !!qc.getQueryData<Settings>(queryKeys.settings)?.discord?.enabled,
  },
  {
    // Develop saves its open photo as it unmounts; `/develop` reopens it after Undo.
    id: "develop-clear-filmstrip",
    label: "Clear Develop's filmstrip",
    group: "Library",
    run: ({ navigate }) => {
      const prev = clearDevelop();
      navigate("/develop");
      toast({ message: "Filmstrip cleared", undo: () => restoreDevelop(prev) });
    },
  },
  nav("go-hidden", "Open Hidden", "/hidden"),
  // Round 10: what the grid on screen holds, and the last thing done.
  { id: "select-all", label: "Select all photos here", group: "Library", shortcut: "Mod+A", run: () => void selectAllHere() },
  { id: "select-none", label: "Select none", group: "Library", shortcut: "Mod+D", run: () => selectNone() },
  {
    id: "undo-last",
    label: "Undo the last action",
    group: "Library",
    shortcut: "Mod+Z",
    run: () => {
      if (!undoLast()) toast({ message: "Nothing to undo" });
    },
  },
  {
    id: "hide-selection",
    label: "Move selection to Hidden",
    group: "Library",
    run: ({ qc }) => void moveToHidden(qc, [...selectionStore.get().ids]),
  },
  {
    id: "rotate-left",
    label: "Rotate selection left",
    group: "Library",
    shortcut: "Mod+[",
    run: ({ qc }) => void rotatePhotos(qc, [...selectionStore.get().ids], -1),
  },
  {
    id: "rotate-right",
    label: "Rotate selection right",
    group: "Library",
    shortcut: "Mod+]",
    run: ({ qc }) => void rotatePhotos(qc, [...selectionStore.get().ids], 1),
  },
  // Round 4G: the context menus' actions on the album, memory or slideshow open now.
  {
    // Round 19 §5: the Library in pick mode, adding to the album open now (not a smart one).
    id: "add-photos-to-album",
    label: "Add photos to this album",
    group: "Library",
    run: ({ navigate, qc }) => {
      const id = openId("albums");
      if (id === null || qc.getQueryData<{ kind: number }>(queryKeys.album(id))?.kind === 1) return;
      navigate(pickAlbumUrl(id));
    },
  },
  {
    id: "duplicate-album",
    label: "Duplicate this album",
    group: "Library",
    run: ({ qc }) => {
      const id = openId("albums");
      if (id !== null) void duplicateAlbum(qc, id);
    },
  },
  // Round 5A: backup.
  {
    id: "back-up-now",
    label: "Back up now",
    group: "Library",
    run: ({ qc }) => void backUpNow(qc),
  },
  {
    id: "stop-backup",
    label: "Stop the backup",
    group: "Library",
    run: ({ qc }) => void stopBackup(qc),
  },
  // Round 5A: collections.
  {
    id: "new-collection",
    label: "New collection…",
    group: "Library",
    run: ({ qc }) => void newCollection(qc),
  },
  {
    id: "move-album-to-collection",
    label: "Move this album to collection…",
    group: "Library",
    run: () => {
      const id = openId("albums");
      if (id !== null) void moveAlbumToCollection(id);
    },
  },
  // Julian's 2026-11-08 list: icon covers.
  {
    id: "album-cover",
    label: "Change this album's cover…",
    group: "Library",
    run: () => {
      const id = openId("albums");
      if (id !== null) void editAlbumCover(id);
    },
  },
  {
    id: "album-cover-shuffle",
    label: "Shuffle this album's cover",
    group: "Library",
    run: ({ qc }) => {
      const id = openId("albums");
      if (id !== null) void shuffleAlbumCover(qc, id);
    },
  },
  {
    id: "collection-cover",
    label: "Change this collection's cover…",
    group: "Library",
    run: () => {
      const id = openCollection();
      if (id !== null) void editCollectionCover(id);
    },
  },
  {
    id: "export-album",
    label: "Export this album…",
    group: "Library",
    run: () => {
      const id = openId("albums");
      if (id !== null) void exportAlbum(id);
    },
  },
  {
    id: "hide-memory",
    label: "Hide this memory",
    group: "Library",
    run: ({ qc }) => {
      const id = openId("memories");
      if (id !== null) void setMemoryHidden(qc, { id }, true);
    },
  },
  {
    id: "duplicate-slideshow",
    label: "Duplicate this slideshow",
    group: "Library",
    run: ({ qc }) => {
      const id = openId("slideshows");
      if (id !== null) void duplicateSlideshow(qc, id);
    },
  },
  {
    id: "browse-device",
    label: "Browse this device…",
    group: "Library",
    run: ({ navigate }) => {
      const d = openDevice();
      if (d) navigate(`/connected/${d.kind}/${encodeURIComponent(d.id)}?browse=1`);
    },
  },
  {
    id: "eject-device",
    label: "Eject this card",
    group: "Library",
    run: () => {
      const d = openDevice();
      if (d?.kind === "volume") void ejectVolume(d.id);
    },
  },
  // Round 22 lane C: hand the photo open now, or the selection, to an editor (`Mod+E` is Export).
  { id: "edit-in-default", label: "Edit in the default app", group: "Library", shortcut: "Mod+Shift+E", run: ({ qc }) => void editInDefault(qc), labelFor: defaultLabel },
  { id: "open-with", label: "Open with…", group: "Library", run: ({ qc }) => void openWithPicker(qc) },
  // Round 15: the page alone, or the whole app (recovered in place when it can be).
  { id: "reload-window", label: "Reload window", group: "View", run: () => reloadWindow() },
  { id: "restart-lumen", label: "Restart Lumen", group: "View", run: () => void restartLumen() },
  ...(import.meta.env.DEV ? [nav("dev-transport", "Transport check (dev)", "/dev/transport")] : []),
];
