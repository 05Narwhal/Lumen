import { invoke } from "@/lib/ipc";
import { createStore } from "@/lib/store";

export interface SelectionState {
  ids: ReadonlySet<number>;
  /** The item a shift-range extends from. */
  anchor: number | null;
}

export const selectionStore = createStore<SelectionState>({ ids: new Set(), anchor: null });

export function clearSelection() {
  selectionStore.set({ ids: new Set(), anchor: null });
}

/**
 * Clear the selection when an action was on exactly it — not when it was on one photo from
 * the viewer, which leaves the grid's selection as it was (Round 4A review).
 */
export function clearIfSelected(ids: readonly number[]) {
  const sel = selectionStore.get().ids;
  if (sel.size > 0 && sel.size === new Set(ids).size && ids.every((i) => sel.has(i))) clearSelection();
}

export function toggleSelection(id: number) {
  selectionStore.set((s) => {
    const ids = new Set(s.ids);
    if (ids.has(id)) ids.delete(id);
    else ids.add(id);
    return { ids, anchor: id };
  });
}

/** Select only `id`. */
export function selectOnly(id: number) {
  selectionStore.set({ ids: new Set([id]), anchor: id });
}

/** What the grid lays out with: enough for Rust to find its cached layout. */
export interface RangeLayout {
  width: number;
  rowHeight: number;
  gap: number;
  scope: unknown;
  groupBy: unknown;
}

/**
 * Extend from the anchor to `id` along `ordered` (grid order). When either end is outside
 * `ordered` — the grid holds only the rows it has loaded — and `layout` is given, the whole
 * run comes from Rust's cached layout instead (Round 5). Without an anchor, selects `id`.
 */
export function selectRange(id: number, ordered: readonly number[], layout?: RangeLayout) {
  const s = selectionStore.get();
  const a = s.anchor === null ? -1 : ordered.indexOf(s.anchor);
  const b = ordered.indexOf(id);
  if (s.anchor !== null && (a === -1 || b === -1) && layout) {
    const anchor = s.anchor;
    void invoke<number[] | null>("layout_range_ids", { ...layout, from: anchor, to: id })
      .then((ids) => {
        selectionStore.set((cur) => ({ ids: new Set([...cur.ids, ...(ids ?? [id])]), anchor: cur.anchor ?? anchor }));
      })
      .catch(() => selectionStore.set((cur) => ({ ids: new Set([...cur.ids, id]), anchor: cur.anchor ?? anchor })));
    return;
  }
  selectionStore.set((cur) => {
    if (a === -1 || b === -1) return { ids: new Set([...cur.ids, id]), anchor: cur.anchor ?? id };
    const [from, to] = a < b ? [a, b] : [b, a];
    const ids = new Set(cur.ids);
    for (let i = from; i <= to; i++) ids.add(ordered[i]);
    return { ids, anchor: cur.anchor };
  });
}

export function selectAll(ids: readonly number[]) {
  selectionStore.set({ ids: new Set(ids), anchor: ids[0] ?? null });
}

export function useSelection() {
  const s = selectionStore.use();
  return {
    ids: s.ids,
    anchor: s.anchor,
    count: s.ids.size,
    isSelected: (id: number) => s.ids.has(id),
    toggle: toggleSelection,
    selectOnly,
    range: selectRange,
    selectAll,
    clear: clearSelection,
  };
}
