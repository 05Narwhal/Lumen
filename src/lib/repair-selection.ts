import type { RepairClass } from "@/types/generated/RepairClass";

/**
 * What the Repair list has ticked (Round 21 B4). *Select all* ticks every row of the current
 * filter, loaded or not, so it cannot be a list of ids: it is the filter plus the rows the person
 * then unticked. The JSON is what `start_repair_import` takes.
 */
export type RepairSelection = { all: true; status: RepairClass; except: number[] } | { ids: number[] };

export const NOTHING: RepairSelection = { ids: [] };

export function selectAll(status: RepairClass): RepairSelection {
  return { all: true, status, except: [] };
}

export function isTicked(sel: RepairSelection, id: number): boolean {
  return "all" in sel ? !sel.except.includes(id) : sel.ids.includes(id);
}

export function toggle(sel: RepairSelection, id: number): RepairSelection {
  if ("all" in sel) return { ...sel, except: sel.except.includes(id) ? sel.except.filter((e) => e !== id) : [...sel.except, id] };
  return { ids: sel.ids.includes(id) ? sel.ids.filter((e) => e !== id) : [...sel.ids, id] };
}

/** How many rows are ticked, given how many the current filter holds. */
export function tickedCount(sel: RepairSelection, inFilter: number): number {
  return "all" in sel ? Math.max(0, inFilter - sel.except.length) : sel.ids.length;
}

/** `In Trash` rows are put back from Trash, not re-imported; a copy damaged on the source too is no repair. */
export const canTick = (status: RepairClass) => status !== "inTrash" && status !== "sourceDamaged";
