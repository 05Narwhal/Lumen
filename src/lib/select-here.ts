// Round 10's keys that act on where the user is: Select all takes every photo of the grid on
// screen (not only the rows loaded so far); Undo runs the newest toast's Undo.
import { dismissToast, toastStore } from "@/components/toast/store";
import { clearSelection, selectAll } from "@/hooks/use-selection";
import { invoke } from "./ipc";
import { gridOnScreen } from "./radar-view";

/** Every photo of the grid on screen; `false` when there is no grid here. */
export async function selectAllHere(): Promise<boolean> {
  const grid = gridOnScreen();
  if (!grid || grid.width <= 0) return false;
  const ids = await invoke<number[]>("layout_ids", grid);
  selectAll(ids);
  return true;
}

export function selectNone() {
  clearSelection();
}

/** Runs the newest Undo still on screen; `false` when there is none. */
export function undoLast(): boolean {
  const t = [...toastStore.get().toasts].reverse().find((t) => t.action?.label === "Undo");
  if (!t?.action) return false;
  t.action.run();
  dismissToast(t.id);
  return true;
}
