// The grid that opened the viewer: its layout key, so previous and next walk exactly what
// that grid shows, in its order — an album in its manual order, a person, a search, a
// memory — instead of the whole timeline (Round 4A). Set by `PhotoGrid` when it opens a
// photo; cleared when the viewer closes. A set of the screen's own (`viewer-sequence`, the
// Map) still wins over it.
import { createStore } from "@/lib/store";
import type { LayoutParams } from "@/hooks/use-layout";

export type ViewerScope = Required<LayoutParams>;

export const viewerScopeStore = createStore<ViewerScope | null>(null);

export function setViewerScope(scope: ViewerScope) {
  viewerScopeStore.set(scope);
}

export function clearViewerScope() {
  if (viewerScopeStore.get() !== null) viewerScopeStore.set(null);
}
