// Which assets have an edit, and at what version. Derivatives are served `immutable`, so
// an edited photo's URLs must change with every save — and a revert must drop the param
// so the original's still-cached thumbnail is used again. One store, read by `mediaUrl`,
// so no call site has to know about edits.
import { useSyncExternalStore } from "react";
import type { EditVersion } from "@/types/generated/EditVersion";

let versions = new Map<number, number>();
const listeners = new Set<() => void>();

export function setEditVersions(list: EditVersion[]) {
  versions = new Map(list.map((e) => [e.assetId, e.version]));
  for (const l of listeners) l();
}

export const editVersion = (id: number): number | undefined => versions.get(id);

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
};

/** Re-render when this asset's version changes (tiles, the viewer). */
export const useEditVersion = (id: number) => useSyncExternalStore(subscribe, () => versions.get(id));
