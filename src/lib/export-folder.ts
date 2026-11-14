import { open } from "@tauri-apps/plugin-dialog";

/**
 * Where exports go (Round 6): every export asks with the system's folder dialog, opening on
 * the folder chosen last time. Nothing is written into the library's own `exports` folder.
 */
const KEY = "lumen.exportFolder";

export function lastExportFolder(): string | null {
  try {
    return localStorage.getItem(KEY);
  } catch {
    return null;
  }
}

/** The folder picked, remembered for next time; `null` when the dialog was cancelled. */
export async function askExportFolder(title = "Export to…"): Promise<string | null> {
  const picked = await open({ directory: true, multiple: false, title, defaultPath: lastExportFolder() ?? undefined });
  if (typeof picked !== "string") return null;
  try {
    localStorage.setItem(KEY, picked);
  } catch {
    // Remembering is a convenience; the export goes ahead.
  }
  return picked;
}
