import { invoke } from "@tauri-apps/api/core";
import type { PickKind } from "@/types/generated/PickKind";
import type { PickRequest } from "@/types/generated/PickRequest";

/**
 * Lumen's one file-dialog module (Round 24 D2). Every dialog goes through Rust's `dialog_pick`,
 * which asks a plugin's file picker when one is active and the system dialog otherwise; a
 * provider that fails falls back there, so these never need a fallback of their own.
 * Each resolves to `null` when the person cancels.
 */
export interface DialogFilter {
  name: string;
  /** Extensions without the dot. */
  extensions: string[];
}

export interface PickOptions {
  title?: string;
  /** The folder the dialog starts in. */
  defaultPath?: string;
  filters?: DialogFilter[];
}

export interface SaveOptions extends PickOptions {
  /** The suggested file name. */
  name?: string;
}

function request(kind: PickKind, o: SaveOptions): PickRequest {
  return {
    kind,
    title: o.title ?? null,
    start: o.defaultPath ?? null,
    name: kind === "save" ? (o.name ?? null) : null,
    filters: (o.filters ?? []).map((f) => ({ label: f.name, extensions: f.extensions })),
  };
}

async function pick(kind: PickKind, o: SaveOptions): Promise<string[] | null> {
  const paths = await invoke<string[] | null>("dialog_pick", { req: request(kind, o) });
  return paths && paths.length > 0 ? paths : null;
}

export const pickFile = async (o: PickOptions = {}): Promise<string | null> => (await pick("open-file", o))?.[0] ?? null;
export const pickFiles = async (o: PickOptions = {}): Promise<string[] | null> => pick("open-multiple", o);
export const pickFolder = async (o: PickOptions = {}): Promise<string | null> => (await pick("open-folder", o))?.[0] ?? null;
export const pickSave = async (o: SaveOptions = {}): Promise<string | null> => (await pick("save", o))?.[0] ?? null;

/** Shows a path in the file manager (a plugin's reveal provider, else the system's). */
export const revealPath = (path: string): Promise<void> => invoke<void>("reveal_path", { path });
