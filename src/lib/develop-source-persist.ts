import { listen } from "@tauri-apps/api/event";
import { developSourceStore, lastDevelopPhoto, type DevelopSource } from "@/lib/develop-source";
import { FLUSH_REQUESTED } from "@/lib/events";
import { invoke } from "@/lib/ipc";
import type { DevelopSourceFile } from "@/types/generated/DevelopSourceFile";

/** How long a change waits for the next one before it is written. */
export const REMEMBER_DEBOUNCE_MS = 500;

/**
 * Round 21 §13: put back the Develop filmstrip the library last had (the strip and the photo
 * last open) before the Develop route first renders. Nothing stored, or the read failing, leaves
 * Develop empty — remembering is a convenience, never a reason to stop starting.
 */
export async function hydrateDevelopSource(): Promise<void> {
  try {
    const file = await invoke<DevelopSourceFile>("get_develop_source");
    if (file.source) developSourceStore.set(file.source as DevelopSource);
    if (file.last !== null) lastDevelopPhoto.set(file.last);
  } catch {
    // Develop starts empty.
  }
}

/**
 * Write the strip and the last photo back whenever either changes: debounced, and at once when
 * Rust asks for a flush (the window closing, *Restart Lumen*). Call it after `hydrateDevelopSource`,
 * so the empty defaults never overwrite what was stored. Returns the function that stops it.
 */
export function rememberDevelopSource(): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let dirty = false;
  const write = () => {
    clearTimeout(timer);
    timer = undefined;
    if (!dirty) return;
    dirty = false;
    void invoke("set_develop_source", { source: developSourceStore.get(), last: lastDevelopPhoto.get() }).catch(() => {});
  };
  const changed = () => {
    dirty = true;
    clearTimeout(timer);
    timer = setTimeout(write, REMEMBER_DEBOUNCE_MS);
  };
  const offs = [developSourceStore.subscribe(changed), lastDevelopPhoto.subscribe(changed)];
  const flush = listen(FLUSH_REQUESTED, write);
  return () => {
    offs.forEach((off) => off());
    void flush.then((f) => f());
    write();
  };
}
