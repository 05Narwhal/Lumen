// The photos the open viewer pages through, when a screen opened it on a set of its own —
// the Map opens it on the photos in view, or on one group. Previous, next and the filmstrip
// then stay inside that set; a photo outside it (or no set) falls back to the timeline.
import { useSyncExternalStore } from "react";

let ids: number[] = [];
const listeners = new Set<() => void>();
const changed = () => {
  for (const l of listeners) l();
};

/** The set the viewer pages through; empty for the timeline. */
export const viewerSequenceIds = (): number[] => [...ids];

export function setViewerSequence(list: number[]) {
  ids = [...new Set(list)];
  changed();
}

export function clearViewerSequence() {
  if (ids.length === 0) return;
  ids = [];
  changed();
}

/** A trashed photo leaves the set, so previous and next close over the gap. */
export function dropFromViewerSequence(id: number) {
  if (!ids.includes(id)) return;
  ids = ids.filter((i) => i !== id);
  changed();
}

export interface SequenceNeighbors {
  prev: number | null;
  next: number | null;
  index: number;
  total: number;
}

export function sequenceNeighbors(id: number | null): SequenceNeighbors | null {
  const i = id == null ? -1 : ids.indexOf(id);
  if (i < 0) return null;
  return { prev: ids[i - 1] ?? null, next: ids[i + 1] ?? null, index: i, total: ids.length };
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};
let snapshot = ids;
const getSnapshot = () => {
  if (snapshot !== ids) snapshot = ids;
  return snapshot;
};

/** Re-renders when the set changes; returns this photo's place in it, or null. */
export function useSequenceNeighbors(id: number | null): SequenceNeighbors | null {
  useSyncExternalStore(subscribe, getSnapshot);
  return sequenceNeighbors(id);
}
