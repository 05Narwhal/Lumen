import { pickFiles, pickFolder } from "@/lib/dialogs";
import type { QueryClient } from "@tanstack/react-query";
import { openPresetNotes } from "@/components/preset-import";
import { toast } from "@/components/toast";
import { invoke, isAppError } from "@/lib/ipc";
import type { PresetImport } from "@/types/generated/PresetImport";

/** The query the presets list reads, refreshed after an import. */
export const presetsKey = ["develop-presets"] as const;
export const cameraRawFolderKey = ["camera-raw-preset-folder"] as const;

export const cameraRawFolderQuery = { queryKey: cameraRawFolderKey, queryFn: () => invoke<string | null>("camera_raw_preset_folder"), staleTime: 60_000 };

const plural = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString()} ${n === 1 ? one : many}`;

/** "Imported 34 Lightroom presets · 3 settings not converted". */
export function importMessage(r: PresetImport): string {
  const notes = r.notConverted.reduce((n, p) => n + p.settings.length, 0);
  const parts = [r.imported > 0 ? `Imported ${plural(r.imported, "Lightroom preset")}` : "No new Lightroom presets"];
  if (r.skippedSameName > 0) parts.push(`${r.skippedSameName.toLocaleString()} already here`);
  if (notes > 0) parts.push(`${plural(notes, "setting")} not converted`);
  if (r.failed.length > 0) parts.push(`${plural(r.failed.length, "file")} not read`);
  return parts.join(" · ");
}

/** Imports the presets in `paths` (files or folders, read only) and says how it went. */
export async function importLightroomPresets(paths: string[], qc: QueryClient): Promise<void> {
  if (paths.length === 0) return;
  try {
    const r = await invoke<PresetImport>("import_lightroom_presets", { paths });
    await qc.invalidateQueries({ queryKey: presetsKey });
    const details = r.notConverted.length > 0 || r.failed.length > 0;
    toast({ message: importMessage(r), action: details ? { label: "Details", run: () => openPresetNotes(r) } : undefined });
  } catch (e) {
    toast({ message: isAppError(e) ? e.message : "The Lightroom presets could not be imported." });
  }
}

/** Picks preset files, or a folder of them, and imports them. */
export async function pickLightroomPresets(kind: "files" | "folder", qc: QueryClient): Promise<void> {
  const picked =
    kind === "folder"
      ? await pickFolder({ title: "Choose a folder of Lightroom presets" })
      : await pickFiles({ title: "Choose Lightroom presets", filters: [{ name: "Lightroom presets", extensions: ["xmp", "lrtemplate"] }] });
  const paths = picked === null ? [] : Array.isArray(picked) ? picked : [picked];
  await importLightroomPresets(paths, qc);
}

/** Camera Raw's own preset folder, where Lightroom keeps them, when it exists. */
export async function importFromLightroom(qc: QueryClient): Promise<void> {
  const folder = await qc.fetchQuery(cameraRawFolderQuery);
  if (!folder) {
    toast({ message: "Lightroom's presets folder was not found on this computer." });
    return;
  }
  await importLightroomPresets([folder], qc);
}
